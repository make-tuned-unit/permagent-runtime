//! D1 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md` G2): turn
//! retained `permagent-eval run --format json --out FILE` reports into
//! durable `RoleEvidence` rows in the capability directory
//! (`crates/goose/src/cost_router/capability_directory.rs`), so a model's
//! `graduated` state traces to specific retained files rather than a human
//! label.
//!
//! ## Why this module does NOT call `cost_router::capability_directory`
//!
//! `crates/goose` (package `permagent`) already depends on THIS crate
//! (`permagent-eval = { path = "../permagent-eval" }`,
//! `crates/goose/Cargo.toml:76`; consumed by `program_bridge.rs`,
//! `decisions_effects.rs`, `decision_inbox/policy.rs`, `council/mod.rs` for
//! `ProgramDag`/`ProgramNode`). A `permagent-eval -> permagent` dependency
//! would therefore be a cyclic package dependency — Cargo refuses to build
//! it, full stop. This module instead reads and writes the SAME on-disk
//! JSON schema directly via `serde_json::Value`, patching only the one
//! `(provider, model, role)` slot it is asked to update and leaving every
//! other byte of the file exactly as written by the real
//! `CapabilityDirectory::save` — this is actually SAFER than a typed
//! round-trip for an unrelated crate, since it can never accidentally drop
//! a field this module doesn't know about.
//!
//! The [`RoleEvidence`] struct below, [`MICRO_SUITE_PASS_FLOOR`],
//! [`KB_SNAPSHOT_STALE_AFTER_DAYS`] and [`validate_graduation`] are
//! DELIBERATE mirrors of `capability_directory.rs`'s private `RoleEvidence`/
//! `RoleEvidenceRaw`, its `MICRO_SUITE_PASS_FLOOR` constant, `knowledge.rs`'s
//! `KB_SNAPSHOT_STALE_AFTER_DAYS`, and `validate_graduation` respectively —
//! kept field-for-field/behavior-for-behavior identical BY HAND, cited at
//! each definition. This is real duplication with a real drift risk (flagged
//! as an open risk in the lane report); it is the honest cost of avoiding
//! the cycle rather than something silently glossed over.
//!
//! ## What counts as "the record"
//!
//! Two artifacts can feed one `--run FILE`: the JSON
//! [`crate::report::render_json`] report itself (`permagent-eval run
//! --format json --out FILE`), read via [`RunFile`]; and, since K3 (X-obs-1),
//! the `<FILE>.qual.json` sidecar every `run`/`ab` invocation ALSO writes
//! next to `FILE` — a JSON array of [`crate::qualification::
//! RunQualificationRecord`] (one per tier/arm), which unlike
//! [`crate::qualification::QualificationRun`] DOES carry provider/model/
//! cost/latency/per-task evidence. [`select_evidence_source`] prefers the
//! sidecar when `<FILE>.qual.json` exists on disk, falling back to `FILE`
//! itself otherwise — see [`EvidenceSource`] for how a caller can tell which
//! one actually fed a given [`RunEvidence`]. The one field the sidecar
//! cannot supply is `dag_selection_pct` (not part of
//! [`crate::qualification::RunQualificationRecord`]'s schema): a
//! qual.json-sourced run reads that as `None`, same honest-absence
//! treatment [`RunFile::dag_selection_pct`] already gets from a report that
//! predates G3.
//!
//! [`RunFile`] reads the report shape exactly, plus a small set of
//! additive, optional top-level fields ([`RunFile::run_id`],
//! [`RunFile::generated_at`], [`RunFile::dag_selection_pct`],
//! [`RunFile::smoke_task_id`]) that a forward-compatible run file may also
//! carry (`main.rs`'s `cmd_run` stamps `run_id`/`generated_at` on every
//! future JSON run — D2). A plain, unmodified report from today's
//! `render_json` still parses: those fields are simply absent, and this
//! module never fabricates them.
//!
//! ## No incremental state
//!
//! Like [`crate::qualification::qualify`], this recomputes a role's
//! evidence FRESH from the run files handed to one invocation — there is no
//! hidden per-invocation streak file. `evidence_run_ids` accumulates onto
//! whatever the persisted card already carries (so history is never lost),
//! but `graduated`/`micro_suite_pass_pct`/`smoke_passed`/`dag_selection_pct`
//! are derived from THIS invocation's run set only. Passing fewer than
//! [`crate::qualification::REQUIRED_CONSECUTIVE_RUNS`] qualifying runs will
//! not preserve a prior `graduated=true` — that is intentional: standing
//! graduation must be re-evidenced from what is actually retained in the
//! call, never carried by inertia (see the lane report's "open risks":
//! re-run `graduate` with the full retained run history you want
//! considered, the same way `qualify --input` is always handed the whole
//! evidence set).
//!
//! ## The graduation gate
//!
//! A run "qualifies" only when ALL of:
//! - `smoke_passed_this_run` is `true` — the run's task set included the
//!   role's smoke task AND it was solved. A run whose task set does not
//!   include the smoke task does NOT qualify (never fabricate a pass from
//!   absence).
//! - `pass_pct >= MICRO_SUITE_PASS_FLOOR` (0.80).
//! - `dag_selection_pct >= DAG_SELECTION_PASS_FLOOR` (0.95) — **carried by the
//!   run itself**. No retained run produces this today (the G3 tool/DAG-
//!   selection micro-fixtures do not exist yet, per the DAG doc's G2
//!   table), so no role graduates through this pipeline until G3 lands.
//!   That is intentional, matching the DAG doc's G6 stance: "do not
//!   graduate on confounded data."
//!
//! `graduated` is true only when the trailing streak of qualifying runs (in
//! the order the caller passed `--run` files) is at least
//! [`crate::qualification::REQUIRED_CONSECUTIVE_RUNS`] — the SAME constant
//! `qualify()` uses, not a re-derived one.

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::cost::CostReading;
use crate::harness_log::LogSignals;
use crate::metrics::{aggregate, TaskResult};
use crate::oracle::OracleOutcome;
use crate::qualification::{
    RunQualificationRecord, TaskQualificationEvidence, REQUIRED_CONSECUTIVE_RUNS,
};

/// Mirrors `crates/goose/src/cost_router/capability_directory.rs:89`'s
/// `MICRO_SUITE_PASS_FLOOR`. See the module doc's "duplication" note.
pub const MICRO_SUITE_PASS_FLOOR: f64 = 0.80;

/// Mirrors `crates/goose/src/cost_router/knowledge.rs:491`'s
/// `KB_SNAPSHOT_STALE_AFTER_DAYS`. See the module doc's "duplication" note.
pub const KB_SNAPSHOT_STALE_AFTER_DAYS: i64 = 90;

/// The capability-directory schema version this module writes, mirroring
/// `capability_directory.rs:331`'s `SCHEMA_VERSION`.
pub const SCHEMA_VERSION: u32 = 1;

/// The DAG-selection pass-rate floor a run must clear to qualify. Mirrors
/// `docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md`'s G0 table
/// (`dag_selection_pct` — "≥ 95% required (Excellence gate)") and the DAG's
/// Objective line ("...plus ... 95% of tool/DAG-selection checks"), applied
/// uniformly to every role per that same sentence — not just Orchestrate.
pub const DAG_SELECTION_PASS_FLOOR: f64 = 0.95;

/// The one coordinator smoke this repo has retained evidence for today:
/// `docs/orchestrator/CODING_HARNESS_EXCELLENCE_DAG.md:207`'s "FizzBuzz-class
/// coordinator smoke", backed by the bundled `crates/permagent-eval/tasks/
/// fizzbuzz` task. No smoke task is established yet for Edit/Mechanical/
/// Review/Local — [`RunFile::smoke_task_id`] lets a caller name one
/// explicitly rather than this module guessing a fixture into that role.
pub const DEFAULT_ORCHESTRATE_SMOKE_TASK_ID: &str = "fizzbuzz";

// ── WorkflowRole (mirrors cost_router::recommend::WorkflowRole) ─────────

/// Mirrors `crates/goose/src/cost_router/recommend.rs`'s `WorkflowRole`:
/// same five variants, same `as_str()` strings (the capability-directory
/// JSON's `roles` map key), same `from_tag` aliases. See the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRole {
    Orchestrate,
    Edit,
    Mechanical,
    Review,
    Local,
}

impl WorkflowRole {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowRole::Orchestrate => "orchestrate",
            WorkflowRole::Edit => "edit",
            WorkflowRole::Mechanical => "mechanical",
            WorkflowRole::Review => "review",
            WorkflowRole::Local => "local",
        }
    }

    pub fn from_tag(tag: &str) -> Option<WorkflowRole> {
        match tag {
            "orchestrate" | "hard" | "plan" | "planning" => Some(WorkflowRole::Orchestrate),
            "edit" | "code_edit" => Some(WorkflowRole::Edit),
            "mechanical" | "read_only" => Some(WorkflowRole::Mechanical),
            "review" | "verify" => Some(WorkflowRole::Review),
            "local" => Some(WorkflowRole::Local),
            _ => None,
        }
    }

    /// The built-in smoke task id for this role, when this repo has
    /// retained evidence for one. See [`DEFAULT_ORCHESTRATE_SMOKE_TASK_ID`]'s
    /// doc for why the other four roles are `None`.
    pub fn default_smoke_task_id(self) -> Option<&'static str> {
        match self {
            WorkflowRole::Orchestrate => Some(DEFAULT_ORCHESTRATE_SMOKE_TASK_ID),
            WorkflowRole::Edit
            | WorkflowRole::Mechanical
            | WorkflowRole::Review
            | WorkflowRole::Local => None,
        }
    }
}

/// See [`WorkflowRole::default_smoke_task_id`].
pub fn default_smoke_task_id(role: WorkflowRole) -> Option<&'static str> {
    role.default_smoke_task_id()
}

