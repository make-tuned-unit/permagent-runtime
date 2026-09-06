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
//! - [`metrics`] — aggregation: pass-rate, $/solved, median $/task, with all the
//!   awkward edge cases (0 solved, all solved, unknown/estimated costs).
//! - [`report`] — deterministic text / markdown / JSON rendering.
//! - [`runner`] — the thin orchestration glue that ties the above together
//!   against real subprocesses (behind traits, so the decision logic is tested
//!   with mocks).

pub mod cost;
pub mod dispatch_inventory;
pub mod harness_log;
pub mod invocation;
pub mod iteration;
pub mod metrics;
pub mod oracle;
pub mod paired;
pub mod program;
pub mod qualification;
pub mod report;
pub mod runner;
pub mod task;
pub mod tier;

pub use cost::{CostReader, CostReading, LedgerCostReader};
pub use dispatch_inventory::{
    scan_production_rust, DispatchInventory, DispatchSeam, SeamClassification, SeamKind,
    RULESET_VERSION as DISPATCH_INVENTORY_RULESET_VERSION,
};
pub use harness_log::{scan as scan_harness_log, LogSignals};
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
    qualify, AreaEvidence, AreaRating, AreaScore, QualificationInput, QualificationReport,
    QualificationRun,
};
pub use task::{Task, TaskSpec};
pub use tier::Tier;
