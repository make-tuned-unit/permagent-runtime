//! Deterministic held-out qualification and area scorecard.
//!
//! This module consumes retained, structured evidence.  It never contacts a
//! provider and never infers a pass from a human-entered status string.  In
//! particular, a single green smoke test cannot produce `Excellent`.

use crate::harness_log::{HarnessOverhead, LandedGoal};
use crate::metrics::TaskResult;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REQUIRED_CONSECUTIVE_RUNS: usize = 3;

/// The nine areas the parent excellence DAG tracks (Council order, matching
/// `docs/orchestrator/CODING_HARNESS_EXCELLENCE_ITERATION_DAG.md`'s
/// scoreboard). [`qualification_input_from_records`] renders evidence for
/// every one of these from a set of retained [`RunQualificationRecord`]s.
pub const NINE_AREAS: [&str; 9] = [
    "observability",
    "safety_and_trust",
    "token_efficiency",
    "cost_routing",
    "dag_enforcement",
    "verification_integrity",
    "cheap_model_usability",
    "latency",
    "coding_success",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AreaRating {
    Excellent,
    Good,
    Poor,
    Unrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaEvidence {
    /// Whether every metric and invariant for this area passed in this run.
    pub gate_passed: bool,
    /// False means that at least one required field/evidence artifact is absent.
    pub evidence_complete: bool,
    /// P0/P1 findings prevent promotion even when metrics pass.
    #[serde(default)]
    pub unresolved_p1: bool,
    /// Names of the specific fields/artifacts absent when `evidence_complete`
    /// is false (X-obs-1: "any area whose evidence is missing is `Unrated`
    /// with the missing field named" — never a silent generic message when a
    /// caller can be specific). Empty when there is nothing to name, e.g.
    /// `evidence_complete` is true, or the input was built by an older caller
    /// that predates this field (`#[serde(default)]` keeps that JSON valid).
    #[serde(default)]
    pub missing_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationRun {
    pub run_id: String,
    pub heldout_passed: bool,
    pub areas: BTreeMap<String, AreaEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationInput {
    pub benchmark_version: String,
    /// Task IDs used while optimizing a candidate. They must not overlap the
    /// held-out set; the validator checks this rather than trusting labels.
    pub optimizer_task_ids: Vec<String>,
    pub heldout_task_ids: Vec<String>,
    pub runs: Vec<QualificationRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaScore {
    pub rating: AreaRating,
    pub qualifying_runs: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationReport {
    pub benchmark_version: String,
    pub heldout_task_count: usize,
    pub heldout_passed: bool,
    pub areas: BTreeMap<String, AreaScore>,
    pub overall: AreaRating,
    pub reasons: Vec<String>,
}

impl QualificationInput {
    pub fn validate(&self) -> Result<()> {
        if self.benchmark_version.trim().is_empty() {
            bail!("benchmark_version is required");
        }
        if self.heldout_task_ids.is_empty() {
            bail!("held-out task set must not be empty");
        }
        let optimizer: BTreeSet<_> = self.optimizer_task_ids.iter().collect();
        if optimizer.len() != self.optimizer_task_ids.len() {
            bail!("optimizer task IDs must be unique");
        }
        let heldout: BTreeSet<_> = self.heldout_task_ids.iter().collect();
        if heldout.len() != self.heldout_task_ids.len() {
            bail!("held-out task IDs must be unique");
        }
        let overlap: Vec<_> = self
            .heldout_task_ids
            .iter()
            .filter(|id| optimizer.contains(id))
            .collect();
        if !overlap.is_empty() {
            bail!("held-out tasks overlap optimizer tasks: {overlap:?}");
        }
        if self.runs.iter().any(|r| r.run_id.trim().is_empty()) {
            bail!("qualification run IDs must not be empty");
        }
        let run_ids: BTreeSet<_> = self.runs.iter().map(|run| &run.run_id).collect();
        if run_ids.len() != self.runs.len() {
            bail!("qualification run IDs must be unique");
        }
        Ok(())
    }
}

/// Build an honest scorecard.  The final three retained runs must qualify;
/// earlier runs are retained for history but cannot substitute for a broken
/// consecutive streak. Missing evidence is `Unrated`.
pub fn qualify(input: &QualificationInput) -> Result<QualificationReport> {
    input.validate()?;
    let mut reasons = Vec::new();
    let heldout_streak = trailing_true_streak(input.runs.iter().map(|r| r.heldout_passed));
    let heldout_passed = heldout_streak >= REQUIRED_CONSECUTIVE_RUNS;
    if !heldout_passed {
        reasons.push(format!(
            "held-out gate has {heldout_streak} consecutive passing run(s); need {REQUIRED_CONSECUTIVE_RUNS}"
        ));
    }

    let names: BTreeSet<String> = input
        .runs
        .iter()
        .flat_map(|r| r.areas.keys().cloned())
        .collect();
    let mut areas = BTreeMap::new();
    for name in names {
        let evidence: Vec<_> = input
            .runs
            .iter()
            .filter_map(|r| r.areas.get(&name))
            .collect();
        let complete =
            evidence.len() == input.runs.len() && evidence.iter().all(|e| e.evidence_complete);
        let streak =
            trailing_true_streak(evidence.iter().map(|e| e.gate_passed && !e.unresolved_p1));
        // A historical P1 is not necessarily still open. The trailing streak
        // already resets on the run where it appears; only the final retained
        // run's unresolved state blocks promotion after a clean recovery.
        let current_unresolved_p1 = evidence.last().is_some_and(|e| e.unresolved_p1);
        let rating = if !complete || input.runs.len() < REQUIRED_CONSECUTIVE_RUNS {
            AreaRating::Unrated
        } else if current_unresolved_p1 {
            AreaRating::Poor
        } else if streak >= REQUIRED_CONSECUTIVE_RUNS && heldout_passed {
            AreaRating::Excellent
        } else if evidence.iter().any(|e| e.unresolved_p1) || streak == 0 {
            AreaRating::Poor
        } else {
            AreaRating::Good
        };
        let reason = match rating {
            AreaRating::Excellent => {
                "three consecutive complete passing runs with held-out qualification".into()
            }
            // X-obs-1: name the specific missing field(s) when the retained
            // evidence itself says which are absent, rather than the old
            // generic message — a caller reading the scorecard should not
            // have to go diff retained JSON to find out what to collect
            // next. Falls back to the generic message when nothing more
            // specific is available (e.g. the run-count itself is short, or
            // an older caller never populated `missing_fields`).
            AreaRating::Unrated if !complete => {
                let missing: BTreeSet<&str> = evidence
                    .iter()
                    .flat_map(|e| e.missing_fields.iter().map(String::as_str))
                    .collect();
                if missing.is_empty() {
                    "required evidence or three-run sample is missing".into()
                } else {
                    format!(
                        "missing: {}",
                        missing.into_iter().collect::<Vec<_>>().join("; ")
                    )
                }
            }
            AreaRating::Unrated => "required evidence or three-run sample is missing".into(),
            AreaRating::Poor => "area gate did not pass on the required consecutive runs".into(),
            AreaRating::Good => {
                "evidence exists but the three-run consecutive gate is incomplete".into()
            }
        };
        if rating != AreaRating::Excellent {
            reasons.push(format!("{name}: {reason}"));
        }
        areas.insert(
            name,
            AreaScore {
                rating,
                qualifying_runs: streak,
                reason,
            },
        );
    }
    let overall = if heldout_passed
        && !areas.is_empty()
        && areas.values().all(|a| a.rating == AreaRating::Excellent)
    {
        AreaRating::Excellent
    } else if areas.values().any(|a| a.rating == AreaRating::Poor) {
        AreaRating::Poor
    } else {
        AreaRating::Unrated
    };
    Ok(QualificationReport {
        benchmark_version: input.benchmark_version.clone(),
        heldout_task_count: input.heldout_task_ids.len(),
        heldout_passed,
        areas,
        overall,
        reasons,
    })
}

fn trailing_true_streak(values: impl Iterator<Item = bool>) -> usize {
    values
        .fold((0, false), |(streak, _), value| {
            if value {
                (streak + 1, true)
            } else {
                (0, false)
            }
        })
        .0
}

// ============================================================================
// X-obs-1: the per-invocation qualification record.
//
// `RunQualificationRecord` is the RAW retained evidence one `run`/`ab`
// invocation writes as its `<out>.qual.json` sidecar (see
// `crates/permagent-eval/src/main.rs`'s `cmd_run`/`cmd_ab`) — task ids,
// fixture revision, binary provenance, provider/model/pins, and the
// per-task fields (result, cost, tokens, tool calls, seconds, not_run,
// errored, seed_commit, and the best-effort harness-log signals from
// [`crate::harness_log`]). It is deliberately a THICKER, more concrete
// shape than [`QualificationInput`]/[`QualificationRun`]/[`AreaEvidence`]
// above: those are the abstract, already-classified-per-area evidence
// `qualify` validates; a `RunQualificationRecord` is what a single run
// actually observed, before anyone has decided which of the nine areas it
// speaks to. [`qualification_input_from_records`] is the bridge from N
// retained records to that abstract shape, so `qualify` renders all nine
// areas without every caller having to hand-classify area evidence itself.
// ============================================================================

/// Where a run's harness binary came from, for provenance in the retained
/// record. `version`/`git_sha` are `None` when they could not be determined
/// (e.g. `--version` produced no output, or `strings` found no git-sha-shaped
/// token) — never defaulted to an empty string, which could be misread as "no
/// version" rather than "unknown".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryProvenance {
    pub path: String,
    pub version: Option<String>,
    pub git_sha: Option<String>,
}

/// One task's retained evidence within a [`RunQualificationRecord`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskQualificationEvidence {
    pub task_id: String,
    /// [`crate::oracle::OracleOutcome::label`]'s stable string: `pass`,
    /// `fail`, `errored`, or `not_run`.
    pub result: String,
    pub not_run: bool,
    pub errored: bool,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub tool_calls: usize,
    pub seconds: f64,
    pub seed_commit: Option<String>,
    /// The effective `PERMAGENT_HEADLESS_AUTOLAND` value the child received
    /// (Lane K3). See `crate::runner::run_task`.
    pub headless_autoland: Option<String>,
    /// The effective `PERMAGENT_HEADLESS_DRAIN_SECS` value the child
    /// received. See `crate::runner::run_task`.
    pub headless_drain_secs: Option<String>,
    /// See [`crate::harness_log::LogSignals`]'s "X-obs-1" docs for why each
    /// of the following is `None`/empty rather than a defaulted value when
    /// its producing lane's line was not found in this task's harness log.
    pub harness_overhead: Option<HarnessOverhead>,
    pub landed_goals: Vec<LandedGoal>,
    pub drain_summary: Option<String>,
    pub create_roadmap_dispatched: Option<usize>,
    pub reviewer_not_independent: Option<usize>,
    pub reviewer_mutated: Option<usize>,
    pub vacuous_pass: Option<usize>,
}

/// The retained evidence for ONE `run`/`ab` invocation (one tier, or one A/B
/// arm) — written as `<out>.qual.json`. See the section docs above.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunQualificationRecord {
    /// A unique identifier for this retained run (becomes
    /// [`QualificationRun::run_id`] via [`qualification_input_from_records`]).
    pub run_id: String,
    /// The git hash of the most recent commit that touched the tasks
    /// directory — `None` when the tasks dir is not inside a git repository,
    /// or `git log` otherwise failed.
    pub fixture_revision: Option<String>,
    pub binary: BinaryProvenance,
    pub provider: String,
    pub model: String,
    pub pinned_packs: bool,
    pub tasks: Vec<TaskQualificationEvidence>,
    /// Named record-level gaps (missing fixture revision, unknown binary
    /// version/git-sha, or a harness-log signal absent across every
    /// attempted task in this record) — named so `qualify`'s Unrated reason
    /// does not have to be a generic sentence. See
    /// [`build_run_qualification_record`].
    pub missing: Vec<String>,
}

/// Build a [`RunQualificationRecord`] from one tier/arm's [`TaskResult`]s,
/// computing `missing` from what is actually absent — never guessing or
/// defaulting a field that could not be determined. Pure (no I/O): callers
/// resolve `fixture_revision`/`binary` themselves (they require shelling out
/// to `git`/`strings`/the harness binary, which does not belong in this
/// crate's pure core) and pass the results in.
pub fn build_run_qualification_record(
    run_id: impl Into<String>,
    fixture_revision: Option<String>,
    binary: BinaryProvenance,
    provider: impl Into<String>,
    model: impl Into<String>,
    pinned_packs: bool,
    results: &[TaskResult],
) -> RunQualificationRecord {
    let mut missing = Vec::new();
    if fixture_revision.is_none() {
        missing.push(
            "fixture_revision: tasks dir is not a git repository, or `git log` failed".to_string(),
        );
    }
    if binary.version.is_none() {
        missing.push("binary.version: `<bin> --version` produced no usable output".to_string());
    }
    if binary.git_sha.is_none() {
        missing.push(
            "binary.git_sha: no git-sha-shaped token found via `strings` on the binary".to_string(),
        );
    }

    let tasks: Vec<TaskQualificationEvidence> = results
        .iter()
        .map(|r| TaskQualificationEvidence {
            task_id: r.task_id.clone(),
            result: r.oracle.label().to_string(),
            not_run: r.oracle == crate::oracle::OracleOutcome::NotRun,
            errored: r.oracle == crate::oracle::OracleOutcome::Errored,
            cost_usd: r.cost.usd,
            input_tokens: r.cost.input_tokens,
            output_tokens: r.cost.output_tokens,
            cache_read_tokens: r.cost.cache_read_tokens,
            cache_write_tokens: r.cost.cache_write_tokens,
            tool_calls: r.signals.tool_calls,
            seconds: r.duration_secs,
            seed_commit: r.seed_commit.clone(),
            headless_autoland: r.headless_autoland.clone(),
            headless_drain_secs: r.headless_drain_secs.clone(),
            harness_overhead: r.signals.harness_overhead,
            landed_goals: r.signals.landed_goals.clone(),
            drain_summary: r.signals.drain_summary.clone(),
            create_roadmap_dispatched: r.signals.create_roadmap_dispatched,
            reviewer_not_independent: r.signals.reviewer_not_independent,
            reviewer_mutated: r.signals.reviewer_mutated,
            vacuous_pass: r.signals.vacuous_pass,
        })
        .collect();

    // Name a harness-log signal ONCE at the record level when it is absent
    // across EVERY attempted task, rather than once per task — the common
    // case before the producing lane has landed in the binary under test at
    // all.
    let attempted: Vec<&TaskQualificationEvidence> = tasks.iter().filter(|t| !t.not_run).collect();
    if !attempted.is_empty() {
        if attempted.iter().all(|t| t.harness_overhead.is_none()) {
            missing.push(
                "harness_overhead: no matching line in any attempted task's harness log"
                    .to_string(),
            );
        }
        if attempted
            .iter()
            .all(|t| t.create_roadmap_dispatched.is_none())
        {
            missing.push(
                "create_roadmap_dispatched: no matching line in any attempted task's harness log"
                    .to_string(),
            );
        }
        if attempted
            .iter()
            .all(|t| t.reviewer_not_independent.is_none() && t.reviewer_mutated.is_none())
        {
            missing.push(
                "reviewer_receipts: no reviewer receipt line in any attempted task's harness log"
                    .to_string(),
            );
        }
        if attempted.iter().all(|t| t.vacuous_pass.is_none()) {
            missing.push(
                "vacuous_pass: no matching line in any attempted task's harness log".to_string(),
            );
        }
    } else {
        missing.push("tasks: no attempted task in this run".to_string());
    }

    RunQualificationRecord {
        run_id: run_id.into(),
        fixture_revision,
        binary,
        provider: provider.into(),
        model: model.into(),
        pinned_packs,
        tasks,
        missing,
    }
}

/// Bridge N retained [`RunQualificationRecord`]s into the abstract
/// [`QualificationInput`] shape [`qualify`] renders. This is the ONLY place
/// raw run evidence is classified into the nine named areas
/// ([`NINE_AREAS`]); see each area's doc comment in the implementation for
/// exactly which fields it consults and why.
///
/// `heldout_task_ids`/`optimizer_task_ids` are a policy decision that does
/// not live inside a single run's evidence — pass the same held-out split
/// used across the whole retained sweep. A record's `heldout_passed` is then
/// derived (not asserted): every task id in `heldout_task_ids` that appears
/// in that record must have passed. A record that ran none of the held-out
/// tasks cannot be read as having passed them.
pub fn qualification_input_from_records(
    records: &[RunQualificationRecord],
    benchmark_version: impl Into<String>,
    optimizer_task_ids: Vec<String>,
    heldout_task_ids: Vec<String>,
) -> QualificationInput {
    let heldout_ids: BTreeSet<&str> = heldout_task_ids.iter().map(String::as_str).collect();
    let runs = records
        .iter()
        .map(|record| {
            let heldout_results: Vec<&TaskQualificationEvidence> = record
                .tasks
                .iter()
                .filter(|t| heldout_ids.contains(t.task_id.as_str()))
                .collect();
            let heldout_passed =
                !heldout_results.is_empty() && heldout_results.iter().all(|t| t.result == "pass");
            let areas = NINE_AREAS
                .iter()
                .map(|&area| (area.to_string(), area_evidence(area, record)))
                .collect();
            QualificationRun {
                run_id: record.run_id.clone(),
                heldout_passed,
                areas,
            }
        })
        .collect();
    QualificationInput {
        benchmark_version: benchmark_version.into(),
        optimizer_task_ids,
        heldout_task_ids,
        runs,
    }
}

/// Classify one record's evidence for one named area. Areas whose gate
/// criterion is not yet defined by their OWNING lane (this eval crate does
/// not invent another lane's policy) are always `Unrated`, naming which lane
/// owns the missing definition — see
/// `docs/orchestrator/CODING_HARNESS_EXCELLENCE_ITERATION_DAG.md`'s node
/// list for who that is.
fn area_evidence(area: &str, record: &RunQualificationRecord) -> AreaEvidence {
    let attempted: Vec<&TaskQualificationEvidence> =
        record.tasks.iter().filter(|t| !t.not_run).collect();
    match area {
        // Observability's own bar: the record itself is complete provenance
        // (this is what X-obs-1 exists to guarantee).
        "observability" => {
            let mut missing_fields: Vec<String> = record
                .missing
                .iter()
                .filter(|m| {
                    m.starts_with("fixture_revision")
                        || m.starts_with("binary.")
                        || m.starts_with("tasks:")
                })
                .cloned()
                .collect();
            if record.tasks.is_empty() && missing_fields.is_empty() {
                missing_fields.push("tasks: no task evidence in this run".to_string());
            }
            let complete = missing_fields.is_empty();
            AreaEvidence {
                gate_passed: complete,
                evidence_complete: complete,
                unresolved_p1: false,
                missing_fields,
            }
        }
        // X-lat's gate is "reported on every run; first number retained as
        // baseline" — a reporting bar, not a numeric threshold, so
        // gate_passed and evidence_complete coincide.
        "latency" => {
            if attempted.is_empty() {
                return unrated(vec!["tasks: no attempted task in this run".to_string()]);
            }
            let with_overhead = attempted
                .iter()
                .filter(|t| t.harness_overhead.is_some())
                .count();
            if with_overhead < attempted.len() {
                return unrated(vec![format!(
                    "harness_overhead: missing on {}/{} attempted task(s)",
                    attempted.len() - with_overhead,
                    attempted.len()
                )]);
            }
            AreaEvidence {
                gate_passed: true,
                evidence_complete: true,
                unresolved_p1: false,
                missing_fields: Vec::new(),
            }
        }
        // X-land's gate: at least one attempted task run under headless
        // auto-land actually landed edits. A task not run under auto-land
        // (autoland off, e.g. an A/B control arm) is not evidence either way
        // and is excluded from both the numerator and denominator.
        "coding_success" => {
            let autoland_attempted: Vec<&&TaskQualificationEvidence> = attempted
                .iter()
                .filter(|t| t.headless_autoland.as_deref() == Some("1"))
                .collect();
            if autoland_attempted.is_empty() {
                return unrated(vec![
                    "landed_goals: no attempted task ran with PERMAGENT_HEADLESS_AUTOLAND=1"
                        .to_string(),
                ]);
            }
            let landed = autoland_attempted
                .iter()
                .any(|t| !t.landed_goals.is_empty());
            AreaEvidence {
                gate_passed: landed,
                evidence_complete: true,
                unresolved_p1: false,
                missing_fields: Vec::new(),
            }
        }
        // The remaining six areas' gate criteria are owned by lanes that had
        // not landed as of this module's writing (X-dag, X-skill, X-verify,
        // X-cost, X-ab, X-grad — see the DAG doc's node list). A raw receipt
        // count alone cannot prove a clean run: absence might mean "checked,
        // clean" or "not wired into this binary at all", and this crate must
        // never guess which. Each stays Unrated, naming its owning lane.
        "safety_and_trust" => unrated(vec![
            "no safety/trust receipt line defined yet (owned by Lanes X-dag, X-skill)".to_string(),
        ]),
        "dag_enforcement" => unrated(vec![
            "no roadmap-bypass receipt line defined yet (owned by Lane X-dag); \
             create_roadmap_dispatched alone cannot prove the absence of a bypass"
                .to_string(),
        ]),
        "verification_integrity" => unrated(vec![
            "no 'review reached'/clean reviewer receipt defined yet (owned by Lane X-verify); \
             a violation count alone cannot distinguish a clean run from a detector \
             that is not wired into this binary"
                .to_string(),
        ]),
        "cost_routing" => unrated(vec![
            "no routing-matrix verdict defined yet (owned by Lane X-cost)".to_string(),
        ]),
        "token_efficiency" => unrated(vec![
            "no knob-measurement verdict defined yet (owned by Lane X-ab)".to_string(),
        ]),
        "cheap_model_usability" => unrated(vec![
            "no capability-card verdict defined yet (owned by Lane X-grad)".to_string(),
        ]),
        _ => unrated(vec![format!("{area}: not one of the nine tracked areas")]),
    }
}

/// An `Unrated` [`AreaEvidence`] naming why.
fn unrated(missing_fields: Vec<String>) -> AreaEvidence {
    AreaEvidence {
        gate_passed: false,
        evidence_complete: false,
        unresolved_p1: false,
        missing_fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(runs: usize, pass: bool) -> QualificationInput {
        let areas: BTreeMap<String, AreaEvidence> = [(
            "trust".into(),
            AreaEvidence {
                gate_passed: pass,
                evidence_complete: true,
                unresolved_p1: false,
                missing_fields: Vec::new(),
            },
        )]
        .into_iter()
        .collect();
        QualificationInput {
            benchmark_version: "v1".into(),
            optimizer_task_ids: vec!["train-1".into()],
            heldout_task_ids: vec!["held-1".into()],
            runs: (0..runs)
                .map(|i| QualificationRun {
                    run_id: format!("r{i}"),
                    heldout_passed: pass,
                    areas: areas.clone(),
                })
                .collect(),
        }
    }

    #[test]
    fn one_green_run_is_unrated() {
        let report = qualify(&input(1, true)).unwrap();
        assert_eq!(report.overall, AreaRating::Unrated);
        assert_eq!(report.areas["trust"].rating, AreaRating::Unrated);
    }

    #[test]
    fn three_consecutive_complete_runs_can_graduate() {
        let report = qualify(&input(3, true)).unwrap();
        assert_eq!(report.overall, AreaRating::Excellent);
        assert_eq!(report.areas["trust"].qualifying_runs, 3);
    }

    #[test]
    fn a_broken_tail_invalidates_streak() {
        let mut value = input(4, true);
        value.runs[2].heldout_passed = false;
        value.runs[2].areas.get_mut("trust").unwrap().gate_passed = false;
        let report = qualify(&value).unwrap();
        assert_ne!(report.overall, AreaRating::Excellent);
        assert_eq!(report.areas["trust"].qualifying_runs, 1);
    }

    #[test]
    fn overlap_is_rejected() {
        let mut value = input(3, true);
        value.optimizer_task_ids.push("held-1".into());
        assert!(qualify(&value).is_err());
    }

    #[test]
    fn duplicate_run_cannot_impersonate_a_three_run_streak() {
        let mut value = input(3, true);
        value.runs[1].run_id = value.runs[0].run_id.clone();
        assert!(qualify(&value).unwrap_err().to_string().contains("unique"));
    }

    #[test]
    fn duplicate_task_ids_cannot_inflate_a_benchmark_set() {
        let mut duplicate_heldout = input(3, true);
        duplicate_heldout.heldout_task_ids.push("held-1".into());
        assert!(qualify(&duplicate_heldout)
            .unwrap_err()
            .to_string()
            .contains("held-out task IDs must be unique"));

        let mut duplicate_optimizer = input(3, true);
        duplicate_optimizer
            .optimizer_task_ids
            .push("train-1".into());
        assert!(qualify(&duplicate_optimizer)
            .unwrap_err()
            .to_string()
            .contains("optimizer task IDs must be unique"));
    }

    #[test]
    fn missing_area_evidence_is_unrated() {
        let mut value = input(3, true);
        value.runs[1].areas.clear();
        let report = qualify(&value).unwrap();
        assert_eq!(report.areas["trust"].rating, AreaRating::Unrated);
    }

    #[test]
    fn resolved_p1_can_recover_after_three_clean_runs() {
        let mut value = input(4, true);
        value.runs[0].areas.get_mut("trust").unwrap().unresolved_p1 = true;
        let report = qualify(&value).unwrap();
        assert_eq!(report.areas["trust"].rating, AreaRating::Excellent);
        assert_eq!(report.overall, AreaRating::Excellent);
    }

    // ------------------------------------------------------------------
    // X-obs-1: RunQualificationRecord, its builder, and the nine-area bridge
    // ------------------------------------------------------------------

    use crate::cost::CostReading;
    use crate::oracle::OracleOutcome;

    fn passing_task(id: &str) -> TaskResult {
        let mut r = TaskResult::new(
            id,
            "classic",
            OracleOutcome::Pass,
            CostReading::known(0.10, false, 1),
        );
        r.duration_secs = 5.0;
        r.headless_autoland = Some("1".to_string());
        r.headless_drain_secs = Some("840".to_string());
        r
    }

    fn complete_binary() -> BinaryProvenance {
        BinaryProvenance {
            path: "/opt/permagent/bin/permagent".to_string(),
            version: Some("permagent 1.2.3".to_string()),
            git_sha: Some("cafebabecafebabecafebabecafebabecafebabe".to_string()),
        }
    }

    #[test]
    fn build_run_qualification_record_names_missing_provenance() {
        let record = build_run_qualification_record(
            "r0",
            None, // fixture_revision unknown
            BinaryProvenance {
                path: "/bin/permagent".to_string(),
                version: None,
                git_sha: None,
            },
            "ollama",
            "qwen3",
            true,
            &[passing_task("t1")],
        );
        assert!(record
            .missing
            .iter()
            .any(|m| m.starts_with("fixture_revision")));
        assert!(record
            .missing
            .iter()
            .any(|m| m.starts_with("binary.version")));
        assert!(record
            .missing
            .iter()
            .any(|m| m.starts_with("binary.git_sha")));
    }

    #[test]
    fn build_run_qualification_record_names_absent_harness_signals_once_not_per_task() {
        let record = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            complete_binary(),
            "anthropic",
            "claude-sonnet-5",
            true,
            &[passing_task("t1"), passing_task("t2")],
        );
        // Neither task's (empty, in this test) harness log produced any of
        // these lines — each must be named exactly once, not once per task.
        for expect in [
            "harness_overhead:",
            "create_roadmap_dispatched:",
            "reviewer_receipts:",
            "vacuous_pass:",
        ] {
            let hits = record
                .missing
                .iter()
                .filter(|m| m.starts_with(expect))
                .count();
            assert_eq!(
                hits, 1,
                "{expect} should be named exactly once: {:?}",
                record.missing
            );
        }
        // Provenance is complete, so nothing provenance-related is named.
        assert!(!record
            .missing
            .iter()
            .any(|m| m.starts_with("fixture_revision")));
        assert!(!record.missing.iter().any(|m| m.starts_with("binary.")));
    }

    #[test]
    fn build_run_qualification_record_with_only_not_run_tasks_names_that_not_the_harness_signals() {
        let not_run = TaskResult::new(
            "t1",
            "classic",
            OracleOutcome::NotRun,
            CostReading::unknown(),
        );
        let record = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            complete_binary(),
            "anthropic",
            "claude-sonnet-5",
            true,
            &[not_run],
        );
        assert!(record.missing.iter().any(|m| m.starts_with("tasks:")));
        assert!(!record
            .missing
            .iter()
            .any(|m| m.starts_with("harness_overhead")));
    }

    #[test]
    fn build_run_qualification_record_carries_headless_autoland_fields_per_task() {
        let record = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            complete_binary(),
            "anthropic",
            "claude-sonnet-5",
            true,
            &[passing_task("t1")],
        );
        assert_eq!(record.tasks[0].headless_autoland.as_deref(), Some("1"));
        assert_eq!(record.tasks[0].headless_drain_secs.as_deref(), Some("840"));
    }

    /// The end-to-end bridge: N complete records, fed through
    /// `qualification_input_from_records` into `qualify`, render all nine
    /// areas — the ones this crate can honestly derive graduate or fail on
    /// their own merits, and the six owned by lanes that had not landed stay
    /// `Unrated`, each naming its owning lane rather than a generic message.
    #[test]
    fn qualification_input_from_records_renders_all_nine_areas() {
        let complete_record = |id: &str| {
            build_run_qualification_record(
                id,
                Some("deadbeef".to_string()),
                complete_binary(),
                "anthropic",
                "claude-sonnet-5",
                true,
                &[passing_task("held-1")],
            )
        };
        let records = vec![
            complete_record("r0"),
            complete_record("r1"),
            complete_record("r2"),
        ];
        let qi = qualification_input_from_records(
            &records,
            "records-v1",
            Vec::new(),
            vec!["held-1".to_string()],
        );
        let report = qualify(&qi).unwrap();

        // held-1 passed in every retained run.
        assert!(report.heldout_passed);

        // Observability is exactly what this lane exists to guarantee: three
        // complete, provenance-clean records graduate it.
        assert_eq!(
            report.areas["observability"].rating,
            AreaRating::Excellent,
            "{:?}",
            report.areas["observability"]
        );

        // Latency: no `harness_overhead` line anywhere in these (empty)
        // harness logs -> Unrated, naming the missing field.
        assert_eq!(report.areas["latency"].rating, AreaRating::Unrated);
        assert!(
            report.areas["latency"].reason.contains("harness_overhead"),
            "{}",
            report.areas["latency"].reason
        );

        // Coding success: every task ran under headless auto-land, but none
        // landed anything -> evidence complete, gate failed -> Poor (matches
        // the DAG scoreboard's "Coding success | Poor (0/10)" reading).
        assert_eq!(report.areas["coding_success"].rating, AreaRating::Poor);

        // The six areas this crate does not own stay Unrated, each naming
        // its owning lane rather than a generic "missing" sentence.
        let owner_by_area = [
            ("safety_and_trust", "X-dag"),
            ("dag_enforcement", "X-dag"),
            ("verification_integrity", "X-verify"),
            ("cost_routing", "X-cost"),
            ("token_efficiency", "X-ab"),
            ("cheap_model_usability", "X-grad"),
        ];
        for (area, owner) in owner_by_area {
            assert_eq!(report.areas[area].rating, AreaRating::Unrated, "{area}");
            assert!(
                report.areas[area].reason.contains(owner),
                "area {area} reason should name its owning lane {owner}: {}",
                report.areas[area].reason
            );
        }
    }

    /// A record whose fixture revision and binary provenance are both
    /// unknown must keep `observability` Unrated and NAME both gaps in the
    /// rendered reason — this is the whole point of X-obs-1 over the old
    /// generic "required evidence ... is missing" sentence.
    #[test]
    fn qualification_input_from_records_names_missing_observability_fields() {
        let incomplete = build_run_qualification_record(
            "r0",
            None,
            BinaryProvenance {
                path: "/bin/permagent".to_string(),
                version: None,
                git_sha: None,
            },
            "ollama",
            "qwen3",
            true,
            &[passing_task("t1")],
        );
        // `qualify` requires a non-empty held-out set to validate at all; use
        // a placeholder id this record never ran — irrelevant to what this
        // test checks (observability, not the held-out gate).
        let qi = qualification_input_from_records(
            &[incomplete],
            "records-v1",
            Vec::new(),
            vec!["unrelated-heldout-task".to_string()],
        );
        let report = qualify(&qi).unwrap();
        assert_eq!(report.areas["observability"].rating, AreaRating::Unrated);
        let reason = &report.areas["observability"].reason;
        assert!(reason.contains("fixture_revision"), "{reason}");
        assert!(reason.contains("binary.version"), "{reason}");
        assert!(reason.contains("binary.git_sha"), "{reason}");
    }

    /// A record with zero task evidence at all must not be silently read as
    /// complete provenance.
    #[test]
    fn qualification_input_from_records_treats_an_empty_run_as_missing_observability_evidence() {
        let empty = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            complete_binary(),
            "anthropic",
            "claude-sonnet-5",
            true,
            &[],
        );
        let qi = qualification_input_from_records(
            &[empty],
            "records-v1",
            Vec::new(),
            vec!["unrelated-heldout-task".to_string()],
        );
        let report = qualify(&qi).unwrap();
        assert_eq!(report.areas["observability"].rating, AreaRating::Unrated);
    }

    /// A held-out task id that never ran in a record must not be read as
    /// having passed held-out qualification — `heldout_passed` requires
    /// having actually run (and passed) every named held-out task.
    #[test]
    fn heldout_passed_requires_the_heldout_task_to_have_actually_run() {
        let record = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            complete_binary(),
            "anthropic",
            "claude-sonnet-5",
            true,
            &[passing_task("some-other-task")],
        );
        let qi = qualification_input_from_records(
            &[record],
            "records-v1",
            Vec::new(),
            vec!["held-1".to_string()], // never actually run
        );
        assert!(!qi.runs[0].heldout_passed);
    }
}