// ── RoleEvidence (mirrors cost_router::capability_directory::RoleEvidence) ─

/// Mirrors `capability_directory.rs`'s `RoleEvidence` field-for-field (same
/// JSON shape, since that struct has no `#[serde(rename)]`s) — see the
/// module doc's "duplication" note. Fields are public here (unlike the real
/// type) because this module is the only writer of this file from THIS
/// crate; [`RoleEvidence::new`] still runs the same validation gate so a
/// `graduated=true` value constructed here is held to the identical bar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleEvidence {
    #[serde(default)]
    pub graduated: bool,
    #[serde(default)]
    pub smoke_passed: bool,
    #[serde(default)]
    pub micro_suite_pass_pct: Option<f64>,
    #[serde(default)]
    pub dag_selection_pct: Option<f64>,
    #[serde(default)]
    pub evidence_run_ids: Vec<String>,
    #[serde(default)]
    pub observed_cost_usd: Option<f64>,
    #[serde(default)]
    pub observed_latency_s: Option<f64>,
    #[serde(default)]
    pub snapshot_date: Option<NaiveDate>,
}

/// Mirrors `capability_directory.rs`'s `validate_graduation` exactly
/// (updated to match Lane Z-fix's G7 finding F3 fix, `2ebf7d8b`): a
/// `graduated=true` claim needs a passed smoke test; a `micro_suite_pass_pct`
/// present AND at/above [`MICRO_SUITE_PASS_FLOOR`]; a `dag_selection_pct`,
/// when present, at/above [`DAG_SELECTION_PASS_FLOOR`] (absent is skipped,
/// not required — only Orchestrate's evidence populates it); at least one
/// evidence-run id; AND a `snapshot_date` no older than
/// [`KB_SNAPSHOT_STALE_AFTER_DAYS`] as of `today`. `graduated=false` needs
/// nothing (the default, unrated state).
#[allow(clippy::too_many_arguments)]
pub fn validate_graduation(
    graduated: bool,
    smoke_passed: bool,
    micro_suite_pass_pct: Option<f64>,
    dag_selection_pct: Option<f64>,
    evidence_run_ids: &[String],
    snapshot_date: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<(), String> {
    if !graduated {
        return Ok(());
    }
    if !smoke_passed {
        return Err("graduated=true requires smoke_passed".to_string());
    }
    match micro_suite_pass_pct {
        None => return Err("graduated=true requires a micro_suite_pass_pct".to_string()),
        Some(p) if p < MICRO_SUITE_PASS_FLOOR => {
            return Err(format!(
                "graduated=true requires micro_suite_pass_pct >= {MICRO_SUITE_PASS_FLOOR:.2} \
                 (got {p:.2})"
            ));
        }
        Some(_) => {}
    }
    if let Some(d) = dag_selection_pct {
        if d < DAG_SELECTION_PASS_FLOOR {
            return Err(format!(
                "graduated=true requires dag_selection_pct >= {DAG_SELECTION_PASS_FLOOR:.2} \
                 when present (got {d:.2})"
            ));
        }
    }
    if evidence_run_ids.is_empty() {
        return Err("graduated=true requires at least one evidence_run_id".to_string());
    }
    match snapshot_date {
        None => Err("graduated=true requires a snapshot_date".to_string()),
        Some(d) => {
            if (today - d).num_days() > KB_SNAPSHOT_STALE_AFTER_DAYS {
                Err(format!(
                    "graduated=true snapshot_date {d} is stale (> {KB_SNAPSHOT_STALE_AFTER_DAYS} \
                     days old as of {today})"
                ))
            } else {
                Ok(())
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
impl RoleEvidence {
    /// Construct a validated evidence record — mirrors
    /// `capability_directory::RoleEvidence::new`'s signature and gate.
    pub fn new(
        graduated: bool,
        smoke_passed: bool,
        micro_suite_pass_pct: Option<f64>,
        dag_selection_pct: Option<f64>,
        evidence_run_ids: Vec<String>,
        observed_cost_usd: Option<f64>,
        observed_latency_s: Option<f64>,
        snapshot_date: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Result<Self, String> {
        validate_graduation(
            graduated,
            smoke_passed,
            micro_suite_pass_pct,
            dag_selection_pct,
            &evidence_run_ids,
            snapshot_date,
            today,
        )?;
        Ok(Self {
            graduated,
            smoke_passed,
            micro_suite_pass_pct,
            dag_selection_pct,
            evidence_run_ids,
            observed_cost_usd,
            observed_latency_s,
            snapshot_date,
        })
    }
}

// ── Run-file shapes ──────────────────────────────────────────────────────

/// One retained run report, as written by `permagent-eval run --format json
/// --out FILE` (see the module doc for the exact provenance and the
/// additive fields this module also reads when present).
#[derive(Debug, Clone, Deserialize)]
pub struct RunFile {
    /// Stable identity for this run, e.g. `run-20260909T101500Z`. `None` for
    /// a plain `render_json` report produced before D2's stamping landed —
    /// callers then fall back to `<path>#<tier>` (see [`resolve_run_id`]).
    #[serde(default)]
    pub run_id: Option<String>,
    /// RFC 3339 timestamp this run was recorded. `None` falls back to
    /// `--snapshot-date`; if neither is present the run is refused (a
    /// `graduated=true` claim requires a real `snapshot_date`, and ordering
    /// the streak requires SOME date).
    #[serde(default)]
    pub generated_at: Option<String>,
    /// Tool/DAG-selection micro-fixture pass rate for this run, `[0,1]`.
    /// Absent for every run today (G3 does not exist yet) — never
    /// synthesized from anything else.
    #[serde(default)]
    pub dag_selection_pct: Option<f64>,
    /// Overrides the default per-role smoke task id for this run.
    #[serde(default)]
    pub smoke_task_id: Option<String>,
    /// Per-tier results, in `render_json`'s exact shape.
    #[serde(default)]
    pub tiers: Vec<TierRecord>,
}

/// One tier's slice of a retained run, mirroring `report::render_json`'s
/// per-tier object.
#[derive(Debug, Clone, Deserialize)]
pub struct TierRecord {
    #[serde(default)]
    pub tier: Option<String>,
    /// `None` when the record genuinely does not carry a provider (a
    /// caller-supplied `--provider` is then required — see
    /// [`resolve_identity`]). Never an empty-string sentinel.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub summary: SummaryRecord,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SummaryRecord {
    #[serde(default)]
    pub attempted: usize,
    #[serde(default)]
    pub solved: usize,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub median_duration_secs: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    #[serde(default)]
    pub solved: bool,
}

// ── One resolved run, ready to fold into a role's evidence ──────────────

/// Which artifact actually fed one [`RunEvidence`]: the `<FILE>.qual.json`
/// sidecar (K3, preferred when it exists — see [`select_evidence_source`])
/// or the `--format json` report itself (Z4's original evidence source).
/// Kept on [`RunEvidence`] (and surfaced per-run on
/// [`GraduateOutcome::run_sources`]) so a caller can always tell which file
/// actually supplied a given run's fields — never silently substituted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceSource {
    /// The `<FILE>.qual.json` sidecar (a JSON array of
    /// [`crate::qualification::RunQualificationRecord`]).
    QualJson,
    /// The `--format json --out FILE` report itself ([`RunFile`]).
    Report,
}

impl EvidenceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceSource::QualJson => "qual.json",
            EvidenceSource::Report => "report",
        }
    }
}

/// One run's contribution to a (provider, model, role)'s evidence — already
/// resolved to a single tier and dated, before the qualifying-streak
/// computation runs over a full ordered batch of these.
#[derive(Debug, Clone, PartialEq)]
pub struct RunEvidence {
    pub run_id: String,
    pub snapshot_date: NaiveDate,
    /// `solved / attempted`, `[0,1]`.
    pub pass_pct: f64,
    pub cost_usd: Option<f64>,
    pub latency_s: Option<f64>,
    /// `Some(true|false)` when the run's task set included the role's smoke
    /// task; `None` when it did not (never fabricated as `false`, since
    /// "absent" and "failed" are different claims for the CARD's
    /// informational field — see [`compute_role_evidence`]. For the
    /// qualifying-streak check itself, absent is treated as not-qualifying;
    /// see [`run_qualifies`]).
    pub smoke_passed_this_run: Option<bool>,
    pub dag_selection_pct: Option<f64>,
    /// Which file actually fed this run's fields. See [`EvidenceSource`].
    pub source: EvidenceSource,
}

/// Select the one tier this invocation is grading out of a run file's
/// tiers. Explicit `--provider`/`--model` selects by exact match; with
/// neither given, a single-tier file is used as-is and a multi-tier file is
/// refused (never guessed which of several tiers the caller meant).
pub fn select_tier<'a>(
    file: &'a RunFile,
    path: &Path,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<&'a TierRecord> {
    if file.tiers.is_empty() {
        bail!(
            "{}: no tiers found — expected a `permagent-eval run --format json --out FILE` report",
            path.display()
        );
    }
    if let (Some(p), Some(m)) = (provider, model) {
        return file
            .tiers
            .iter()
            .find(|t| t.provider.as_deref() == Some(p) && t.model.as_deref() == Some(m))
            .ok_or_else(|| {
                anyhow!(
                    "{}: no tier matches --provider {p} --model {m}",
                    path.display()
                )
            });
    }
    if file.tiers.len() == 1 {
        return Ok(&file.tiers[0]);
    }
    bail!(
        "{}: contains {} tiers — pass --provider/--model to select one",
        path.display(),
        file.tiers.len()
    );
}

/// Resolve the (provider, model) this run's evidence applies to. The run
/// record wins when it carries a value; a flag is required (and must not
/// conflict) only when the record's field is absent — never guessed.
pub fn resolve_identity(
    tier: &TierRecord,
    path: &Path,
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
) -> Result<(String, String)> {
    let provider = match (&tier.provider, provider_flag) {
        (Some(p), None) => p.clone(),
        (Some(p), Some(flag)) if p == flag => p.clone(),
        (Some(p), Some(flag)) => bail!(
            "{}: run record provider '{p}' conflicts with --provider '{flag}'",
            path.display()
        ),
        (None, Some(flag)) => flag.to_string(),
        (None, None) => bail!(
            "{}: run record has no provider recorded; pass --provider (and --model) explicitly \
             — never guessed",
            path.display()
        ),
    };
    let model = match (&tier.model, model_flag) {
        (Some(m), None) => m.clone(),
        (Some(m), Some(flag)) if m == flag => m.clone(),
        (Some(m), Some(flag)) => bail!(
            "{}: run record model '{m}' conflicts with --model '{flag}'",
            path.display()
        ),
        (None, Some(flag)) => flag.to_string(),
        (None, None) => bail!(
            "{}: run record has no model recorded; pass --model (and --provider) explicitly — \
             never guessed",
            path.display()
        ),
    };
    Ok((provider, model))
}

/// `run_id` for a resolved tier: the file's own `run_id` when present, else
/// a stable fallback derived from the file path and tier name.
pub fn resolve_run_id(file: &RunFile, path: &Path, tier: &TierRecord) -> String {
    if let Some(id) = &file.run_id {
        return id.clone();
    }
    let tier_name = tier.tier.as_deref().unwrap_or("tier");
    format!("{}#{}", path.display(), tier_name)
}

/// Parse `generated_at` (RFC 3339) into a date, falling back to
/// `--snapshot-date` (`YYYY-MM-DD`), refusing with a clear error when
/// neither is available.
pub fn resolve_snapshot_date(
    file: &RunFile,
    path: &Path,
    snapshot_date_flag: Option<&str>,
) -> Result<NaiveDate> {
    if let Some(raw) = &file.generated_at {
        let dt = chrono::DateTime::parse_from_rfc3339(raw).with_context(|| {
            format!(
                "{}: generated_at {raw:?} is not a valid RFC 3339 timestamp",
                path.display()
            )
        })?;
        return Ok(dt.date_naive());
    }
    if let Some(raw) = snapshot_date_flag {
        return NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .with_context(|| format!("--snapshot-date {raw:?} is not YYYY-MM-DD"));
    }
    bail!(
        "{}: no generated_at in the run record and no --snapshot-date given — a run needs a \
         date to be ordered and to satisfy the graduation schema's snapshot_date requirement",
        path.display()
    );
}

/// Build one [`RunEvidence`] from a parsed, selected, identity-resolved
/// tier. `role_smoke_default` is the built-in smoke task id for the target
/// role (see [`default_smoke_task_id`]); `file.smoke_task_id` overrides it.
pub fn build_run_evidence(
    file: &RunFile,
    tier: &TierRecord,
    path: &Path,
    snapshot_date_flag: Option<&str>,
    role_smoke_default: Option<&str>,
) -> Result<RunEvidence> {
    if tier.summary.attempted == 0 {
        bail!(
            "{}: summary.attempted is 0 — no evidence to grade",
            path.display()
        );
    }
    let pass_pct = tier.summary.solved as f64 / tier.summary.attempted as f64;
    let run_id = resolve_run_id(file, path, tier);
    let snapshot_date = resolve_snapshot_date(file, path, snapshot_date_flag)?;
    let smoke_task_id = file.smoke_task_id.as_deref().or(role_smoke_default);
    let smoke_passed_this_run = smoke_task_id.and_then(|id| {
        tier.tasks
            .iter()
            .find(|t| t.task_id == id)
            .map(|t| t.solved)
    });
    Ok(RunEvidence {
        run_id,
        snapshot_date,
        pass_pct,
        cost_usd: tier.summary.total_cost_usd,
        latency_s: tier.summary.median_duration_secs,
        smoke_passed_this_run,
        dag_selection_pct: file.dag_selection_pct,
        source: EvidenceSource::Report,
    })
}

/// The `<FILE>.qual.json` sidecar path a `run`/`ab` invocation's `--out
/// FILE` writes alongside its report (`.qual.json` APPENDED, never a
/// replaced extension — mirrors `main.rs`'s private
/// `qualification_sibling_path` exactly, so a report written to `report.json`
/// pairs with `report.json.qual.json`).
pub fn qual_json_sidecar_path(run_path: &Path) -> PathBuf {
    let mut name = run_path.as_os_str().to_os_string();
    name.push(".qual.json");
    PathBuf::from(name)
}

/// Reconstruct the [`TaskResult`] shape [`crate::metrics::aggregate`] needs
/// from one qual.json task record, so a qual.json-sourced run's
/// `pass_pct`/`cost_usd`/`latency_s` are computed by the EXACT SAME
/// aggregation code path a report's `summary` was originally derived from
/// (`report::render_json` calls `TierReport::aggregate`, which is
/// `crate::metrics::aggregate` over the same per-task shape) — this is what
/// makes the two sources produce identical numbers for equivalent evidence,
/// not a re-derivation that could drift from it. Fields `aggregate` never
/// reads (`category`, `harness_exit`, `note`) are filled with harmless
/// placeholders; `oracle` is parsed from `t.result`'s stable label, refusing
/// (never guessing) on a value outside the four this crate ever writes.
fn task_result_from_qual_evidence(t: &TaskQualificationEvidence) -> Result<TaskResult> {
    let oracle = match t.result.as_str() {
        "pass" => OracleOutcome::Pass,
        "fail" => OracleOutcome::Fail,
        "errored" => OracleOutcome::Errored,
        "not_run" => OracleOutcome::NotRun,
        other => bail!(
            "task '{}': unrecognized qualification-record result label '{other}' — expected \
             one of pass, fail, errored, not_run",
            t.task_id
        ),
    };
    Ok(TaskResult {
        task_id: t.task_id.clone(),
        category: String::new(),
        solved: oracle.solved(),
        cost: CostReading {
            usd: t.cost_usd,
            estimated: false,
            ledger_rows: 0,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cache_read_tokens: t.cache_read_tokens,
            cache_write_tokens: t.cache_write_tokens,
        },
        duration_secs: t.seconds,
        harness_exit: None,
        harness_timed_out: false,
        oracle,
        // Lane K7: a qual.json record predates (or never retained) these
        // two fields — `#[serde(default)]` on
        // `TaskQualificationEvidence::oracle_timed_out` means an older
        // record simply reads back `false`/`None` here, never fabricated;
        // `metrics::aggregate` (the sole consumer of this reconstruction)
        // does not read either field.
        oracle_exit: None,
        oracle_timed_out: t.oracle_timed_out.unwrap_or(false),
        oracle_secs: t.oracle_secs.unwrap_or(0.0),
        note: None,
        signals: LogSignals {
            tool_calls: t.tool_calls,
            harness_overhead: t.harness_overhead,
            landed_goals: t.landed_goals.clone(),
            drain_summary: t.drain_summary.clone(),
            create_roadmap_dispatched: t.create_roadmap_dispatched,
            reviewer_not_independent: t.reviewer_not_independent,
            reviewer_mutated: t.reviewer_mutated,
            vacuous_pass: t.vacuous_pass,
            ..LogSignals::default()
        },
        // Lane R18c (A17 Q6): `selection_correct` (and its failure detail)
        // now HAS a real qual.json-schema counterpart
        // (`TaskQualificationEvidence::selection_correct`/
        // `selection_check_failures`) — read it back rather than fabricate
        // it. `metrics::aggregate`, the sole consumer of this
        // reconstruction, does not read either field, but carrying the real
        // value costs nothing and keeps this function's own rule ("never
        // fabricated from absent data") literally true rather than merely
        // harmless.
        selection_correct: t.selection_correct,
        selection_check_failures: t.selection_check_failures.clone(),
        seed_commit: t.seed_commit.clone(),
        seeded_state: t.seeded_state,
        headless_autoland: t.headless_autoland.clone(),
        headless_drain_secs: t.headless_drain_secs.clone(),
        // Lane R17a: a qualification record has no `receipt_source`
        // counterpart (it is informational-only and deliberately excluded
        // from the gate) — never fabricated from absent data.
        receipt_source: None,
        // Lane K6: the qual.json schema DOES carry this — read it back
        // rather than fabricate it, same rule as `selection_correct` above.
        harness_timeout_secs: t.harness_timeout_secs,
    })
}

/// Select the one qualification record this invocation is grading out of a
/// `<FILE>.qual.json` sidecar's array (one entry per tier/arm the writing
/// `run`/`ab` invocation covered) — same never-guess precedence as
/// [`select_tier`].
pub fn select_qual_record<'a>(
    records: &'a [RunQualificationRecord],
    path: &Path,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<&'a RunQualificationRecord> {
    if records.is_empty() {
        bail!(
            "{}: no qualification records found — expected a `<run>.qual.json` sidecar array",
            path.display()
        );
    }
    if let (Some(p), Some(m)) = (provider, model) {
        return records
            .iter()
            .find(|r| r.provider == p && r.model == m)
            .ok_or_else(|| {
                anyhow!(
                    "{}: no qualification record matches --provider {p} --model {m}",
                    path.display()
                )
            });
    }
    if records.len() == 1 {
        return Ok(&records[0]);
    }
    bail!(
        "{}: contains {} qualification records — pass --provider/--model to select one",
        path.display(),
        records.len()
    );
}

