//! Permagent eval harness — a lightweight, objective measure of how well the
//! Permagent coding harness solves real coding tasks, and at what dollar cost,
//! across model tiers (local Ollama vs cheap cloud vs frontier).
//!
//! The harness itself is invoked by shelling out to the `permagent` binary
//! (`permagent run --recipe permagent-coding …`). Everything else — loading the
//! curated task set, constructing the invocation, running each task's
//! deterministic test oracle, reading the run's cost from the per-call cost
//! ledger (#714), and computing pass-rate / $/solved / median $/task — lives
//! here as small, unit-tested pure logic.
//!
//! Module map:
//! - [`task`] — task spec loading + validation (the curated set is bundled under
//!   `tasks/`).
//! - [`tier`] — model-tier profiles (provider + model + cost-router pack pins).
//! - [`invocation`] — pure construction of the `permagent run` command line +
//!   environment for one (task, tier). This is the crux the whole harness turns
//!   on, so it is exhaustively tested.
//! - [`oracle`] — turning a test process exit code into pass/fail.
//! - [`cost`] — reading the total USD cost of a run from an isolated cost-ledger
//!   SQLite database, behind a [`cost::CostReader`] trait so it can be mocked.
//! - [`harness_log`] — best-effort tool-call / rate-limit / turn-limit signal
//!   extraction from the harness's captured stdout+stderr log.
//! - [`provider_unavailable`] — Lane K10: detecting a provider
//!   billing/auth/quota failure from a run's harness log and/or
//!   `llm_request.*.jsonl` logs, and deciding when a round of tasks against
//!   the same provider must stop launching more of them.
//! - [`selection`] — G3 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md`):
//!   the tool/DAG-selection micro-fixtures backing `dag_selection_pct` — whether
//!   the harness made the correct high-level decision (which tool, which order),
//!   evaluated against [`harness_log::LogSignals`], not the code it produced.
//! - [`graduate`] — D1 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md`
//!   G2): turn retained eval-run JSON reports into durable capability-directory
//!   `RoleEvidence` rows.
//! - [`metrics`] — aggregation: pass-rate, $/solved, median $/task, with all the
//!   awkward edge cases (0 solved, all solved, unknown/estimated costs).
//! - [`report`] — deterministic text / markdown / JSON rendering.
//! - [`paired`] — the statistics of a two-arm comparison: delta plus the
//!   30-run honesty guard that refuses to let a thin sample read as a verdict.
//! - [`ab`] — what a *configuration* is (an environment "arm"), the registry of
//!   harness knobs that can actually be flipped from the environment, and the
//!   serialisable A/B result file the `ab` and `report` subcommands read/write.
//! - [`tier0`] — the $0 offline loop metrics: a scripted stub model on loopback
//!   plus structural assertions over what the real harness asked it for.
//! - [`runner`] — the thin orchestration glue that ties the above together
//!   against real subprocesses (behind traits, so the decision logic is tested
//!   with mocks).
//! - [`swebench`] — SWE-bench as a task source: a pinned slice
//!   (`benchmarks/swe-arm-30.yaml`) materialized into ordinary task
//!   directories, judged by applying the agent's diff inside the instance's
//!   arm64 image on a remote Docker host over SSH, plus the upstream log
//!   parsers and resolved rule, and a $/solved summary.
//! - [`seed_state`] — Lane K8: a fixture may ship `seed/harness_state.json`
//!   describing pre-existing projects/roadmaps/goal cards to insert into a
//!   run's isolated store before the harness starts, so a selection fixture
//!   whose prose depends on pre-existing coordinator-visible state is
//!   actually satisfiable.

pub mod ab;
pub mod cost;
pub mod dispatch_inventory;
mod evidence_validity;
pub mod graduate;
pub mod harness_log;
pub mod invocation;
pub mod iteration;
pub mod metrics;
pub mod oracle;
pub mod paired;
pub mod program;
pub mod provider_unavailable;
pub mod qualification;
pub mod report;
pub mod runner;
pub mod seed_state;
pub mod selection;
pub mod swebench;
pub mod task;
pub mod tier;
pub mod tier0;

pub use ab::{AbReport, ArmConfig, Knob, Verdict, KNOBS};
pub use cost::{CostReader, CostReading, LedgerCostReader};
pub use dispatch_inventory::{
    scan_production_rust, DispatchInventory, DispatchSeam, SeamClassification, SeamKind,
    RULESET_VERSION as DISPATCH_INVENTORY_RULESET_VERSION,
};
pub use graduate::{
    apply_role_evidence_json, build_run_evidence, build_run_evidence_from_qual_json,
    compute_role_evidence, default_directory_path, default_smoke_task_id, describe_change,
    empty_directory_json, existing_role_evidence, graduate_runs_into_directory,
    load_directory_json, qual_json_sidecar_path, resolve_identity,
    resolve_identity_from_qual_record, resolve_run_id, resolve_snapshot_date, run_qualifies,
    save_directory_json, select_evidence_source, select_qual_record, select_tier,
    validate_graduation, EvidenceSource, GraduateOutcome, GraduateRequest, RoleEvidence,
    RunEvidence, RunFile, SummaryRecord, TaskRecord, TierRecord, WorkflowRole,
    DAG_SELECTION_PASS_FLOOR, DEFAULT_ORCHESTRATE_SMOKE_TASK_ID, KB_SNAPSHOT_STALE_AFTER_DAYS,
    MICRO_SUITE_PASS_FLOOR, SCHEMA_VERSION as CAPABILITY_DIRECTORY_SCHEMA_VERSION,
};
pub use harness_log::{
    scan as scan_harness_log, HarnessOverhead, LandedGoal, LogSignals, PlannerRoute,
};
pub use invocation::{build_invocation, recipe_with_prompt, Invocation};
pub use iteration::{
    run_training_loop, ArmRun, ArmRunner, DagEvidence, GraduationDecision, GraduationGates,
    GraduationStatus, IterationConfig, IterationReport, Observation, TokenTotals, TrainingReport,
};
pub use metrics::{aggregate, Aggregate, TaskResult};
pub use oracle::{outcome_from_exit, OracleOutcome};
pub use program::{
    ApprovalPolicy, DeliveryMode, ExitGateReceipt, ProgramDag, ProgramFrontier, ProgramNode,
    ProgramNodeStatus, ProgramReopen, ProgramReopenError, ProgramTransition,
    ProgramTransitionError,
};
pub use qualification::{
    build_run_qualification_record, qualification_input_from_records, qualify, AreaEvidence,
    AreaRating, AreaScore, BinaryProvenance, QualificationInput, QualificationReport,
    QualificationRun, RunQualificationRecord, TaskQualificationEvidence, NINE_AREAS,
};
pub use seed_state::{HarnessStateSeed, SeedGoal, SeedProject};
pub use selection::SelectionCheck;
pub use task::{Task, TaskSpec};
pub use tier::Tier;
pub use tier0::{Tier0Report, Tier0Scenario};
