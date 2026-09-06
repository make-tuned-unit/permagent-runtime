//! Deterministic held-out qualification and area scorecard.
//!
//! This module consumes retained, structured evidence.  It never contacts a
//! provider and never infers a pass from a human-entered status string.  In
//! particular, a single green smoke test cannot produce `Excellent`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REQUIRED_CONSECUTIVE_RUNS: usize = 3;

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
}