/// Resolve (provider, model) from a selected [`RunQualificationRecord`] —
/// same never-guess precedence as [`resolve_identity`], except the record
/// always carries both (unlike [`TierRecord`]'s optional fields), so a flag
/// can only ever CONFIRM or CONFLICT with it, never fill a gap.
pub fn resolve_identity_from_qual_record(
    record: &RunQualificationRecord,
    path: &Path,
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
) -> Result<(String, String)> {
    if let Some(flag) = provider_flag {
        if flag != record.provider {
            bail!(
                "{}: qualification record provider '{}' conflicts with --provider '{flag}'",
                path.display(),
                record.provider
            );
        }
    }
    if let Some(flag) = model_flag {
        if flag != record.model {
            bail!(
                "{}: qualification record model '{}' conflicts with --model '{flag}'",
                path.display(),
                record.model
            );
        }
    }
    Ok((record.provider.clone(), record.model.clone()))
}

/// Build one [`RunEvidence`] from a selected, identity-resolved qualification
/// record. Unlike [`build_run_evidence`], a qual.json record carries no
/// `generated_at` of its own, so `--snapshot-date` is REQUIRED here (never
/// guessed). `dag_selection_pct` IS derivable since Lane R18c
/// (`TaskQualificationEvidence::selection_correct` now round-trips through
/// [`task_result_from_qual_evidence`] into the reconstructed
/// [`TaskResult`]s this function aggregates below) — computed the exact same
/// way [`build_run_evidence`]'s report-sourced path derives it
/// (`metrics::aggregate`'s own `dag_selection_pct`), so a qual.json-sourced
/// and a report-sourced `RunEvidence` for equivalent underlying evidence
/// agree on this field too, not just `pass_pct`/`cost_usd`/`latency_s`.
pub fn build_run_evidence_from_qual_json(
    record: &RunQualificationRecord,
    path: &Path,
    snapshot_date_flag: Option<&str>,
    role_smoke_default: Option<&str>,
) -> Result<RunEvidence> {
    let results: Vec<TaskResult> = record
        .tasks
        .iter()
        .map(task_result_from_qual_evidence)
        .collect::<Result<_>>()?;
    let agg = aggregate(&results);
    if agg.attempted == 0 {
        bail!(
            "{}: no attempted tasks in this qualification record — no evidence to grade",
            path.display()
        );
    }
    let pass_pct = agg.solved as f64 / agg.attempted as f64;
    let snapshot_date = match snapshot_date_flag {
        Some(raw) => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .with_context(|| format!("--snapshot-date {raw:?} is not YYYY-MM-DD"))?,
        None => bail!(
            "{}: qualification records carry no generated_at — pass --snapshot-date explicitly",
            path.display()
        ),
    };
    let smoke_passed_this_run = role_smoke_default.and_then(|id| {
        record
            .tasks
            .iter()
            .find(|t| t.task_id == id)
            .map(|t| t.result == "pass")
    });
    Ok(RunEvidence {
        run_id: record.run_id.clone(),
        snapshot_date,
        pass_pct,
        cost_usd: agg.total_cost_usd,
        latency_s: agg.median_duration_secs,
        smoke_passed_this_run,
        dag_selection_pct: agg.dag_selection_pct,
        source: EvidenceSource::QualJson,
    })
}

/// Load and resolve one `--run FILE` into an identity plus [`RunEvidence`],
/// preferring `<FILE>.qual.json` when it exists on disk (see the module
/// doc's "What counts as the record" section) and falling back to `FILE`
/// itself (parsed as a [`RunFile`] report) otherwise. This is the ONE place
/// that decides which artifact feeds a run — [`graduate_runs_into_directory`]
/// never re-derives that choice.
pub fn select_evidence_source(
    path: &Path,
    provider: Option<&str>,
    model: Option<&str>,
    snapshot_date_flag: Option<&str>,
    role_smoke_default: Option<&str>,
) -> Result<(String, String, RunEvidence)> {
    let sidecar = qual_json_sidecar_path(path);
    if sidecar.exists() {
        let raw = std::fs::read_to_string(&sidecar)
            .with_context(|| format!("reading qualification record from {}", sidecar.display()))?;
        let records: Vec<RunQualificationRecord> =
            serde_json::from_str(&raw).with_context(|| {
                format!(
                    "parsing qualification record JSON from {}",
                    sidecar.display()
                )
            })?;
        let record = select_qual_record(&records, &sidecar, provider, model)?;
        let (resolved_provider, resolved_model) =
            resolve_identity_from_qual_record(record, &sidecar, provider, model)?;
        let evidence = build_run_evidence_from_qual_json(
            record,
            &sidecar,
            snapshot_date_flag,
            role_smoke_default,
        )?;
        return Ok((resolved_provider, resolved_model, evidence));
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading run report from {}", path.display()))?;
    let file: RunFile = serde_json::from_str(&raw)
        .with_context(|| format!("parsing run report JSON from {}", path.display()))?;
    let tier = select_tier(&file, path, provider, model)?;
    let (resolved_provider, resolved_model) = resolve_identity(tier, path, provider, model)?;
    let evidence = build_run_evidence(&file, tier, path, snapshot_date_flag, role_smoke_default)?;
    Ok((resolved_provider, resolved_model, evidence))
}

/// Whether one run, on its own, qualifies toward the consecutive-streak
/// gate. See the module doc's "The graduation gate" section.
pub fn run_qualifies(run: &RunEvidence) -> bool {
    let smoke_ok = run.smoke_passed_this_run.unwrap_or(false);
    let micro_ok = run.pass_pct >= MICRO_SUITE_PASS_FLOOR;
    let dag_ok = run
        .dag_selection_pct
        .map(|p| p >= DAG_SELECTION_PASS_FLOOR)
        .unwrap_or(false);
    smoke_ok && micro_ok && dag_ok
}

/// The length of the trailing run of `true` values — same shape as
/// `qualification.rs`'s private `trailing_true_streak`, kept local because
/// only the [`REQUIRED_CONSECUTIVE_RUNS`] CONSTANT is shared, not the
/// helper (per the lane brief).
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

/// Fold an ordered batch of runs (oldest first) plus whatever the card
/// already carries for this role into a fresh, validated [`RoleEvidence`].
/// Pure: no IO, no wall clock (`today` is caller-supplied, matching
/// [`RoleEvidence::new`]'s own testable contract).
///
/// - `evidence_run_ids` accumulates onto `existing`'s ids (deduped,
///   order-preserving) — history is never lost.
/// - `micro_suite_pass_pct`/`observed_cost_usd`/`observed_latency_s` take
///   the LATEST run's value, falling back to `existing`'s when the latest
///   run didn't carry it (never regress an informational field to unknown
///   just because one run happened not to report it).
/// - `smoke_passed`/`dag_selection_pct` are set ONLY when at least one run
///   in `runs` measured them (latest wins among those that did); otherwise
///   `existing`'s value carries forward unchanged.
/// - `graduated` is the FRESH streak computed over `runs` alone (see the
///   module doc's "No incremental state").
pub fn compute_role_evidence(
    existing: Option<&RoleEvidence>,
    runs: &[RunEvidence],
    today: NaiveDate,
) -> Result<RoleEvidence, String> {
    if runs.is_empty() {
        return Err("compute_role_evidence: at least one run is required".to_string());
    }
    let mut evidence_run_ids: Vec<String> = existing
        .map(|e| e.evidence_run_ids.clone())
        .unwrap_or_default();
    for run in runs {
        if !evidence_run_ids.contains(&run.run_id) {
            evidence_run_ids.push(run.run_id.clone());
        }
    }

    let latest = runs.last().expect("checked non-empty above");
    let micro_suite_pass_pct = Some(latest.pass_pct);
    let observed_cost_usd = latest
        .cost_usd
        .or_else(|| existing.and_then(|e| e.observed_cost_usd));
    let observed_latency_s = latest
        .latency_s
        .or_else(|| existing.and_then(|e| e.observed_latency_s));
    let snapshot_date = Some(latest.snapshot_date);

    // smoke_passed / dag_selection_pct: latest MEASURED value among `runs`
    // wins; a run that didn't measure it never overwrites a prior reading.
    let smoke_passed = runs
        .iter()
        .rev()
        .find_map(|r| r.smoke_passed_this_run)
        .unwrap_or_else(|| existing.map(|e| e.smoke_passed).unwrap_or(false));
    let dag_selection_pct = runs
        .iter()
        .rev()
        .find_map(|r| r.dag_selection_pct)
        .or_else(|| existing.and_then(|e| e.dag_selection_pct));

    let qualifies: Vec<bool> = runs.iter().map(run_qualifies).collect();
    let streak = trailing_true_streak(qualifies.into_iter());
    let graduated = streak >= REQUIRED_CONSECUTIVE_RUNS;

    RoleEvidence::new(
        graduated,
        smoke_passed,
        micro_suite_pass_pct,
        dag_selection_pct,
        evidence_run_ids,
        observed_cost_usd,
        observed_latency_s,
        snapshot_date,
        today,
    )
}

// ── Directory JSON (surgical patch — see the module doc) ────────────────

/// The capability-directory file path THIS host's runtime would default to
/// — mirrors `crates/goose/src/config/paths.rs:6-13,36-38`'s `Paths::
/// base_dir()`/`in_data_dir("models")` plus `capability_directory.rs:348-
/// 350`'s `.join("capability_directory.json")`, by hand (see the module
/// doc's "duplication" note): `$PERMAGENT_PATH_ROOT/models/
/// capability_directory.json` if that env var is set, else `~/.permagent/
/// models/capability_directory.json`.
pub fn default_directory_path() -> Result<PathBuf> {
    let base = if let Ok(root) = std::env::var("PERMAGENT_PATH_ROOT") {
        PathBuf::from(root)
    } else {
        dirs::home_dir()
            .ok_or_else(|| {
                anyhow!(
                    "could not resolve a home directory for the default \
                 capability-directory path — pass --directory explicitly"
                )
            })?
            .join(".permagent")
    };
    Ok(base.join("models").join("capability_directory.json"))
}

/// An empty directory JSON value, in the same shape `CapabilityDirectory`
/// serializes.
pub fn empty_directory_json() -> Value {
    json!({
        "schema": SCHEMA_VERSION,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "cards": [],
    })
}

/// Load the directory JSON from `path`, tolerant of a missing file (an
/// empty directory, matching `CapabilityDirectory::load_persisted`'s own
/// "missing file -> None -> KB-only directory" tolerance).
pub fn load_directory_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing capability directory JSON from {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty_directory_json()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// A minimal, valid card JSON object for a model with no existing card.
/// Strengths are honestly zeroed (this module has no access to the KB — see
/// the module doc) rather than guessed; `permagent packs directory
/// --refresh` re-seeds real KB strengths onto this SAME card the next time
/// it runs discovery (`CapabilityDirectory::build_with_persisted` merges
/// persisted `roles` forward by `(provider, model)`), so evidence recorded
/// here is never lost, only enriched later.
fn new_card_json(provider: &str, model: &str) -> Value {
    let roles: serde_json::Map<String, Value> = [
        WorkflowRole::Orchestrate,
        WorkflowRole::Edit,
        WorkflowRole::Mechanical,
        WorkflowRole::Review,
        WorkflowRole::Local,
    ]
    .into_iter()
    .map(|r| {
        (
            r.as_str().to_string(),
            serde_json::to_value(RoleEvidence::default()).expect("RoleEvidence always serializes"),
        )
    })
    .collect();
    json!({
        "provider": provider,
        "model": model,
        "display_name": format!("{provider}/{model}"),
        "family": provider,
        "billing_class": "paid_api",
        "is_local": false,
        "context_window": 0,
        "input_usd_per_mtok": 0.0,
        "output_usd_per_mtok": 0.0,
        "strengths": {
            "orchestration": 0.0,
            "edit_format": 0.0,
            "mechanical": false,
            "review": false,
        },
        "roles": roles,
        "source": "evidence",
    })
}

/// Read the existing [`RoleEvidence`] for `(provider, model, role)` out of
/// `directory`, if a matching card and a parseable role entry exist.
pub fn existing_role_evidence(
    directory: &Value,
    provider: &str,
    model: &str,
    role: WorkflowRole,
) -> Option<RoleEvidence> {
    let cards = directory.get("cards")?.as_array()?;
    let card = cards.iter().find(|c| {
        c.get("provider").and_then(Value::as_str) == Some(provider)
            && c.get("model").and_then(Value::as_str) == Some(model)
    })?;
    let raw = card.get("roles")?.get(role.as_str())?;
    serde_json::from_value(raw.clone()).ok()
}

/// Apply `new_evidence` to `directory`'s card for `(provider, model)`,
/// creating the card (via [`new_card_json`]) if it does not exist yet.
/// Every other field of the directory/card/role JSON is left untouched.
pub fn apply_role_evidence_json(
    directory: &mut Value,
    provider: &str,
    model: &str,
    role: WorkflowRole,
    new_evidence: &RoleEvidence,
) -> Result<()> {
    if directory.get("cards").is_none() {
        *directory = empty_directory_json();
    }
    let cards = directory
        .get_mut("cards")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("directory JSON's `cards` field is not an array"))?;
    let idx = cards.iter().position(|c| {
        c.get("provider").and_then(Value::as_str) == Some(provider)
            && c.get("model").and_then(Value::as_str) == Some(model)
    });
    let idx = match idx {
        Some(i) => i,
        None => {
            cards.push(new_card_json(provider, model));
            cards.len() - 1
        }
    };
    let roles = cards[idx]
        .get_mut("roles")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("card JSON's `roles` field is not an object"))?;
    roles.insert(
        role.as_str().to_string(),
        serde_json::to_value(new_evidence).context("serializing the new role evidence")?,
    );
    Ok(())
}

/// Atomic write: a sibling `.tmp` file, then rename over the target —
/// mirrors `capability_directory.rs:452-461`'s `CapabilityDirectory::save`.
pub fn save_directory_json(directory: &Value, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(directory).context("serializing capability directory")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Human-readable before/after summary for `--dry-run` and the confirmation
/// line after a real write.
pub fn describe_change(
    provider: &str,
    model: &str,
    role: WorkflowRole,
    before: Option<&RoleEvidence>,
    after: &RoleEvidence,
) -> String {
    fn fmt_pct(p: Option<f64>) -> String {
        p.map(|v| format!("{:.1}%", v * 100.0))
            .unwrap_or_else(|| "n/a".to_string())
    }
    fn line(e: &RoleEvidence) -> String {
        format!(
            "graduated={} smoke_passed={} micro_suite={} dag_selection={} run_ids={} \
             snapshot_date={}",
            e.graduated,
            e.smoke_passed,
            fmt_pct(e.micro_suite_pass_pct),
            fmt_pct(e.dag_selection_pct),
            e.evidence_run_ids.len(),
            e.snapshot_date
                .map(|d| d.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
        )
    }
    let before_line = before.map(line).unwrap_or_else(|| {
        "graduated=false smoke_passed=false micro_suite=n/a dag_selection=n/a run_ids=0 \
         snapshot_date=n/a (new)"
            .to_string()
    });
    format!(
        "{provider}/{model} [{}]\n  before: {before_line}\n  after:  {}",
        role.as_str(),
        line(after)
    )
}

// ── Library entry point ──────────────────────────────────────────────────

/// Request for [`graduate_runs_into_directory`] — the real read/compute/
/// write path `permagent-eval graduate` (the CLI, `main.rs::cmd_graduate`)
/// runs. Exposed here as a library entry point (rather than living only in
/// the binary) so a caller OUTSIDE this crate — notably `crates/goose`'s
/// cross-crate schema-drift test (`cost_router::directory_evidence_drift`,
/// `#[cfg(test)]`-only, possible only from the goose side since THIS crate
/// cannot depend on `permagent` — see the module doc) — can exercise the
/// real thing instead of re-deriving it.
pub struct GraduateRequest<'a> {
    pub runs: &'a [PathBuf],
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub role: WorkflowRole,
    pub snapshot_date: Option<&'a str>,
    pub directory_path: &'a Path,
    pub dry_run: bool,
}

/// The resolved identity/role and before/after evidence from one
/// [`graduate_runs_into_directory`] call. `written` is `false` only for
/// `dry_run` requests — nothing is ever written in that case.
#[derive(Debug, Clone)]
pub struct GraduateOutcome {
    pub provider: String,
    pub model: String,
    pub role: WorkflowRole,
    pub before: Option<RoleEvidence>,
    pub after: RoleEvidence,
    pub written: bool,
    /// Which artifact fed each `--run` path, in the same order as
    /// `req.runs` — `(run_id, source)`. See [`EvidenceSource`]: this is how
    /// a caller sees, per run, whether the `<FILE>.qual.json` sidecar or the
    /// report itself actually supplied that run's fields.
    pub run_sources: Vec<(String, EvidenceSource)>,
}

/// Read `req.runs`, resolve identity/role, fold them into the capability
/// directory JSON at `req.directory_path` (creating it if absent), and
/// (unless `req.dry_run`) write the result back atomically. This is the
/// exact logic `cmd_graduate` runs — see that function for the CLI-specific
/// role-flag resolution and the printed summary, both of which stay in
/// `main.rs` since they are presentation, not the read/compute/write core.
pub fn graduate_runs_into_directory(req: &GraduateRequest) -> Result<GraduateOutcome> {
    let role_smoke_default = default_smoke_task_id(req.role);
    let mut identity: Option<(String, String)> = None;
    let mut run_evidences = Vec::new();
    let mut run_sources = Vec::new();
    for path in req.runs {
        let (provider, model, evidence) = select_evidence_source(
            path,
            req.provider,
            req.model,
            req.snapshot_date,
            role_smoke_default,
        )?;
        match &identity {
            None => identity = Some((provider.clone(), model.clone())),
            Some((p, m)) if p == &provider && m == &model => {}
            Some((p, m)) => bail!(
                "{}: reports provider={provider} model={model}, but this invocation is grading \
                 provider={p} model={m} (from an earlier --run file or --provider/--model) — \
                 pass matching runs only",
                path.display()
            ),
        }
        run_sources.push((evidence.run_id.clone(), evidence.source));
        run_evidences.push(evidence);
    }
    let Some((provider, model)) = identity else {
        bail!("at least one --run file is required");
    };

    let mut directory = load_directory_json(req.directory_path)?;
    let before = existing_role_evidence(&directory, &provider, &model, req.role);

    let today = chrono::Utc::now().date_naive();
    let after = compute_role_evidence(before.as_ref(), &run_evidences, today)
        .map_err(|e| anyhow!(e))
        .context("computing role evidence")?;

    if req.dry_run {
        return Ok(GraduateOutcome {
            provider,
            model,
            role: req.role,
            before,
            after,
            written: false,
            run_sources,
        });
    }

    apply_role_evidence_json(&mut directory, &provider, &model, req.role, &after)?;
    save_directory_json(&directory, req.directory_path)?;
    Ok(GraduateOutcome {
        provider,
        model,
        role: req.role,
        before,
        after,
        written: true,
        run_sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qualification::BinaryProvenance;

    fn today() -> NaiveDate {
        chrono::Utc::now().date_naive()
    }

    fn run(
        run_id: &str,
        date: NaiveDate,
        pass_pct: f64,
        smoke: Option<bool>,
        dag: Option<f64>,
    ) -> RunEvidence {
        RunEvidence {
            run_id: run_id.to_string(),
            snapshot_date: date,
            pass_pct,
            cost_usd: Some(0.01),
            latency_s: Some(3.0),
            smoke_passed_this_run: smoke,
            dag_selection_pct: dag,
            source: EvidenceSource::Report,
        }
    }

    fn qualifying_run(run_id: &str, date: NaiveDate) -> RunEvidence {
        run(run_id, date, 0.9, Some(true), Some(0.97))
    }

    // ── RoleEvidence gate: mirrors capability_directory.rs's own tests ────

    #[test]
    fn graduated_requires_evidence_ids_and_a_fresh_snapshot_date() {
        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            Vec::new(),
            None,
            None,
            Some(today()),
            today(),
        )
        .expect_err("graduated=true with zero evidence_run_ids must be rejected");
        assert!(err.contains("evidence_run_id"), "{err}");

        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            vec!["run-1".to_string()],
            None,
            None,
            None,
            today(),
        )
        .expect_err("graduated=true with no snapshot_date must be rejected");
        assert!(err.contains("snapshot_date"), "{err}");

        let stale = today() - chrono::Duration::days(KB_SNAPSHOT_STALE_AFTER_DAYS + 1);
        let err = RoleEvidence::new(
            true,
            true,
            Some(0.9),
            Some(0.95),
            vec!["run-1".to_string()],
            None,
            None,
            Some(stale),
            today(),
        )
        .expect_err("a stale snapshot_date must be rejected");
        assert!(err.contains("stale"), "{err}");
    }

    // ── RunFile parsing (a real `render_json` shape, unmodified) ─────────

    #[test]
    fn parses_a_plain_render_json_report_with_no_additive_fields() {
        let raw = r#"{
            "tiers": [{
                "tier": "sonnet5",
                "provider": "anthropic",
                "model": "claude-sonnet-5",
                "pinned_packs": true,
                "summary": {"attempted": 2, "solved": 2, "total_cost_usd": 0.5, "median_duration_secs": 12.0},
                "tasks": [
                    {"task_id": "fizzbuzz", "solved": true},
                    {"task_id": "roman", "solved": true}
                ]
            }]
        }"#;
        let file: RunFile =
            serde_json::from_str(raw).expect("a plain render_json report must parse");
        assert!(file.run_id.is_none());
        assert!(file.generated_at.is_none());
        assert!(file.dag_selection_pct.is_none());
        assert_eq!(file.tiers.len(), 1);
        assert_eq!(file.tiers[0].provider.as_deref(), Some("anthropic"));
    }

    // ── D4: fixture -> exact expected fields ──────────────────────────────

    #[test]
    fn fixture_evidence_produces_exact_expected_fields() {
        let runs = vec![run(
            "run-1",
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            0.9,
            Some(true),
            Some(0.97),
        )];
        let evidence = compute_role_evidence(None, &runs, today()).expect("must build");
        assert!(
            !evidence.graduated,
            "one qualifying run is below the 3-run streak requirement"
        );
        assert!((evidence.micro_suite_pass_pct.unwrap() - 0.9).abs() < 1e-9);
        assert!(evidence.smoke_passed);
        assert!((evidence.dag_selection_pct.unwrap() - 0.97).abs() < 1e-9);
        assert_eq!(evidence.evidence_run_ids, vec!["run-1".to_string()]);
        assert_eq!(
            evidence.snapshot_date,
            Some(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap())
        );
        assert_eq!(evidence.observed_cost_usd, Some(0.01));
        assert_eq!(evidence.observed_latency_s, Some(3.0));
    }

    #[test]
    fn a_79_percent_run_does_not_graduate() {
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        let runs = vec![
            run("r1", d(1), 0.79, Some(true), Some(0.97)),
            run("r2", d(2), 0.79, Some(true), Some(0.97)),
            run("r3", d(3), 0.79, Some(true), Some(0.97)),
        ];
        let evidence = compute_role_evidence(None, &runs, today()).unwrap();
        assert!(!evidence.graduated);
        assert!((evidence.micro_suite_pass_pct.unwrap() - 0.79).abs() < 1e-9);
    }

    #[test]
    fn evidence_beats_kb_and_revokes_a_prior_graduation_when_new_evidence_drops_below_floor() {
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 8, day).unwrap();
        let prior = RoleEvidence::new(
            true,
            true,
            Some(0.95),
            Some(0.98),
            vec!["r1".to_string(), "r2".to_string(), "r3".to_string()],
            Some(0.02),
            Some(4.0),
            Some(d(28)),
            today(),
        )
        .unwrap();
        assert!(prior.graduated);

        let new_run = run(
            "r4",
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            0.5,
            Some(true),
            Some(0.97),
        );
        let revoked = compute_role_evidence(Some(&prior), &[new_run], today()).unwrap();
        assert!(
            !revoked.graduated,
            "a below-floor run must revoke a prior graduated=true, not preserve it"
        );
        assert_eq!(
            revoked.evidence_run_ids,
            vec![
                "r1".to_string(),
                "r2".to_string(),
                "r3".to_string(),
                "r4".to_string()
            ]
        );
    }

    #[test]
    fn three_consecutive_qualifying_runs_graduate_two_do_not() {
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        let two = vec![qualifying_run("r1", d(1)), qualifying_run("r2", d(2))];
        let evidence_two = compute_role_evidence(None, &two, today()).unwrap();
        assert!(
            !evidence_two.graduated,
            "two qualifying runs must not graduate"
        );

        let three = vec![
            qualifying_run("r1", d(1)),
            qualifying_run("r2", d(2)),
            qualifying_run("r3", d(3)),
        ];
        let evidence_three = compute_role_evidence(None, &three, today()).unwrap();
        assert!(
            evidence_three.graduated,
            "three consecutive qualifying runs must graduate"
        );
    }

    #[test]
    fn a_broken_tail_resets_the_streak_even_with_three_total_runs() {
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        let runs = vec![
            qualifying_run("r1", d(1)),
            run("r2", d(2), 0.5, Some(true), Some(0.97)),
            qualifying_run("r3", d(3)),
        ];
        let evidence = compute_role_evidence(None, &runs, today()).unwrap();
        assert!(
            !evidence.graduated,
            "only the trailing 1-run streak counts, not 3 total qualifying runs"
        );
    }

    #[test]
    fn a_run_missing_dag_selection_never_qualifies_today() {
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        let runs = vec![
            run("r1", d(1), 1.0, Some(true), None),
            run("r2", d(2), 1.0, Some(true), None),
            run("r3", d(3), 1.0, Some(true), None),
        ];
        let evidence = compute_role_evidence(None, &runs, today()).unwrap();
        assert!(
            !evidence.graduated,
            "no dag_selection measurement must never graduate — do not fabricate the gate"
        );
        assert!(
            evidence.dag_selection_pct.is_none(),
            "an unmeasured field must stay unrated, not fabricated as passing"
        );
    }

    #[test]
    fn smoke_absent_from_a_runs_task_set_keeps_the_prior_reading_not_false() {
        let existing = RoleEvidence::new(
            false,
            true,
            Some(0.9),
            None,
            vec!["r0".to_string()],
            None,
            None,
            Some(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()),
            today(),
        )
        .unwrap();
        let new_run = run(
            "r1",
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            0.85,
            None,
            Some(0.9),
        );
        let evidence = compute_role_evidence(Some(&existing), &[new_run], today()).unwrap();
        assert!(
            evidence.smoke_passed,
            "absence must not overwrite a prior true reading with false"
        );
    }

    #[test]
    fn cost_and_latency_fall_back_to_the_prior_reading_when_a_run_omits_them() {
        let existing = RoleEvidence::new(
            false,
            false,
            Some(0.9),
            None,
            vec!["r0".to_string()],
            Some(0.5),
            Some(9.0),
            Some(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()),
            today(),
        )
        .unwrap();
        let mut new_run = qualifying_run("r1", NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        new_run.cost_usd = None;
        new_run.latency_s = None;
        let evidence = compute_role_evidence(Some(&existing), &[new_run], today()).unwrap();
        assert_eq!(evidence.observed_cost_usd, Some(0.5));
        assert_eq!(evidence.observed_latency_s, Some(9.0));
    }

    #[test]
    fn evidence_run_ids_accumulate_and_dedupe_across_invocations() {
        let existing = RoleEvidence::new(
            false,
            false,
            None,
            None,
            vec!["r1".to_string(), "r2".to_string()],
            None,
            None,
            None,
            today(),
        )
        .unwrap();
        let runs = vec![
            run(
                "r2",
                NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                0.9,
                Some(true),
                Some(0.97),
            ),
            run(
                "r3",
                NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
                0.9,
                Some(true),
                Some(0.97),
            ),
        ];
        let evidence = compute_role_evidence(Some(&existing), &runs, today()).unwrap();
        assert_eq!(
            evidence.evidence_run_ids,
            vec!["r1".to_string(), "r2".to_string(), "r3".to_string()]
        );
    }

    // ── identity resolution: never guessed ────────────────────────────────

    fn tier_record(provider: Option<&str>, model: Option<&str>) -> TierRecord {
        TierRecord {
            tier: Some("custom".to_string()),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            summary: SummaryRecord {
                attempted: 1,
                solved: 1,
                total_cost_usd: None,
                median_duration_secs: None,
            },
            tasks: vec![],
        }
    }

    #[test]
    fn a_run_without_provider_or_model_and_no_flags_is_refused_with_a_clear_error() {
        let tier = tier_record(None, None);
        let err = resolve_identity(&tier, Path::new("run.json"), None, None)
            .expect_err("missing provider/model with no flags must be refused");
        let msg = err.to_string();
        assert!(msg.contains("provider"), "{msg}");
        assert!(msg.contains("never guessed"), "{msg}");
    }

    #[test]
    fn explicit_flags_fill_a_record_missing_provider_and_model() {
        let tier = tier_record(None, None);
        let (p, m) = resolve_identity(
            &tier,
            Path::new("run.json"),
            Some("anthropic"),
            Some("claude-sonnet-5"),
        )
        .expect("flags must fill an absent record identity");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-5");
    }

    #[test]
    fn a_conflicting_flag_is_refused() {
        let tier = tier_record(Some("anthropic"), Some("claude-sonnet-5"));
        let err = resolve_identity(&tier, Path::new("run.json"), Some("openai"), None)
            .expect_err("a conflicting --provider must be refused");
        assert!(err.to_string().contains("conflicts"));
    }

    // ── tier selection ─────────────────────────────────────────────────

    #[test]
    fn a_multi_tier_file_with_no_flags_is_refused_as_ambiguous() {
        let file: RunFile = serde_json::from_str(
            r#"{"tiers": [
                {"provider": "a", "model": "x", "summary": {"attempted": 1, "solved": 1}},
                {"provider": "b", "model": "y", "summary": {"attempted": 1, "solved": 1}}
            ]}"#,
        )
        .unwrap();
        let err = select_tier(&file, Path::new("f.json"), None, None).unwrap_err();
        assert!(err.to_string().contains("2 tiers"));
    }

    #[test]
    fn compute_role_evidence_is_pure_and_deterministic() {
        let runs = vec![qualifying_run(
            "r1",
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        )];
        let a = compute_role_evidence(None, &runs, today()).unwrap();
        let b = compute_role_evidence(None, &runs, today()).unwrap();
        assert_eq!(a, b);
    }

    // ── directory JSON patching ────────────────────────────────────────

    #[test]
    fn apply_role_evidence_json_creates_a_new_card_when_absent() {
        let mut dir = empty_directory_json();
        let evidence = RoleEvidence::new(
            false,
            true,
            Some(0.9),
            None,
            vec!["r1".to_string()],
            Some(0.1),
            Some(2.0),
            Some(today()),
            today(),
        )
        .unwrap();
        apply_role_evidence_json(
            &mut dir,
            "acme",
            "made-up-model",
            WorkflowRole::Edit,
            &evidence,
        )
        .unwrap();
        let card = dir["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["provider"] == "acme" && c["model"] == "made-up-model")
            .expect("a new card must be created");
        assert_eq!(card["roles"]["edit"]["evidence_run_ids"][0], "r1");
        // Untouched roles remain the default, unrated state.
        assert_eq!(card["roles"]["review"]["graduated"], false);
        assert_eq!(card["source"], "evidence");
    }

    #[test]
    fn apply_role_evidence_json_updates_only_the_targeted_role_on_an_existing_card() {
        let mut dir = json!({
            "schema": 1,
            "generated_at": "2026-01-01T00:00:00Z",
            "cards": [{
                "provider": "anthropic",
                "model": "claude-sonnet-5",
                "display_name": "Claude Sonnet 5",
                "family": "anthropic",
                "billing_class": "paid_api",
                "is_local": false,
                "context_window": 400000,
                "input_usd_per_mtok": 3.0,
                "output_usd_per_mtok": 15.0,
                "strengths": {"orchestration": 0.8, "edit_format": 0.9, "mechanical": true, "review": true},
                "roles": {
                    "orchestrate": {},
                    "edit": {},
                    "mechanical": {},
                    "review": {},
                    "local": {},
                },
                "source": "knowledge_base",
            }],
        });
        let before_strengths = dir["cards"][0]["strengths"].clone();
        let evidence = RoleEvidence::new(
            false,
            true,
            Some(0.9),
            None,
            vec!["r1".to_string()],
            None,
            None,
            Some(today()),
            today(),
        )
        .unwrap();
        apply_role_evidence_json(
            &mut dir,
            "anthropic",
            "claude-sonnet-5",
            WorkflowRole::Orchestrate,
            &evidence,
        )
        .unwrap();
        assert_eq!(
            dir["cards"].as_array().unwrap().len(),
            1,
            "no duplicate card"
        );
        assert_eq!(
            dir["cards"][0]["strengths"], before_strengths,
            "untouched fields survive"
        );
        assert_eq!(
            dir["cards"][0]["roles"]["orchestrate"]["smoke_passed"],
            true
        );
        // Sibling roles on the SAME card are untouched.
        assert_eq!(dir["cards"][0]["roles"]["edit"], json!({}));
    }

    #[test]
    fn existing_role_evidence_reads_back_what_was_written() {
        let mut dir = empty_directory_json();
        let evidence = RoleEvidence::new(
            false,
            false,
            Some(0.5),
            None,
            vec!["r1".to_string()],
            None,
            None,
            Some(today()),
            today(),
        )
        .unwrap();
        apply_role_evidence_json(&mut dir, "acme", "m1", WorkflowRole::Review, &evidence).unwrap();
        let read_back = existing_role_evidence(&dir, "acme", "m1", WorkflowRole::Review).unwrap();
        assert_eq!(read_back, evidence);
        // A sibling role on the SAME (newly created) card is explicit but
        // unrated — `new_card_json` seeds every role, matching the real
        // crate's "unrated is never an absent key" philosophy
        // (`capability_directory.rs`'s own doc). It is `Some(default)`, not
        // `None` — a card genuinely absent from the directory is the only
        // case `existing_role_evidence` returns `None` for (see the next
        // assertion).
        let sibling = existing_role_evidence(&dir, "acme", "m1", WorkflowRole::Edit)
            .expect("every role on an existing card is explicit, never absent");
        assert_eq!(sibling, RoleEvidence::default());
        assert!(
            existing_role_evidence(&dir, "acme", "no-such-model", WorkflowRole::Review).is_none()
        );
    }

    #[test]
    fn load_directory_json_tolerates_a_missing_file() {
        let missing = Path::new("/tmp/permagent-eval-z4-does-not-exist-capdir.json");
        let dir = load_directory_json(missing).expect("a missing file must tolerate, not error");
        assert_eq!(dir["cards"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn save_and_load_directory_json_round_trips() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::remove_file(tmp.path()).ok();
        let mut dir = empty_directory_json();
        let evidence = RoleEvidence::new(
            false,
            false,
            Some(0.5),
            None,
            vec!["r1".to_string()],
            None,
            None,
            Some(today()),
            today(),
        )
        .unwrap();
        apply_role_evidence_json(&mut dir, "acme", "m1", WorkflowRole::Mechanical, &evidence)
            .unwrap();
        save_directory_json(&dir, tmp.path()).unwrap();
        let loaded = load_directory_json(tmp.path()).unwrap();
        assert_eq!(loaded, dir);
    }

    // ── graduate_runs_into_directory: the library entry point ─────────────

    #[test]
    fn graduate_runs_into_directory_writes_and_returns_the_outcome() {
        let tmp = tempfile::tempdir().unwrap();
        let directory_path = tmp.path().join("capability_directory.json");
        let run_path = tmp.path().join("run.json");
        std::fs::write(
            &run_path,
            serde_json::to_vec(&json!({
                "generated_at": "2026-09-01T00:00:00Z",
                "tiers": [{
                    "provider": "anthropic",
                    "model": "claude-sonnet-5",
                    "summary": {"attempted": 10, "solved": 9},
                    "tasks": [{"task_id": "fizzbuzz", "solved": true}],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = graduate_runs_into_directory(&GraduateRequest {
            runs: &[run_path],
            provider: None,
            model: None,
            role: WorkflowRole::Orchestrate,
            snapshot_date: None,
            directory_path: &directory_path,
            dry_run: false,
        })
        .expect("must succeed");
        assert!(outcome.written);
        assert_eq!(outcome.provider, "anthropic");
        assert_eq!(outcome.model, "claude-sonnet-5");
        assert!(outcome.before.is_none(), "no prior card existed");
        assert!((outcome.after.micro_suite_pass_pct.unwrap() - 0.9).abs() < 1e-9);
        let reloaded = load_directory_json(&directory_path).unwrap();
        let read_back = existing_role_evidence(
            &reloaded,
            "anthropic",
            "claude-sonnet-5",
            WorkflowRole::Orchestrate,
        )
        .unwrap();
        assert_eq!(read_back, outcome.after);
    }

    #[test]
    fn graduate_runs_into_directory_dry_run_does_not_write() {
        let tmp = tempfile::tempdir().unwrap();
        let directory_path = tmp.path().join("capability_directory.json");
        let run_path = tmp.path().join("run.json");
        std::fs::write(
            &run_path,
            serde_json::to_vec(&json!({
                "generated_at": "2026-09-01T00:00:00Z",
                "tiers": [{
                    "provider": "anthropic",
                    "model": "claude-sonnet-5",
                    "summary": {"attempted": 10, "solved": 9},
                    "tasks": [{"task_id": "fizzbuzz", "solved": true}],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = graduate_runs_into_directory(&GraduateRequest {
            runs: &[run_path],
            provider: None,
            model: None,
            role: WorkflowRole::Orchestrate,
            snapshot_date: None,
            directory_path: &directory_path,
            dry_run: true,
        })
        .unwrap();
        assert!(!outcome.written);
        assert!(!directory_path.exists());
    }

    // ── EvidenceSource: qual.json vs report equivalence ────────────────────

    fn qual_task(
        task_id: &str,
        result: &str,
        cost_usd: f64,
        seconds: f64,
    ) -> TaskQualificationEvidence {
        TaskQualificationEvidence {
            task_id: task_id.to_string(),
            result: result.to_string(),
            not_run: result == "not_run",
            errored: result == "errored",
            oracle_timed_out: None,
            oracle_secs: None,
            cost_usd: Some(cost_usd),
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            tool_calls: 0,
            seconds,
            seed_commit: None,
            headless_autoland: None,
            headless_drain_secs: None,
            harness_overhead: None,
            landed_goals: Vec::new(),
            drain_summary: None,
            create_roadmap_dispatched: None,
            reviewer_not_independent: None,
            reviewer_mutated: None,
            vacuous_pass: None,
            dag_enforcement: None,
            verification_integrity: None,
            safety_and_trust: None,
            cost_routing: None,
            selection_correct: None,
            selection_check_failures: Vec::new(),
            harness_timeout_secs: None,
            seeded_state: false,
        }
    }

    fn qual_record(
        run_id: &str,
        provider: &str,
        model: &str,
        cost_usd: f64,
    ) -> RunQualificationRecord {
        RunQualificationRecord {
            run_id: run_id.to_string(),
            fixture_revision: None,
            binary: BinaryProvenance {
                path: "permagent".to_string(),
                version: None,
                git_sha: None,
            },
            provider: provider.to_string(),
            model: model.to_string(),
            pinned_packs: true,
            tasks: vec![qual_task("fizzbuzz", "pass", cost_usd, 10.0)],
            missing: Vec::new(),
            capability_verdict: None,
            input_tokens_per_solved_median: None,
            token_verdict: None,
        }
    }

    /// [`build_run_evidence`] (report) and [`build_run_evidence_from_qual_json`]
    /// (qual.json) must derive IDENTICAL `pass_pct`/`cost_usd`/`latency_s`/
    /// `smoke_passed_this_run`/`snapshot_date` from equivalent underlying
    /// evidence — only `run_id` and `source` are allowed to differ, since a
    /// qual.json sidecar is independently stamped from its report (see
    /// `main.rs`'s `qualification_run_stamp` vs `cmd_run`'s `run_id`).
    #[test]
    fn qual_json_evidence_matches_the_equivalent_report_evidence() {
        let file = RunFile {
            run_id: Some("run-report-1".to_string()),
            generated_at: Some("2026-09-03T00:00:00Z".to_string()),
            dag_selection_pct: None,
            smoke_task_id: None,
            tiers: vec![TierRecord {
                tier: Some("sonnet5".to_string()),
                provider: Some("anthropic".to_string()),
                model: Some("claude-sonnet-5".to_string()),
                summary: SummaryRecord {
                    attempted: 1,
                    solved: 1,
                    total_cost_usd: Some(0.05),
                    median_duration_secs: Some(10.0),
                },
                tasks: vec![TaskRecord {
                    task_id: "fizzbuzz".to_string(),
                    solved: true,
                }],
            }],
        };
        let tier = &file.tiers[0];
        let report_evidence = build_run_evidence(
            &file,
            tier,
            Path::new("report.json"),
            None,
            Some("fizzbuzz"),
        )
        .expect("report evidence must build");

        let record = qual_record("run-qual-1", "anthropic", "claude-sonnet-5", 0.05);
        let qual_evidence = build_run_evidence_from_qual_json(
            &record,
            Path::new("report.json.qual.json"),
            Some("2026-09-03"),
            Some("fizzbuzz"),
        )
        .expect("qual.json evidence must build");

        assert_eq!(report_evidence.source, EvidenceSource::Report);
        assert_eq!(qual_evidence.source, EvidenceSource::QualJson);
        assert_eq!(qual_evidence.pass_pct, report_evidence.pass_pct);
        assert_eq!(qual_evidence.cost_usd, report_evidence.cost_usd);
        assert_eq!(qual_evidence.latency_s, report_evidence.latency_s);
        assert_eq!(
            qual_evidence.smoke_passed_this_run,
            report_evidence.smoke_passed_this_run
        );
        assert_eq!(qual_evidence.snapshot_date, report_evidence.snapshot_date);
    }

    /// Lane R18c-b (G23 LOW): `dag_selection_pct` used to be hardcoded
    /// `None` for every qual.json-sourced run, even when the record's own
    /// tasks carried a real `selection_correct` verdict (round-tripped
    /// since Lane R18c) — a real, computable value silently discarded.
    /// Must now derive it the SAME way the report-sourced path does.
    #[test]
    fn qual_json_evidence_derives_a_real_dag_selection_pct() {
        let mut record = qual_record("run-qual-2", "anthropic", "claude-sonnet-5", 0.05);
        let mut correct = qual_task("select-a", "pass", 0.01, 5.0);
        correct.selection_correct = Some(true);
        let mut wrong = qual_task("select-b", "pass", 0.01, 5.0);
        wrong.selection_correct = Some(false);
        wrong.selection_check_failures = vec!["must_call: escalate (never called)".to_string()];
        record.tasks.push(correct);
        record.tasks.push(wrong);

        let evidence = build_run_evidence_from_qual_json(
            &record,
            Path::new("run.json.qual.json"),
            Some("2026-09-03"),
            None,
        )
        .expect("qual.json evidence must build");

        // 1 of 2 selection tasks correct; the non-selection `fizzbuzz` task
        // from `qual_record`'s baseline does not count toward this metric.
        assert!(
            (evidence.dag_selection_pct.unwrap() - 0.5).abs() < 1e-12,
            "{:?}",
            evidence.dag_selection_pct
        );
    }

    /// A record with no `category: selection` tasks at all must keep
    /// `dag_selection_pct` `None` — never fabricated as `0.0` or `1.0`.
    #[test]
    fn qual_json_evidence_dag_selection_pct_is_none_with_no_selection_tasks() {
        let record = qual_record("run-qual-3", "anthropic", "claude-sonnet-5", 0.05);
        let evidence = build_run_evidence_from_qual_json(
            &record,
            Path::new("run.json.qual.json"),
            Some("2026-09-03"),
            None,
        )
        .expect("qual.json evidence must build");
        assert_eq!(evidence.dag_selection_pct, None);
    }

    /// When `<FILE>.qual.json` exists next to `FILE`, [`select_evidence_source`]
    /// reads it INSTEAD of `FILE` — proven here by giving the report a
    /// different provider than the sidecar and asserting the sidecar's
    /// provider wins.
    #[test]
    fn select_evidence_source_prefers_the_qual_json_sidecar_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let run_path = tmp.path().join("run.json");
        std::fs::write(
            &run_path,
            serde_json::to_vec(&json!({
                "generated_at": "2026-09-01T00:00:00Z",
                "tiers": [{
                    "provider": "wrong-provider",
                    "model": "wrong-model",
                    "summary": {"attempted": 1, "solved": 1},
                    "tasks": [{"task_id": "fizzbuzz", "solved": true}],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        let sidecar_path = qual_json_sidecar_path(&run_path);
        let records = vec![qual_record(
            "run-qual-1",
            "anthropic",
            "claude-sonnet-5",
            0.05,
        )];
        std::fs::write(&sidecar_path, serde_json::to_vec(&records).unwrap()).unwrap();

        let (provider, model, evidence) =
            select_evidence_source(&run_path, None, None, Some("2026-09-01"), Some("fizzbuzz"))
                .expect("must select the sidecar");
        assert_eq!(provider, "anthropic");
        assert_eq!(model, "claude-sonnet-5");
        assert_eq!(evidence.source, EvidenceSource::QualJson);
        assert_eq!(evidence.run_id, "run-qual-1");
    }

    /// End-to-end: `graduate_runs_into_directory` over three qual.json-only
    /// runs (no report file ever read — see [`select_evidence_source`])
    /// produces the SAME `RoleEvidence` as the equivalent three report-only
    /// runs, field for field.
    #[test]
    fn graduate_runs_into_directory_produces_identical_evidence_from_either_source() {
        let report_dir = tempfile::tempdir().unwrap();
        let report_directory_path = report_dir.path().join("capability_directory.json");
        let mut report_runs = Vec::new();
        for (i, run_id) in ["run-1", "run-2", "run-3"].into_iter().enumerate() {
            let path = report_dir.path().join(format!("r{i}.json"));
            std::fs::write(
                &path,
                serde_json::to_vec(&json!({
                    "run_id": run_id,
                    "generated_at": "2026-09-03T00:00:00Z",
                    "tiers": [{
                        "provider": "anthropic",
                        "model": "claude-sonnet-5",
                        "summary": {
                            "attempted": 1,
                            "solved": 1,
                            "total_cost_usd": 0.05,
                            "median_duration_secs": 10.0,
                        },
                        "tasks": [{"task_id": "fizzbuzz", "solved": true}],
                    }],
                }))
                .unwrap(),
            )
            .unwrap();
            report_runs.push(path);
        }
        let report_outcome = graduate_runs_into_directory(&GraduateRequest {
            runs: &report_runs,
            provider: None,
            model: None,
            role: WorkflowRole::Orchestrate,
            snapshot_date: None,
            directory_path: &report_directory_path,
            dry_run: false,
        })
        .expect("report-sourced graduation must succeed");
        assert!(report_outcome
            .run_sources
            .iter()
            .all(|(_, s)| *s == EvidenceSource::Report));

        let qual_dir = tempfile::tempdir().unwrap();
        let qual_directory_path = qual_dir.path().join("capability_directory.json");
        let mut qual_runs = Vec::new();
        for (i, run_id) in ["run-1", "run-2", "run-3"].into_iter().enumerate() {
            // The `--run` path itself is never read once its sidecar exists
            // (see `select_evidence_source`) — it need not even be a valid
            // report.
            let path = qual_dir.path().join(format!("q{i}.json"));
            let sidecar_path = qual_json_sidecar_path(&path);
            let records = vec![qual_record(run_id, "anthropic", "claude-sonnet-5", 0.05)];
            std::fs::write(&sidecar_path, serde_json::to_vec(&records).unwrap()).unwrap();
            qual_runs.push(path);
        }
        let qual_outcome = graduate_runs_into_directory(&GraduateRequest {
            runs: &qual_runs,
            provider: None,
            model: None,
            role: WorkflowRole::Orchestrate,
            snapshot_date: Some("2026-09-03"),
            directory_path: &qual_directory_path,
            dry_run: false,
        })
        .expect("qual.json-sourced graduation must succeed");
        assert!(qual_outcome
            .run_sources
            .iter()
            .all(|(_, s)| *s == EvidenceSource::QualJson));

        assert_eq!(qual_outcome.after, report_outcome.after);
        assert_eq!(qual_outcome.provider, report_outcome.provider);
        assert_eq!(qual_outcome.model, report_outcome.model);
    }
}
