//! `permagent-eval` — run the curated coding task set against the Permagent
//! coding harness under one or more model tiers, then report pass-rate, $/solved
//! and median $/task.
//!
//! Subcommands:
//! - `run`      — execute tasks under the given tier(s) and print/write a report.
//! - `ab`       — run the suite twice under two configurations at ONE tier and
//!   report the paired delta, with the 30-run honesty guard enforced.
//! - `report`   — re-render a saved `ab` result file (no models are called).
//! - `knobs`    — list the harness knobs that are A/B-able from the environment.
//! - `tier0`    — the $0 offline harness-loop metrics against a scripted mock
//!   model on loopback (no provider is contacted, nothing is spent).
//! - `plan`     — print the exact `permagent run` invocations without executing
//!   (safe anywhere; no models are called).
//! - `list`     — list the bundled tasks.
//! - `validate` — load and validate every task spec.
//! - `qualify`  — validate retained held-out evidence and emit a scorecard.
//! - `graduate` — fold retained run reports into the capability directory's
//!   durable per-role evidence (D1, `docs/orchestrator/
//!   CODING_HARNESS_ROLE_GRADUATION_DAG.md` G2).

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use permagent_eval::{
    ExitGateReceipt, ProgramDag, ProgramFrontier, ProgramReopen, ProgramTransition,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
// NOTE: deliberately NOT `use std::process::Command;` — this file already
// declares its own CLI `enum Command` (the clap subcommand enum below), and
// importing the std type shadows/conflicts with every `Command::Variant`
// reference. New code that needs to spawn a process must fully-qualify
// `std::process::Command`.

use permagent_eval::ab::{self, AbReport, ArmConfig, Knob};
use permagent_eval::cost::LedgerCostReader;
use permagent_eval::graduate::{
    default_directory_path, describe_change, graduate_runs_into_directory, GraduateRequest,
    WorkflowRole,
};
use permagent_eval::invocation::build_invocation;
use permagent_eval::metrics::TaskResult;
use permagent_eval::qualification::{
    build_run_qualification_record, qualification_input_from_records, BinaryProvenance,
    QualificationInput, RunQualificationRecord,
};
use permagent_eval::report::{
    overall_dag_selection_pct, render_json, render_markdown, render_text, TierReport,
};
use permagent_eval::runner::{
    run_tier, BudgetTracker, Deps, RunConfig, SubprocessHarnessRunner, SubprocessOracleRunner,
    SubprocessRecipeSource,
};
use permagent_eval::task::{load_task_set, select_tasks, Task};
use permagent_eval::tier::Tier;
use permagent_eval::tier0;

/// The task set bundled with this crate (used when `--tasks-dir` is omitted).
const DEFAULT_TASKS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tasks");

#[derive(Parser, Debug)]
#[command(
    name = "permagent-eval",
    about = "Objectively measure the Permagent coding harness: pass-rate + $/solved, cheap vs frontier",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the task set under one or more tiers and report results.
    Run(RunArgs),
    /// Paired A/B: run the task set under two configurations at ONE tier,
    /// N repeats each, and report the delta with the honesty guard enforced.
    Ab(AbArgs),
    /// Re-render a saved `ab` result file. Calls no models.
    Report(ReportArgs),
    /// List the harness knobs that can be A/B'd from the environment.
    Knobs,
    /// Tier 0: the $0 offline harness-loop metrics against a scripted mock
    /// model on loopback. No provider is contacted and nothing is spent.
    Tier0(Tier0Args),
    /// Print the constructed harness invocations without running them.
    Plan(PlanArgs),
    /// List the bundled tasks.
    List(TasksDirArg),
    /// Validate every task spec (parse + invariants + id/dir coupling).
    Validate(TasksDirArg),
    /// Validate retained held-out evidence and emit its derived scorecard.
    Qualify(QualifyArgs),
    /// D1: fold retained run reports into the capability directory's
    /// durable per-role evidence (`docs/orchestrator/
    /// CODING_HARNESS_ROLE_GRADUATION_DAG.md` G2).
    Graduate(GraduateArgs),
    /// Inventory production provider/process dispatch seams without running a model.
    DispatchInventory(DispatchInventoryArgs),
    /// Inspect or advance the master program manifest. The existing Permagent
    /// roadmap remains the execution authority; this command only validates
    /// and records explicit child-DAG exit receipts.
    Program {
        #[command(subcommand)]
        command: ProgramCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ProgramCommand {
    /// Read-only manifest validation and frontier summary.
    Validate(ProgramInspectArgs),
    /// Read-only projection of active, ready, approval-required and blocked nodes.
    Frontier(ProgramInspectArgs),
    /// Apply explicit exit-gate receipts and optionally hand them to the
    /// authenticated daemon for roadmap continuation.
    Transition(ProgramTransitionArgs),
    /// Reopen a passed node after a retained regression and reset its descendants.
    Reopen(ProgramReopenArgs),
}

#[derive(Args, Debug)]
struct ProgramInspectArgs {
    /// Master program YAML manifest to inspect.
    #[arg(long, value_name = "FILE")]
    manifest: PathBuf,

    /// Summary format.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]),
        default_value = "text"
    )]
    format: String,
}

#[derive(Args, Debug)]
struct ProgramTransitionArgs {
    /// Master program YAML manifest to transition.
    #[arg(long, value_name = "FILE")]
    manifest: PathBuf,

    /// Active child-DAG node whose exit gates were completed.
    #[arg(long, value_name = "NODE")]
    node: String,

    /// Explicit receipt in the form `gate=passed` or `gate=failed`; repeat for
    /// every declared exit gate. Failed receipts are reported by the pure
    /// controller and never write a manifest.
    #[arg(long = "receipt", value_name = "GATE=STATUS", required = true)]
    receipts: Vec<String>,

    /// Existing mapped goal that completed this program node. Required with
    /// --daemon; no goals are created from a manifest path.
    #[arg(long, value_name = "GOAL-ID", requires = "daemon")]
    goal: Option<String>,

    /// Send the validated transition to the authenticated loopback daemon.
    /// Without this flag the command remains manifest-only.
    #[arg(long)]
    daemon: bool,

    /// Write the transitioned manifest to this new path atomically.
    #[arg(long, value_name = "FILE", conflicts_with = "in_place")]
    out: Option<PathBuf>,

    /// Atomically replace the input manifest after a successful transition.
    #[arg(long, conflicts_with = "out")]
    in_place: bool,

    /// Summary format printed after a successful in-memory transition.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]),
        default_value = "text"
    )]
    format: String,
}

#[derive(Args, Debug)]
struct ProgramReopenArgs {
    /// Master program YAML manifest to reopen.
    #[arg(long, value_name = "FILE")]
    manifest: PathBuf,

    /// Earliest passed child-DAG node that owns the retained regression.
    #[arg(long, value_name = "NODE")]
    node: String,

    /// Evidence-backed reason for reopening the node.
    #[arg(long, value_name = "REASON", required = true)]
    reason: String,

    /// Write the reopened manifest to this new path atomically.
    #[arg(long, value_name = "FILE", conflicts_with = "in_place")]
    out: Option<PathBuf>,

    /// Atomically replace the input manifest after a successful reopen.
    #[arg(long, conflicts_with = "out")]
    in_place: bool,

    /// Summary format printed after a successful in-memory reopen.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]),
        default_value = "text"
    )]
    format: String,
}

#[derive(Args, Debug)]
struct QualifyArgs {
    /// JSON evidence file produced by the outer evaluation DAG: the abstract
    /// `QualificationInput` shape (already classified per area). Use `-` for
    /// stdin. Mutually exclusive with `--record`.
    #[arg(long, value_name = "FILE", conflicts_with = "records")]
    input: Option<PathBuf>,

    /// One or more per-invocation qualification records — the `<out>.qual.json`
    /// sidecar `run`/`ab` write alongside their own `--out` — to fold into the
    /// nine tracked areas before validating. Repeatable; each file may hold
    /// one record or a JSON array of them (`run` writes one per tier, `ab`
    /// one per arm). Requires `--heldout-task` (at least one): which tasks are
    /// held-out is a policy decision the raw records do not carry.
    #[arg(long = "record", value_name = "FILE")]
    records: Vec<PathBuf>,

    /// Task id(s) used while optimizing a candidate (pairs with `--record`).
    #[arg(long = "optimizer-task", value_name = "ID")]
    optimizer_tasks: Vec<String>,

    /// Held-out task id(s) (pairs with `--record`); at least one is required
    /// when `--record` is used.
    #[arg(long = "heldout-task", value_name = "ID", requires = "records")]
    heldout_tasks: Vec<String>,

    /// Benchmark version label recorded on the derived `QualificationInput`
    /// when building it from `--record` files (ignored with `--input`, which
    /// already carries its own).
    #[arg(long, value_name = "LABEL", default_value = "eval-records-v1")]
    benchmark_version: String,

    /// Also write the derived scorecard to this file.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct GraduateArgs {
    /// One or more retained run report JSON files, as written by
    /// `permagent-eval run --format json --out FILE` (repeatable; passed in
    /// the order the streak should be evaluated, oldest first, when the
    /// files don't carry their own `generated_at`). When `FILE.qual.json`
    /// (the sidecar every `run`/`ab` invocation also writes) exists next to
    /// `FILE`, it is read INSTEAD of `FILE` for that run — see
    /// `permagent_eval::graduate`'s module doc's "What counts as the
    /// record" section; a qual.json-sourced run requires `--snapshot-date`
    /// since the sidecar carries no `generated_at`.
    #[arg(long = "run", value_name = "FILE", required = true)]
    runs: Vec<PathBuf>,

    /// Provider id, required only when a run record doesn't carry one.
    #[arg(long)]
    provider: Option<String>,

    /// Model string, required only when a run record doesn't carry one.
    #[arg(long)]
    model: Option<String>,

    /// Workflow role this evidence applies to: orchestrate (alias: hard),
    /// edit, mechanical, review, or local. Defaults to `orchestrate` (a
    /// `permagent-eval run --tier` invocation drives the harness's full
    /// main loop end-to-end on each task) unless --worker is given.
    #[arg(long)]
    role: Option<String>,

    /// This run represents worker-role (not coordinator) fixtures; role
    /// defaults to `edit` when --role is omitted (routing_matrix::
    /// capability_for_category's own doc: "EDIT ... is the evidence every
    /// fixture capability here ultimately measures").
    #[arg(long)]
    worker: bool,

    /// Fallback date (YYYY-MM-DD) for a run record with no `generated_at`.
    #[arg(long, value_name = "YYYY-MM-DD")]
    snapshot_date: Option<String>,

    /// Capability directory JSON file to read and update. Defaults to the
    /// same path the running harness uses
    /// (`permagent_eval::graduate::default_directory_path`, which mirrors
    /// `crates/goose/src/cost_router/capability_directory.rs`'s
    /// `CapabilityDirectory::default_path` by hand — see `graduate.rs`'s
    /// module doc for why this crate cannot call that function directly).
    #[arg(long, value_name = "FILE")]
    directory: Option<PathBuf>,

    /// Print the resulting card diff without writing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args, Debug)]
struct DispatchInventoryArgs {
    /// Production Rust source root to scan (for example crates/goose/src).
    #[arg(long, value_name = "DIR")]
    root: PathBuf,

    /// Require every paid-capable seam to have a valid wrapper/exclusion marker.
    #[arg(long)]
    strict: bool,

    /// Render JSON rather than the compact text inventory.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct TasksDirArg {
    /// Directory holding the task set (defaults to the bundled `tasks/`).
    #[arg(long, value_name = "DIR")]
    tasks_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct TierSelection {
    /// A built-in tier to run (repeatable). See `Tier::builtin_names()` for the
    /// full, current list (includes `local`, `kimi`, `minimax`, `sonnet`,
    /// `frontier` and the model-defaults-bench candidates: `haiku`, `sonnet5`,
    /// `glm53`, `glm47`, `minimax27`, `dschat`, `dsreason`, `kimi25`,
    /// `gpt54mini`).
    #[arg(long = "tier", value_name = "NAME")]
    tiers: Vec<String>,

    /// Ad-hoc tier: provider id (pair with --model).
    #[arg(long)]
    provider: Option<String>,

    /// Ad-hoc tier: model string (pair with --provider).
    #[arg(long)]
    model: Option<String>,

    /// Don't pin the cost-router packs to the tier (measure native routing).
    #[arg(long)]
    native_routing: bool,
}

impl TierSelection {
    /// Resolve the requested tiers, applying the pack-pinning choice. Repeated
    /// `--tier NAME` values are de-duplicated (first occurrence wins, preserving
    /// order) with a warning, so a tier is never accidentally run twice.
    fn resolve(&self) -> Result<Vec<Tier>> {
        let mut tiers: Vec<Tier> = Vec::new();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for name in &self.tiers {
            if !seen.insert(name.as_str()) {
                eprintln!("warning: tier {name:?} requested more than once; running it once");
                continue;
            }
            let tier = Tier::builtin(name).with_context(|| {
                format!(
                    "unknown tier {name:?}; built-in tiers are: {}",
                    Tier::builtin_names().join(", ")
                )
            })?;
            tiers.push(tier);
        }
        match (&self.provider, &self.model) {
            (Some(p), Some(m)) => tiers.push(Tier::custom("custom", p.clone(), m.clone())),
            (None, None) => {}
            _ => bail!("--provider and --model must be given together"),
        }
        if tiers.is_empty() {
            bail!(
                "no tier selected; pass --tier <name> (one of: {}) and/or --provider/--model",
                Tier::builtin_names().join(", ")
            );
        }
        if self.native_routing {
            tiers = tiers.into_iter().map(|t| t.with_pin_packs(false)).collect();
        }
        Ok(tiers)
    }
}

#[derive(Args, Debug)]
struct RunArgs {
    #[command(flatten)]
    tiers: TierSelection,

    #[command(flatten)]
    tasks_dir: TasksDirArg,

    /// Only run these task ids (repeatable). Omit to run all.
    #[arg(long = "task", value_name = "ID")]
    tasks: Vec<String>,

    /// The `permagent` binary to invoke (name on PATH or an absolute path).
    #[arg(long, default_value = "permagent")]
    permagent_bin: String,

    /// Where per-run scratch directories are created.
    #[arg(long, value_name = "DIR")]
    runs_dir: Option<PathBuf>,

    /// Report format.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["text", "md", "json"]),
        default_value = "text"
    )]
    format: String,

    /// Also write the report to this file. Also writes a sibling
    /// `<FILE>.qual.json` qualification record (task ids, fixture revision,
    /// binary provenance, provider/model/pins, and per-task evidence) that
    /// `permagent-eval qualify --record` folds into the nine tracked areas.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Keep scratch directories after each run (for debugging).
    #[arg(long)]
    keep: bool,

    /// Proceed even if a tier's API-key environment variable is absent.
    #[arg(long)]
    allow_missing_keys: bool,

    /// Exit non-zero unless at least one tier reaches this pass-rate percentage
    /// (0-100). Omit to always exit 0 regardless of outcome (the default, so
    /// existing behavior is unchanged). E.g. `--fail-under 80` fails CI when even
    /// the best tier solves under 80% of tasks.
    #[arg(long, value_name = "PERCENT")]
    fail_under: Option<f64>,

    /// Let the child `permagent` process read the OS keychain for provider
    /// secrets, instead of forcing it to read them from file/env only. Required
    /// when secrets are not present as environment variables — e.g. on a
    /// machine where they live only in the macOS keychain (service
    /// `permagent`, account `secrets`) and are read by the signed bundled CLI.
    /// Without this flag, `PERMAGENT_DISABLE_KEYRING=1` is set on every child
    /// AND any copy of it exported in your own shell is left alone (today's
    /// behavior, unchanged).
    #[arg(long)]
    use_keyring: bool,

    /// Stop launching further tasks once measured spend (summed from the
    /// ledger, across every tier and task run so far in this session) exceeds
    /// this many dollars. Already-collected results are still reported in
    /// full; remaining tasks are recorded as a distinct not-run state (not a
    /// fail) and excluded from pass-rate. Omit for no cap (default: unlimited).
    #[arg(long, value_name = "USD")]
    budget_usd: Option<f64>,

    /// Lane R1 (harness improvement loop): run against a provider-traffic
    /// cassette instead of a live model. Sets `PERMAGENT_PROVIDER_CASSETTE`
    /// on the child `permagent` process (same mechanism as an A/B arm env
    /// var, `RunConfig::arm_env`, applied last so it always wins) to
    /// `<cassette-mode>:<DIR>`. In `replay` mode (the default) the harness
    /// binary never constructs a real provider or spends anything for calls
    /// this cassette covers -- see `crates/goose/src/providers/cassette.rs`.
    /// Record one first with `--cassette-mode record`.
    #[arg(long, value_name = "DIR")]
    cassette: Option<PathBuf>,

    /// Whether `--cassette` records real provider traffic or replays a
    /// previously recorded one. Ignored without `--cassette`.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["record", "replay"]),
        default_value = "replay"
    )]
    cassette_mode: String,
}

/// `ab` — the paired comparison.
///
/// Exactly ONE tier is allowed: a delta measured across two models measures the
/// models, not the harness (see `docs/design/HARNESS_BENCH_RESEARCH.md` §1.5 on
/// holding a weak model constant).
#[derive(Args, Debug)]
struct AbArgs {
    #[command(flatten)]
    tiers: TierSelection,

    #[command(flatten)]
    tasks_dir: TasksDirArg,

    /// Only run these task ids (repeatable). Omit to run all.
    #[arg(long = "task", value_name = "ID")]
    tasks: Vec<String>,

    /// The `permagent` binary to invoke (name on PATH or an absolute path).
    #[arg(long, default_value = "permagent")]
    permagent_bin: String,

    /// Where per-run scratch directories are created.
    #[arg(long, value_name = "DIR")]
    runs_dir: Option<PathBuf>,

    /// Keep scratch directories after each run (for debugging).
    #[arg(long)]
    keep: bool,

    /// Proceed even if a tier's API-key environment variable is absent.
    #[arg(long)]
    allow_missing_keys: bool,

    /// Let the child `permagent` process read the OS keychain for provider
    /// secrets, instead of forcing it to read them from file/env only. See
    /// `run --help` for the full explanation.
    #[arg(long)]
    use_keyring: bool,

    /// Stop launching further tasks once measured spend (summed from the
    /// ledger, across both arms of this comparison) exceeds this many
    /// dollars. Already-collected results are still reported in full;
    /// remaining tasks are recorded as a distinct not-run state (not a fail)
    /// and excluded from pass-rate. Omit for no cap (default: unlimited).
    #[arg(long, value_name = "USD")]
    budget_usd: Option<f64>,

    /// A registry knob to compare off-vs-on (`permagent-eval knobs` lists them).
    /// Mutually exclusive with --a-env/--b-env.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["a_env", "b_env"])]
    knob: Option<String>,

    /// Ad-hoc control-arm environment, `KEY=VALUE` (repeatable).
    #[arg(long = "a-env", value_name = "KEY=VALUE")]
    a_env: Vec<String>,

    /// Ad-hoc treatment-arm environment, `KEY=VALUE` (repeatable).
    #[arg(long = "b-env", value_name = "KEY=VALUE")]
    b_env: Vec<String>,

    /// Label for the control arm (A).
    #[arg(long, value_name = "LABEL", default_value = "baseline")]
    baseline: String,

    /// Label for the treatment arm (B).
    #[arg(long = "candidate", value_name = "LABEL", default_value = "candidate")]
    candidate: String,

    /// How many times to run the whole suite per arm. The honesty guard needs
    /// 30 runs in the smaller arm, so on the bundled fixture suite that is
    /// `--repeats 2`; anything less reports INCONCLUSIVE by construction.
    #[arg(long, default_value_t = 1, value_name = "N")]
    repeats: usize,

    /// Where to write the machine-readable result (JSON). Defaults to a
    /// timestamped file under the runs directory; the path is always
    /// printed. Also writes a sibling `<FILE>.qual.json` qualification
    /// record per arm (task ids, fixture revision, binary provenance,
    /// provider/model/pins, and per-task evidence) that `permagent-eval
    /// qualify --record` folds into the nine tracked areas.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// Print the two arms and the planned invocation count, then exit WITHOUT
    /// running anything or spending anything.
    #[arg(long)]
    dry_run: bool,

    /// Exit non-zero when the run does not earn a directional verdict (i.e. the
    /// sample is under the honesty threshold). For CI, where an inconclusive
    /// A/B is a failed measurement rather than a result.
    #[arg(long)]
    require_conclusive: bool,
}

/// `tier0` — the $0 loop metrics.
#[derive(Args, Debug)]
struct Tier0Args {
    /// The `permagent` binary to drive (name on PATH or an absolute path).
    #[arg(long, default_value = "permagent")]
    permagent_bin: String,

    /// Only run these scenario ids (repeatable). Omit to run all.
    #[arg(long = "scenario", value_name = "ID")]
    scenarios: Vec<String>,

    /// Where per-scenario scratch directories are created (kept, so a failed
    /// scenario's harness log survives for inspection).
    #[arg(long, value_name = "DIR")]
    work_dir: Option<PathBuf>,

    /// Also write the machine-readable result here.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// List the scenarios and what each measures, then exit without running.
    #[arg(long)]
    list: bool,

    /// Exit non-zero if any scenario fails (the CI posture).
    #[arg(long)]
    fail_on_regression: bool,
}

#[derive(Args, Debug)]
struct ReportArgs {
    /// A result file written by `ab`.
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Output format: `md` re-renders the table, `json` re-emits the parsed
    /// file (a round-trip check that the file is well-formed).
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(["md", "json"]),
        default_value = "md"
    )]
    format: String,
}

#[derive(Args, Debug)]
struct PlanArgs {
    #[command(flatten)]
    tiers: TierSelection,

    #[command(flatten)]
    tasks_dir: TasksDirArg,

    /// Only plan these task ids (repeatable). Omit for all.
    #[arg(long = "task", value_name = "ID")]
    tasks: Vec<String>,

    /// The `permagent` binary name to show in the planned command.
    #[arg(long, default_value = "permagent")]
    permagent_bin: String,

    /// Plan as if `--use-keyring` were passed to `run` — shows
    /// `PERMAGENT_DISABLE_KEYRING` as removed (`env -u`) rather than set, so
    /// the planned invocation matches what `run --use-keyring` will actually
    /// do. See `run --help` for what the flag is for.
    #[arg(long)]
    use_keyring: bool,
}

fn resolve_tasks_dir(arg: &TasksDirArg) -> PathBuf {
    arg.tasks_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TASKS_DIR))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => cmd_run(args),
        Command::Ab(args) => cmd_ab(args),
        Command::Report(args) => cmd_report(args),
        Command::Knobs => cmd_knobs(),
        Command::Tier0(args) => cmd_tier0(args),
        Command::Plan(args) => cmd_plan(args),
        Command::List(arg) => cmd_list(&resolve_tasks_dir(&arg)),
        Command::Validate(arg) => cmd_validate(&resolve_tasks_dir(&arg)),
        Command::Qualify(args) => cmd_qualify(args),
        Command::Graduate(args) => cmd_graduate(args),
        Command::DispatchInventory(args) => cmd_dispatch_inventory(args),
        Command::Program { command } => cmd_program(command),
    }
}

fn cmd_dispatch_inventory(args: DispatchInventoryArgs) -> Result<()> {
    let inventory =
        permagent_eval::scan_production_rust(&args.root).map_err(|error| anyhow::anyhow!(error))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&inventory)?);
    } else {
        println!("dispatch inventory {}", inventory.ruleset);
        for seam in &inventory.seams {
            println!(
                "{}:{} {:?} {} {:?}",
                seam.path, seam.line, seam.kind, seam.symbol, seam.classification
            );
        }
        println!(
            "{} seam(s), {} strict failure(s)",
            inventory.seams.len(),
            inventory.strict_failures().len()
        );
    }
    if args.strict {
        inventory
            .validate_promotion()
            .map_err(|error| anyhow::anyhow!(error))?;
    }
    Ok(())
}

fn cmd_program(command: ProgramCommand) -> Result<()> {
    match command {
        ProgramCommand::Validate(args) => {
            let (program, frontier) = load_program_manifest(&args.manifest)?;
            render_program_validation(&program, &frontier, &args.format)
        }
        ProgramCommand::Frontier(args) => {
            let (_, frontier) = load_program_manifest(&args.manifest)?;
            render_program_frontier(&frontier, &args.format)
        }
        ProgramCommand::Transition(args) => cmd_program_transition(args),
        ProgramCommand::Reopen(args) => cmd_program_reopen(args),
    }
}

fn load_program_manifest(path: &Path) -> Result<(ProgramDag, ProgramFrontier)> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading program manifest from {}", path.display()))?;
    let program = ProgramDag::from_yaml(&raw)
        .with_context(|| format!("parsing program manifest from {}", path.display()))?;
    let frontier = program
        .validate()
        .with_context(|| format!("validating program manifest {}", path.display()))?;
    Ok((program, frontier))
}

fn render_program_validation(
    program: &ProgramDag,
    frontier: &ProgramFrontier,
    format: &str,
) -> Result<()> {
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "valid": true,
                "program_id": program.program_id,
                "objective": program.objective,
                "terminal_node": program.terminal_node,
                "frontier": frontier,
            }))?
        );
    } else {
        println!("OK: program {:?} is valid", program.program_id);
        println!("objective: {}", program.objective);
        println!("terminal_node: {}", program.terminal_node);
        print_frontier_text(frontier);
    }
    Ok(())
}

fn render_program_frontier(frontier: &ProgramFrontier, format: &str) -> Result<()> {
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(frontier)?);
    } else {
        print_frontier_text(frontier);
    }
    Ok(())
}

fn print_frontier_text(frontier: &ProgramFrontier) {
    println!("active: {}", display_nodes(&frontier.active));
    println!("ready: {}", display_nodes(&frontier.ready));
    println!(
        "approval_required: {}",
        display_nodes(&frontier.approval_required)
    );
    println!("blocked: {}", display_nodes(&frontier.blocked));
    println!("complete: {}", frontier.complete);
}

fn display_nodes(nodes: &[String]) -> String {
    if nodes.is_empty() {
        "(none)".to_owned()
    } else {
        nodes.join(", ")
    }
}

fn cmd_program_transition(args: ProgramTransitionArgs) -> Result<()> {
    let source_manifest = std::fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading program manifest {}", args.manifest.display()))?;
    let (mut program, _) = load_program_manifest(&args.manifest)?;
    let receipts = args
        .receipts
        .iter()
        .map(|raw| parse_exit_gate_receipt(raw))
        .collect::<Result<Vec<_>>>()?;

    let daemon_response = if args.daemon {
        // The daemon is authoritative for a handoff. In particular, an
        // in-place retry starts from the daemon's already-transitioned
        // manifest (the source node is Passed), so applying the pure local
        // Active -> Passed transition first would incorrectly reject a safe
        // durable retry. Shape validation still happens before the request;
        // lifecycle/completion validation remains in the authenticated daemon.
        program.validate().context("validating program manifest")?;
        let goal = args
            .goal
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--goal is required with --daemon"))?;
        Some(post_program_handoff(
            goal,
            &args.node,
            &source_manifest,
            &receipts,
        )?)
    } else {
        None
    };
    let transition = if let Some(response) = daemon_response.as_ref() {
        daemon_response_transition(response)?
    } else {
        program
            .transition_node(&args.node, &receipts)
            .map_err(|error| anyhow::anyhow!(error))?
    };

    let destination = match (args.out, args.in_place) {
        (Some(_), true) => unreachable!("clap rejects --out with --in-place"),
        (Some(path), false) => Some(path),
        (None, true) => Some(args.manifest.clone()),
        (None, false) => None,
    };
    let rendered_manifest = daemon_response
        .as_ref()
        .map(|response| response.manifest.clone())
        .unwrap_or_else(|| serde_yaml::to_string(&program).expect("validated program serializes"));
    if let Some(path) = &destination {
        atomic_write(path, rendered_manifest.as_bytes()).with_context(|| {
            format!("atomically writing program manifest to {}", path.display())
        })?;
    }
    render_program_transition(
        &transition,
        destination.as_deref(),
        &args.format,
        daemon_response
            .as_ref()
            .map(|response| response.status.as_str()),
    )
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonHandoffResponse {
    status: String,
    program_id: String,
    node_id: String,
    activated: Vec<String>,
    approval_required: Vec<String>,
    manifest: String,
}

fn daemon_response_transition(response: &DaemonHandoffResponse) -> Result<ProgramTransition> {
    let program = ProgramDag::from_yaml(&response.manifest)
        .context("decoding transitioned daemon manifest")?;
    if response.program_id != program.program_id {
        bail!("daemon response program_id does not match its manifest");
    }
    if response.node_id.trim().is_empty() {
        bail!("daemon response node_id is empty");
    }
    if !program.nodes.iter().any(|node| node.id == response.node_id) {
        bail!("daemon response node_id is absent from its manifest");
    }
    let frontier = program
        .validate()
        .context("validating transitioned daemon manifest")?;
    Ok(ProgramTransition {
        node_id: response.node_id.clone(),
        activated: response.activated.clone(),
        approval_required: response.approval_required.clone(),
        frontier,
    })
}

fn daemon_port() -> u16 {
    let Some(home) = dirs::home_dir() else {
        return 3001;
    };
    let path = home.join(".permagent/config.yaml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_yaml::from_str::<serde_yaml::Value>(&contents).ok())
        .and_then(|yaml| {
            yaml.get("daemon")
                .and_then(|d| d.get("port"))
                .and_then(|p| p.as_u64())
        })
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(3001)
}

fn daemon_token() -> Result<String> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    let path = home.join(".permagent/secrets/daemon_token.json");
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading daemon token from {}", path.display()))?;
    serde_json::from_str::<serde_json::Value>(&contents)
        .context("parsing daemon token")?
        .get("token")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .context("daemon token is missing")
}

fn post_program_handoff(
    source_goal_id: &str,
    node_id: &str,
    manifest: &str,
    receipts: &[ExitGateReceipt],
) -> Result<DaemonHandoffResponse> {
    let token = daemon_token()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building daemon handoff client")?;
    let response = client
        .post(format!(
            "http://127.0.0.1:{}/api/program/handoff",
            daemon_port()
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "sourceGoalId": source_goal_id,
            "nodeId": node_id,
            "manifest": manifest,
            "receipts": receipts,
        }))
        .send()
        .context("posting program handoff to daemon")?;
    let status = response.status();
    let body = response.text().context("reading daemon handoff response")?;
    if !status.is_success() {
        let bounded: String = body.chars().take(500).collect();
        bail!("daemon program handoff failed ({status}): {bounded}");
    }
    serde_json::from_str(&body).context("decoding daemon program handoff response")
}

fn cmd_program_reopen(args: ProgramReopenArgs) -> Result<()> {
    let (mut program, _) = load_program_manifest(&args.manifest)?;
    let reopened = program
        .reopen_for_regression(&args.node, &args.reason)
        .map_err(|error| anyhow::anyhow!(error))?;

    let destination = match (args.out, args.in_place) {
        (Some(_), true) => unreachable!("clap rejects --out with --in-place"),
        (Some(path), false) => Some(path),
        (None, true) => Some(args.manifest.clone()),
        (None, false) => None,
    };
    let rendered_manifest =
        serde_yaml::to_string(&program).context("serializing program manifest")?;
    if let Some(path) = &destination {
        atomic_write(path, rendered_manifest.as_bytes()).with_context(|| {
            format!("atomically writing program manifest to {}", path.display())
        })?;
    }
    render_program_reopen(&reopened, destination.as_deref(), &args.format)
}

fn parse_exit_gate_receipt(raw: &str) -> Result<ExitGateReceipt> {
    let (gate, status) = raw
        .rsplit_once('=')
        .ok_or_else(|| anyhow::anyhow!("invalid receipt {raw:?}; expected GATE=passed|failed"))?;
    let gate = gate.trim();
    if gate.is_empty() {
        bail!("invalid receipt {raw:?}; gate name cannot be empty");
    }
    match status.trim() {
        "passed" => Ok(ExitGateReceipt::passed(gate)),
        "failed" => Ok(ExitGateReceipt::failed(gate)),
        other => bail!("invalid receipt status {other:?} in {raw:?}; expected passed or failed"),
    }
}

fn render_program_transition(
    transition: &ProgramTransition,
    destination: Option<&Path>,
    format: &str,
    daemon_status: Option<&str>,
) -> Result<()> {
    if format == "json" {
        let mut value = serde_json::to_value(transition)?;
        if let Some(path) = destination {
            value["manifest_written"] = serde_json::json!(path);
        }
        if let Some(status) = daemon_status {
            value["daemon_handoff"] = serde_json::json!(status);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("passed node: {}", transition.node_id);
        println!("activated: {}", display_nodes(&transition.activated));
        println!(
            "approval_required: {}",
            display_nodes(&transition.approval_required)
        );
        if let Some(path) = destination {
            println!("manifest written: {}", path.display());
        } else {
            println!("manifest not written (read-only; pass --out or --in-place)");
        }
        if let Some(status) = daemon_status {
            println!("daemon handoff: {status}");
        }
        print_frontier_text(&transition.frontier);
    }
    Ok(())
}

fn render_program_reopen(
    reopened: &ProgramReopen,
    destination: Option<&Path>,
    format: &str,
) -> Result<()> {
    if format == "json" {
        let mut value = serde_json::to_value(reopened)?;
        if let Some(path) = destination {
            value["manifest_written"] = serde_json::json!(path);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("reopened node: {}", reopened.node_id);
        println!("reason: {}", reopened.reason);
        println!(
            "reset descendants: {}",
            display_nodes(&reopened.reset_descendants)
        );
        println!("approval_required: {}", reopened.approval_required);
        if let Some(path) = destination {
            println!("manifest written: {}", path.display());
        } else {
            println!("manifest not written (read-only; pass --out or --in-place)");
        }
        print_frontier_text(&reopened.frontier);
    }
    Ok(())
}

/// Write bytes using a same-directory temporary file followed by rename. No
/// caller invokes this until parsing, validation, receipts, and the pure
/// transition have all succeeded, so failed/replayed transitions leave the
/// original manifest untouched.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!("output parent {} is not a directory", parent.display());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("output path must have a valid file name"))?;
    let pid = std::process::id();
    let mut temp_path = None;
    let mut temp_file = None;
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".{file_name}.permagent-eval-{pid}-{attempt}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temp_path = Some(candidate);
                temp_file = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let temp_path =
        temp_path.ok_or_else(|| anyhow::anyhow!("could not allocate temporary output file"))?;
    let mut temp_file = temp_file.expect("temporary path and file are paired");
    let result = (|| -> Result<()> {
        temp_file.write_all(bytes)?;
        temp_file.sync_all()?;
        fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// Read one `--record` file's qualification record(s). Each file may hold a
/// single [`RunQualificationRecord`] object (what `run`/`ab` write for one
/// tier/arm) or a JSON array of them (what `run` writes when it ran more
/// than one tier in a single invocation) — tried in that order so either
/// shape round-trips.
fn read_qualification_records(path: &Path) -> Result<Vec<RunQualificationRecord>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading qualification record {}", path.display()))?;
    if let Ok(many) = serde_json::from_str::<Vec<RunQualificationRecord>>(&raw) {
        return Ok(many);
    }
    let one: RunQualificationRecord = serde_json::from_str(&raw).with_context(|| {
        format!(
            "{} is neither a qualification record nor an array of them",
            path.display()
        )
    })?;
    Ok(vec![one])
}

fn cmd_qualify(args: QualifyArgs) -> Result<()> {
    let input: QualificationInput = if !args.records.is_empty() {
        if args.heldout_tasks.is_empty() {
            bail!(
                "--record requires at least one --heldout-task: the held-out/optimizer split \
                 is a policy decision, not something a raw run record carries"
            );
        }
        let mut records = Vec::new();
        for path in &args.records {
            records.extend(read_qualification_records(path)?);
        }
        qualification_input_from_records(
            &records,
            args.benchmark_version.clone(),
            args.optimizer_tasks.clone(),
            args.heldout_tasks.clone(),
        )
    } else {
        let path = args
            .input
            .as_deref()
            .context("either --input or --record is required")?;
        let raw = if path == Path::new("-") {
            let mut raw = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)
                .context("reading qualification evidence from stdin")?;
            raw
        } else {
            std::fs::read_to_string(path).with_context(|| {
                format!("reading qualification evidence from {}", path.display())
            })?
        };
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing qualification evidence JSON from {}",
                path.display()
            )
        })?
    };
    let report = permagent_eval::qualify(&input).context("validating qualification evidence")?;
    let rendered = serde_json::to_string_pretty(&report)?;
    println!("{rendered}");
    if let Some(path) = &args.out {
        std::fs::write(path, format!("{rendered}\n"))
            .with_context(|| format!("writing scorecard to {}", path.display()))?;
        eprintln!("scorecard written to {}", path.display());
    }
    Ok(())
}

/// Resolve the target role from `--role`/`--worker` (see [`GraduateArgs`]'s
/// doc for the precedence). Never guesses beyond that documented default.
fn resolve_graduate_role(args: &GraduateArgs) -> Result<WorkflowRole> {
    if let Some(tag) = &args.role {
        return WorkflowRole::from_tag(tag).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown role '{}' — expected one of: orchestrate (alias: hard), edit, \
                 mechanical, review, local",
                tag
            )
        });
    }
    if args.worker {
        return Ok(WorkflowRole::Edit);
    }
    Ok(WorkflowRole::Orchestrate)
}

/// Thin CLI wrapper over the library entry point
/// [`permagent_eval::graduate::graduate_runs_into_directory`]: resolve the
/// `--role`/`--worker` flags and the default directory path (both
/// CLI-specific), delegate the real read/compute/write work, then print the
/// before/after summary. Kept thin deliberately — the read/compute/write
/// core lives in the library so it is reachable from OUTSIDE this crate too
/// (see that function's doc for why: `crates/goose`'s cross-crate
/// schema-drift test).
fn cmd_graduate(args: GraduateArgs) -> Result<()> {
    let role = resolve_graduate_role(&args)?;
    let directory_path = match &args.directory {
        Some(p) => p.clone(),
        None => default_directory_path()?,
    };
    let outcome = graduate_runs_into_directory(&GraduateRequest {
        runs: &args.runs,
        provider: args.provider.as_deref(),
        model: args.model.as_deref(),
        role,
        snapshot_date: args.snapshot_date.as_deref(),
        directory_path: &directory_path,
        dry_run: args.dry_run,
    })?;

    let summary = describe_change(
        &outcome.provider,
        &outcome.model,
        outcome.role,
        outcome.before.as_ref(),
        &outcome.after,
    );
    println!("{summary}");
    for (run_id, source) in &outcome.run_sources {
        println!("  evidence: {run_id} <- {}", source.as_str());
    }
    if outcome.written {
        println!("\nWrote {}", directory_path.display());
    } else {
        println!("\n(dry run — nothing written)");
    }
    Ok(())
}

fn cmd_list(tasks_dir: &Path) -> Result<()> {
    let tasks = load_task_set(tasks_dir)?;
    println!("{} tasks in {}", tasks.len(), tasks_dir.display());
    for t in &tasks {
        println!("  {:<20} [{}] {}", t.spec.id, t.spec.category, t.spec.title);
    }
    Ok(())
}

fn cmd_validate(tasks_dir: &Path) -> Result<()> {
    let tasks = load_task_set(tasks_dir)?;
    for t in &tasks {
        // Extra on-disk coupling checks beyond spec.validate().
        if !t.has_oracle() && !t.has_workspace() {
            eprintln!(
                "warning: task {:?} has neither a workspace/ nor an oracle/ directory",
                t.spec.id
            );
        }
    }
    println!(
        "OK: {} task(s) valid in {}",
        tasks.len(),
        tasks_dir.display()
    );
    Ok(())
}

fn cmd_plan(args: PlanArgs) -> Result<()> {
    let tiers = args.tiers.resolve()?;
    let tasks_dir = resolve_tasks_dir(&args.tasks_dir);
    let tasks = select_tasks(load_task_set(&tasks_dir)?, &args.tasks)?;
    let workdir = Path::new("<workspace>");
    let data_root = Path::new("<data-root>");
    for tier in &tiers {
        println!("# tier: {} ({} {})", tier.name, tier.provider, tier.model);
        for task in &tasks {
            // `plan` never touches disk or shells out (it's "safe anywhere; no
            // models are called"), so the recipe path shown here is the SHAPE
            // `run` would actually write, not a real file. The task's prompt
            // is never a separate `-t`/`--text` flag — it is embedded inside
            // that recipe file (`--recipe` and `-t` are mutually exclusive on
            // the CLI: crates/goose-cli/src/cli.rs:188-220).
            let recipe_path = data_root.join(format!("{}-recipe.yaml", task.spec.id));
            let inv = build_invocation(
                &task.spec,
                tier,
                workdir,
                data_root,
                &recipe_path,
                &args.permagent_bin,
                args.use_keyring,
            );
            println!("## {}", task.spec.id);
            println!("{}", inv.display_line());
            println!(
                "  (prompt for {:?} is embedded in {} — see `recipe_with_prompt`; \
                 --recipe and -t/--text cannot both be passed)\n",
                task.spec.id,
                recipe_path.display()
            );
        }
    }
    Ok(())
}

/// Add `run_id`/`generated_at` to a `render_json` report's top-level object
/// — pure so it is unit-tested without shelling out to the harness (see
/// `cmd_run`'s call site for why this is additive envelope metadata rather
/// than a `report.rs` change).
fn stamp_json_envelope(
    mut value: serde_json::Value,
    run_id: &str,
    generated_at: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("run_id".to_string(), serde_json::json!(run_id));
        obj.insert(
            "generated_at".to_string(),
            serde_json::json!(generated_at.to_rfc3339()),
        );
    }
    value
}

/// G3 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md`): add the
/// run-wide `dag_selection_pct` to a `render_json` report's top-level object
/// — the exact field `permagent-eval graduate`'s `RunFile::dag_selection_pct`
/// already reads (`graduate.rs`), populated for real once this run attempted
/// at least one `category: "selection"` fixture. Kept as its own small
/// function (rather than folded into `stamp_json_envelope`) so that
/// function's own D2 contract/tests are untouched — see its call site in
/// `cmd_run`.
fn stamp_dag_selection_pct(mut value: serde_json::Value, pct: Option<f64>) -> serde_json::Value {
    if let (Some(obj), Some(p)) = (value.as_object_mut(), pct) {
        obj.insert("dag_selection_pct".to_string(), serde_json::json!(p));
    }
    value
}

/// The env var name the harness under test reads to install a provider
/// cassette. Mirrors `crate::providers::cassette::
/// PERMAGENT_PROVIDER_CASSETTE_ENV` in the `permagent` crate — this binary
/// deliberately does not depend on that crate, so the name is duplicated
/// here and pinned by `cassette_arg_becomes_the_provider_cassette_env_var`.
const PERMAGENT_PROVIDER_CASSETTE_ENV: &str = "PERMAGENT_PROVIDER_CASSETTE";

/// Lane R1: `--cassette <dir>` rides the SAME "arm env wins last" seam
/// `runner::run_task` already gives an A/B arm ([`RunConfig::arm_env`],
/// applied after the invocation's own defaults — runner.rs's
/// `inv.envs.extend(cfg.arm_env...)`), so a plain `run` needs no new
/// plumbing in `runner.rs`/`invocation.rs` to reach the child `permagent`
/// process. Empty without `--cassette`, so a normal run is byte-for-byte
/// unchanged.
fn cassette_arm_env(cassette: Option<&Path>, mode: &str) -> Vec<(String, String)> {
    match cassette {
        Some(dir) => vec![(
            PERMAGENT_PROVIDER_CASSETTE_ENV.to_string(),
            format!("{}:{}", mode, dir.display()),
        )],
        None => Vec::new(),
    }
}

/// Whether this run will be served entirely from a cassette — i.e. no real
/// provider is constructed, nothing is spent, and no credential is needed.
fn cassette_is_replaying(cassette: Option<&Path>, mode: &str) -> bool {
    cassette.is_some() && mode == "replay"
}

fn cmd_run(args: RunArgs) -> Result<()> {
    let tiers = args.tiers.resolve()?;
    let tasks_dir = resolve_tasks_dir(&args.tasks_dir);
    let tasks: Vec<Task> = select_tasks(load_task_set(&tasks_dir)?, &args.tasks)?;
    if tasks.is_empty() {
        bail!("no tasks to run in {}", tasks_dir.display());
    }

    // Lane R1: a `replay` run never constructs a real provider at all (see
    // `crates/goose/src/providers/cassette.rs`), so a tier's API key is not
    // required for it. Demanding one would make the free replay slice
    // impossible on a machine with no credentials — which is exactly the
    // machine M0.5 ("replay slice green before any paid slice") exists to
    // serve. A `record` run DOES call the real provider and still needs its
    // key, so the bypass is deliberately narrow.
    let replaying = cassette_is_replaying(args.cassette.as_deref(), &args.cassette_mode);
    preflight_keys(
        &tiers,
        args.allow_missing_keys || replaying,
        args.use_keyring,
    )?;
    if !binary_resolves(&args.permagent_bin) {
        eprintln!(
            "warning: harness binary {:?} not found on PATH — runs will error. \
             Pass --permagent-bin <path> or build/install it first.",
            args.permagent_bin
        );
    }
    // Every task changes the child process cwd to its isolated workspace. A
    // caller-supplied relative path such as `./target/debug/permagent` would
    // therefore resolve inside that scratch directory and fail before the
    // first model/tool call. Resolve path-like binaries once from the
    // operator's cwd; bare command names intentionally continue to use PATH.
    let permagent_bin = resolved_binary(&args.permagent_bin);

    let runs_root = args
        .runs_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("permagent-eval-runs"));
    let arm_env = cassette_arm_env(args.cassette.as_deref(), &args.cassette_mode);
    let cfg = RunConfig {
        permagent_bin,
        runs_root,
        keep: args.keep,
        use_keyring: args.use_keyring,
        arm_env,
        ..Default::default()
    };
    let harness = SubprocessHarnessRunner;
    let oracle = SubprocessOracleRunner;
    let cost = LedgerCostReader;
    let recipe = SubprocessRecipeSource;
    let deps = Deps {
        harness: &harness,
        oracle: &oracle,
        cost: &cost,
        recipe: &recipe,
    };

    // Shared across every tier: measured spend accumulates for the WHOLE
    // session, not per tier, so `--budget-usd` caps the sweep as a whole.
    let mut budget = match args.budget_usd {
        Some(cap) => BudgetTracker::with_cap(cap),
        None => BudgetTracker::unlimited(),
    };

    let mut reports: Vec<TierReport> = Vec::new();
    for tier in &tiers {
        eprintln!(
            "==> tier {} ({} {}) — {} task(s)",
            tier.name,
            tier.provider,
            tier.model,
            tasks.len()
        );
        let results = run_tier(&tasks, tier, &cfg, &deps, &mut budget);
        for r in &results {
            eprintln!(
                "    {:<6} {:<22} {}",
                match r.oracle {
                    permagent_eval::oracle::OracleOutcome::NotRun => "SKIP",
                    _ if r.solved => "PASS",
                    _ => "FAIL",
                },
                r.task_id,
                permagent_eval::report::fmt_usd(r.cost.usd)
            );
        }
        reports.push(TierReport {
            tier: tier.name.clone(),
            provider: tier.provider.clone(),
            model: tier.model.clone(),
            pinned_packs: tier.pin_packs,
            results,
        });
        // Print the stop line exactly once, right after the tier in which the
        // cap tripped finishes (it, and every later tier, will have its
        // remaining tasks recorded as not-run above).
        if let Some(msg) = budget.take_stop_message() {
            eprintln!("{msg}");
        }
    }

    // D2 (`docs/orchestrator/CODING_HARNESS_ROLE_GRADUATION_DAG.md` G2): stamp
    // a run identity + timestamp onto the JSON report ONLY — `render_json`
    // itself (report.rs, shared by every format and by its own tests) stays
    // untouched; this is additive envelope metadata `permagent-eval
    // graduate` (D1) reads to avoid needing --provider/--model/
    // --snapshot-date flags on a fresh run.
    let generated_at = chrono::Utc::now();
    let run_id = format!("run-{}", generated_at.format("%Y%m%dT%H%M%SZ"));
    let rendered = match args.format.as_str() {
        "json" => {
            let value = stamp_json_envelope(render_json(&reports), &run_id, generated_at);
            let value = stamp_dag_selection_pct(value, overall_dag_selection_pct(&reports));
            serde_json::to_string_pretty(&value)?
        }
        "md" => render_markdown(&reports),
        _ => render_text(&reports),
    };
    println!("{rendered}");
    if let Some(path) = &args.out {
        std::fs::write(path, &rendered)
            .with_context(|| format!("writing report to {}", path.display()))?;
        eprintln!("report written to {}", path.display());

        // X-obs-1: every run that writes a report also writes the sibling
        // qualification record `permagent-eval qualify --record` reads.
        let qual_path = qualification_sibling_path(path);
        let records = build_qualification_records(&tasks_dir, &cfg.permagent_bin, &reports);
        write_qualification_records(&qual_path, &records)?;
        eprintln!("qualification record written to {}", qual_path.display());
    }

    // Gate the exit code on the pass-rate threshold, if one was given. The report
    // is already printed/written above, so a failure here still leaves the full
    // results for inspection.
    if let Some(threshold) = args.fail_under {
        if let Some((best_pct, want_pct)) = fail_under_violation(&reports, threshold)? {
            bail!("best tier pass-rate {best_pct:.1}% is below --fail-under {want_pct:.1}%");
        }
    }
    Ok(())
}

fn cmd_knobs() -> Result<()> {
    println!("Harness knobs that can be A/B'd from the environment:\n");
    for k in ab::KNOBS {
        println!("  {}", k.name);
        println!("    {}", k.summary);
        println!("    shipped default: {}", k.shipped_default);
        println!(
            "    A: {}   B: {}",
            render_env_pairs(k.off),
            render_env_pairs(k.on)
        );
        println!();
    }
    println!(
        "Anything not listed can still be compared with `ab --a-env KEY=VALUE --b-env KEY=VALUE`.\n\
         Note: keys read through the config layer parse their value as JSON, so a boolean must be\n\
         spelled `true`/`false` — `1` silently falls back to the default and both arms become the\n\
         same configuration."
    );
    Ok(())
}

fn render_env_pairs(kv: &[(&str, &str)]) -> String {
    kv.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve the two arms from either a registry knob or the ad-hoc
/// `--a-env`/`--b-env` pairs. Pure over the parsed args so it is unit-tested.
fn resolve_arms(args: &AbArgs) -> Result<(ArmConfig, ArmConfig, Option<&'static Knob>)> {
    if let Some(name) = &args.knob {
        let k = ab::knob(name).with_context(|| {
            format!(
                "unknown knob {name:?}; known knobs are: {}",
                ab::knob_names().join(", ")
            )
        })?;
        let mut a = k.control_arm();
        let mut b = k.treatment_arm();
        a.label = format!("{} ({})", args.baseline, a.label);
        b.label = format!("{} ({})", args.candidate, b.label);
        return Ok((a, b, Some(k)));
    }
    if args.a_env.is_empty() && args.b_env.is_empty() {
        bail!(
            "nothing to compare: pass --knob <name> (see `permagent-eval knobs`) or \
             --a-env/--b-env KEY=VALUE"
        );
    }
    let mut a = ArmConfig::new(args.baseline.clone());
    for kv in &args.a_env {
        a = a.with_env_kv(kv)?;
    }
    let mut b = ArmConfig::new(args.candidate.clone());
    for kv in &args.b_env {
        b = b.with_env_kv(kv)?;
    }
    if a.env == b.env {
        bail!(
            "arm A and arm B have identical environments — this would compare a configuration \
             against itself and report a delta that is pure noise"
        );
    }
    // A variable set on only ONE side is a trap: the other arm silently
    // inherits whatever the operator's shell (or the harness default) happens
    // to be, so the comparison is not controlled.
    let a_keys: Vec<&str> = a.env.iter().map(|(k, _)| k.as_str()).collect();
    let b_keys: Vec<&str> = b.env.iter().map(|(k, _)| k.as_str()).collect();
    for k in a_keys.iter().chain(b_keys.iter()) {
        if !a_keys.contains(k) || !b_keys.contains(k) {
            eprintln!(
                "warning: {k} is set on only one arm; the other arm inherits the ambient \
                 default, so the comparison is not fully controlled. Pin it on both sides."
            );
        }
    }
    Ok((a, b, None))
}

// --- X-obs-1: the qualification record every `run`/`ab` invocation writes
// alongside its own `--out` -------------------------------------------------

/// The sibling qualification-record path for a report/result file: `<out>` +
/// `.qual.json` appended (never a replaced extension), so `report.json`
/// becomes `report.json.qual.json` and `report.md` becomes
/// `report.md.qual.json`.
fn qualification_sibling_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(".qual.json");
    PathBuf::from(name)
}

/// The git hash of the most recent commit that touched `tasks_dir` — the
/// "fixture revision" a qualification record pins task evidence to. `None`
/// when `tasks_dir` is not inside a git repository, `git` is not on `PATH`,
/// or the command otherwise fails (a fresh checkout with no commits yet,
/// for instance). Never a fabricated placeholder: an absent revision must
/// read as missing evidence, not as "revision zero".
fn fixture_revision(tasks_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(tasks_dir)
        .args(["log", "-1", "--format=%H", "--", "."])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// The harness binary's own `--version` output, trimmed. `None` when the
/// binary cannot be run or prints nothing — never an empty-string default
/// that could be misread as "no version" rather than "unknown".
fn detect_binary_version(bin_path: &str) -> Option<String> {
    let output = std::process::Command::new(bin_path)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The first git-sha-shaped (40 hex characters) token found via `strings` on
/// the binary — a best-effort provenance signal for a binary built without
/// `--version` embedding one. `None` when `strings` is unavailable, the
/// binary cannot be read, or no such token appears.
fn detect_binary_git_sha(bin_path: &str) -> Option<String> {
    static SHA_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\b[0-9a-f]{40}\b").expect("valid regex"));
    let output = std::process::Command::new("strings")
        .arg(bin_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    SHA_RE.find(&text).map(|m| m.as_str().to_string())
}

/// A short, unique id for one `run`/`ab` invocation's retained qualification
/// record(s), so re-running the same tier/arm never collides in a directory
/// that accumulates records over time.
fn qualification_run_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Build one [`RunQualificationRecord`] per tier report from a `run`
/// invocation, sharing the one `fixture_revision`/binary provenance lookup
/// across every tier (they all ran the same fixtures against the same
/// binary).
fn build_qualification_records(
    tasks_dir: &Path,
    bin_path: &str,
    reports: &[TierReport],
) -> Vec<RunQualificationRecord> {
    let fixture_revision = fixture_revision(tasks_dir);
    let binary = BinaryProvenance {
        path: bin_path.to_string(),
        version: detect_binary_version(bin_path),
        git_sha: detect_binary_git_sha(bin_path),
    };
    let stamp = qualification_run_stamp();
    reports
        .iter()
        .map(|tr| {
            build_run_qualification_record(
                format!("run-{stamp}-{}", tr.tier),
                fixture_revision.clone(),
                binary.clone(),
                tr.provider.clone(),
                tr.model.clone(),
                tr.pinned_packs,
                &tr.results,
            )
        })
        .collect()
}

/// Write one or more qualification records as a JSON array to `path`,
/// creating its parent directory if needed (mirrors [`write_ab_result`]).
fn write_qualification_records(path: &Path, records: &[RunQualificationRecord]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(records)?;
    std::fs::write(path, json)
        .with_context(|| format!("writing qualification record to {}", path.display()))
}

/// Default result-file path: stable prefix, unique suffix, under the runs dir.
fn default_ab_out(runs_root: &Path, tier: &str, knob: Option<&Knob>) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let knob = knob.map(|k| k.name).unwrap_or("adhoc");
    runs_root.join(format!("ab-{tier}-{knob}-{stamp}.json"))
}

/// `ab` holds the model constant by construction. Two tiers would make the
/// delta a model comparison wearing a harness comparison's clothes.
fn single_tier(tiers: &[Tier]) -> Result<&Tier> {
    match tiers {
        [one] => Ok(one),
        many => bail!(
            "`ab` compares two CONFIGURATIONS at one model tier, but {} tiers were selected \
             ({}). A delta measured across two models measures the models, not the harness — \
             pass exactly one --tier (or one --provider/--model pair).",
            many.len(),
            many.iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `ab`'s runs directory, defaulting the same way `run`'s does.
fn ab_runs_root(args: &AbArgs) -> PathBuf {
    args.runs_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("permagent-eval-runs"))
}

/// The base [`RunConfig`] for an `ab` arm — no arm applied yet. Mirrors
/// `cmd_run`'s own construction, including the relative-`--permagent-bin`
/// resolution fix (a caller-supplied relative path must be resolved against
/// the operator's cwd, since every task changes the child's cwd to its
/// isolated workspace).
fn ab_run_config(args: &AbArgs) -> RunConfig {
    RunConfig {
        permagent_bin: resolved_binary(&args.permagent_bin),
        runs_root: ab_runs_root(args),
        keep: args.keep,
        use_keyring: args.use_keyring,
        ..Default::default()
    }
}

fn cmd_ab(args: AbArgs) -> Result<()> {
    let tiers = args.tiers.resolve()?;
    let tier = single_tier(&tiers)?;
    if args.repeats == 0 {
        bail!("--repeats must be at least 1");
    }

    let tasks_dir = resolve_tasks_dir(&args.tasks_dir);
    let tasks: Vec<Task> = select_tasks(load_task_set(&tasks_dir)?, &args.tasks)?;
    if tasks.is_empty() {
        bail!("no tasks to run in {}", tasks_dir.display());
    }
    let (arm_a, arm_b, knob) = resolve_arms(&args)?;

    let runs_root = ab_runs_root(&args);
    let out_path = args
        .out
        .clone()
        .unwrap_or_else(|| default_ab_out(&runs_root, &tier.name, knob));

    // Say what this will cost in invocations BEFORE anything is launched — an
    // A/B is 2×N×fixtures runs, which is easy to under-estimate by a factor of
    // ten when reaching for `--repeats`.
    let invocations = 2 * args.repeats * tasks.len();
    eprintln!(
        "==> ab @ tier {} ({} {}) — {} fixture(s) × {} repeat(s) × 2 arms = {} harness invocation(s)",
        tier.name,
        tier.provider,
        tier.model,
        tasks.len(),
        args.repeats,
        invocations
    );
    eprintln!("    A: {}  [{}]", arm_a.label, render_arm_env(&arm_a));
    eprintln!("    B: {}  [{}]", arm_b.label, render_arm_env(&arm_b));
    let requirement = permagent_eval::ab::SampleRequirement::compute(
        tasks.len(),
        args.repeats,
        args.repeats * tasks.len(),
    );
    if !requirement.is_met() {
        eprintln!(
            "    NOTE: this sample cannot produce a verdict — {}",
            requirement.what_it_would_take()
        );
    }
    if args.dry_run {
        eprintln!("--dry-run: nothing was executed and nothing was spent.");
        eprintln!("result file would be: {}", out_path.display());
        return Ok(());
    }

    preflight_keys(&tiers, args.allow_missing_keys, args.use_keyring)?;
    if !binary_resolves(&args.permagent_bin) {
        eprintln!(
            "warning: harness binary {:?} not found on PATH — runs will error. \
             Pass --permagent-bin <path> or build/install it first.",
            args.permagent_bin
        );
    }

    let harness = SubprocessHarnessRunner;
    let oracle = SubprocessOracleRunner;
    let cost = LedgerCostReader;
    let recipe = SubprocessRecipeSource;
    let deps = Deps {
        harness: &harness,
        oracle: &oracle,
        cost: &cost,
        recipe: &recipe,
    };
    // One tracker across BOTH arms: `--budget-usd` caps the comparison, not
    // each side of it. A cap that stopped arm B alone would silently produce a
    // lopsided sample, which the honesty guard would then have to catch.
    let mut budget = match args.budget_usd {
        Some(cap) => BudgetTracker::with_cap(cap),
        None => BudgetTracker::unlimited(),
    };

    let mut a_results: Vec<TaskResult> = Vec::new();
    let mut b_results: Vec<TaskResult> = Vec::new();
    // Interleave the arms by repeat rather than running A to completion first:
    // provider latency, rate limits and model-side drift are time-varying, and
    // an A-then-B ordering would charge all of that to arm B.
    for rep in 1..=args.repeats {
        for (label, arm, sink) in [
            (&arm_a.label, &arm_a, &mut a_results),
            (&arm_b.label, &arm_b, &mut b_results),
        ] {
            eprintln!("--> repeat {}/{} · arm {}", rep, args.repeats, label);
            let mut cfg = ab_run_config(&args);
            cfg.arm_env = arm.env.clone();
            cfg.arm_unset = arm.unset.clone();
            let results = run_tier(&tasks, tier, &cfg, &deps, &mut budget);
            for r in &results {
                eprintln!(
                    "    {:<6} {:<22} {}",
                    match r.oracle {
                        permagent_eval::oracle::OracleOutcome::NotRun => "SKIP",
                        _ if r.solved => "PASS",
                        _ => "FAIL",
                    },
                    r.task_id,
                    permagent_eval::report::fmt_usd(r.cost.usd)
                );
            }
            sink.extend(results);
        }
        if let Some(msg) = budget.take_stop_message() {
            eprintln!("{msg}");
        }
    }

    let report = AbReport::build(
        tier,
        knob,
        args.repeats,
        tasks.len(),
        &arm_a,
        &a_results,
        &arm_b,
        &b_results,
    );
    write_ab_result(&out_path, &report)?;
    println!("{}", ab::render_markdown(&report));
    println!("result file: {}", out_path.display());

    // X-obs-1: one qualification record per arm, sibling to the result file.
    let qual_path = qualification_sibling_path(&out_path);
    let bin_path = resolved_binary(&args.permagent_bin);
    let fixture_revision = fixture_revision(&tasks_dir);
    let binary = BinaryProvenance {
        path: bin_path.clone(),
        version: detect_binary_version(&bin_path),
        git_sha: detect_binary_git_sha(&bin_path),
    };
    let stamp = qualification_run_stamp();
    let records = vec![
        build_run_qualification_record(
            format!("ab-{stamp}-{}", arm_a.label),
            fixture_revision.clone(),
            binary.clone(),
            tier.provider.clone(),
            tier.model.clone(),
            tier.pin_packs,
            &a_results,
        ),
        build_run_qualification_record(
            format!("ab-{stamp}-{}", arm_b.label),
            fixture_revision,
            binary,
            tier.provider.clone(),
            tier.model.clone(),
            tier.pin_packs,
            &b_results,
        ),
    ];
    write_qualification_records(&qual_path, &records)?;
    println!("qualification record: {}", qual_path.display());

    if args.require_conclusive && !report.verdict.is_conclusive() {
        bail!(
            "--require-conclusive: the run earned no verdict. {}",
            report.requirement.what_it_would_take()
        );
    }
    Ok(())
}

fn render_arm_env(arm: &ArmConfig) -> String {
    if arm.is_empty() {
        return "no overrides".to_string();
    }
    let mut parts: Vec<String> = arm.unset.iter().map(|n| format!("-{n}")).collect();
    parts.extend(arm.env.iter().map(|(k, v)| format!("{k}={v}")));
    parts.join(" ")
}

fn write_ab_result(path: &Path, report: &AbReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, json).with_context(|| format!("writing result to {}", path.display()))
}

/// Pick the requested scenarios, erroring on an id that does not exist rather
/// than silently running a smaller suite than the operator asked for.
fn select_scenarios(wanted: &[String]) -> Result<Vec<tier0::Tier0Scenario>> {
    let all = tier0::scenarios();
    if wanted.is_empty() {
        return Ok(all);
    }
    let known: Vec<&str> = all.iter().map(|s| s.id).collect();
    let mut out = Vec::new();
    for id in wanted {
        let s = all.iter().find(|s| s.id == id).with_context(|| {
            format!(
                "unknown tier-0 scenario {id:?}; known scenarios are: {}",
                known.join(", ")
            )
        })?;
        out.push(s.clone());
    }
    Ok(out)
}

fn cmd_tier0(args: Tier0Args) -> Result<()> {
    let selected = select_scenarios(&args.scenarios)?;
    if args.list {
        println!("Tier 0 scenarios ($0 — a scripted model on loopback, no provider contacted):\n");
        for s in &selected {
            println!("  {}", s.id);
            println!("    {}", s.measures);
            if !s.env.is_empty() {
                println!("    env: {}", render_env_pairs(s.env));
            }
            println!();
        }
        return Ok(());
    }

    let work_root = args
        .work_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("permagent-eval-tier0"));
    if !binary_resolves(&args.permagent_bin) {
        eprintln!(
            "warning: harness binary {:?} not found on PATH — every scenario will error. \
             Pass --permagent-bin <path> or build it first.",
            args.permagent_bin
        );
    }
    eprintln!(
        "==> tier0 — {} scenario(s) against a scripted stub model; nothing is spent",
        selected.len()
    );

    let report = tier0::run_all(&selected, &args.permagent_bin, &work_root)?;
    println!("{}", tier0::render_markdown(&report));
    if let Some(path) = &args.out {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("result file: {}", path.display());
    }
    eprintln!("scratch kept under {}", work_root.display());

    if args.fail_on_regression && !report.all_passed() {
        let failed: Vec<&str> = report
            .scenarios
            .iter()
            .filter(|s| !s.passed)
            .map(|s| s.id.as_str())
            .collect();
        bail!("tier-0 scenario(s) failed: {}", failed.join(", "));
    }
    Ok(())
}

fn cmd_report(args: ReportArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.file)
        .with_context(|| format!("reading {}", args.file.display()))?;
    let report: AbReport = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is not a `{}` result file",
            args.file.display(),
            permagent_eval::ab::AB_SCHEMA
        )
    })?;
    if report.schema != permagent_eval::ab::AB_SCHEMA {
        bail!(
            "{} declares schema {:?}, but this build reads {:?}",
            args.file.display(),
            report.schema,
            permagent_eval::ab::AB_SCHEMA
        );
    }
    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        _ => println!("{}", ab::render_markdown(&report)),
    }
    Ok(())
}

/// Decide whether a `--fail-under PERCENT` threshold is violated. Returns
/// `Ok(Some((best_pct, threshold_pct)))` when the best tier's pass-rate is below
/// the threshold (the caller should fail), `Ok(None)` when the bar is met, and
/// `Err` when the threshold is out of the `[0, 100]` range. Pure over the
/// reports so it is unit-tested without running anything.
fn fail_under_violation(reports: &[TierReport], threshold_pct: f64) -> Result<Option<(f64, f64)>> {
    if !(0.0..=100.0).contains(&threshold_pct) {
        bail!("--fail-under must be a percentage in [0, 100], got {threshold_pct}");
    }
    let best_pct = reports
        .iter()
        .map(|r| r.aggregate().pass_rate * 100.0)
        .fold(0.0_f64, f64::max);
    if best_pct + f64::EPSILON < threshold_pct {
        Ok(Some((best_pct, threshold_pct)))
    } else {
        Ok(None)
    }
}

/// Error (unless allowed) when a selected tier needs an API key that is not set.
/// A set-but-empty (or whitespace-only) value counts as missing — it would only
/// send an empty credential and fail deeper in the harness.
fn preflight_keys(tiers: &[Tier], allow_missing: bool, use_keyring: bool) -> Result<()> {
    // With `--use-keyring` the CHILD reads secrets from the OS keychain, so this
    // process's environment is not where the credential lives and its absence
    // proves nothing. Erroring on it forced `--allow-missing-keys` on every
    // keychain-only provider, which then also silenced the case this check
    // exists for: a genuinely absent key, which otherwise fails 68 times in a
    // row instead of once up front.
    preflight_keys_with(tiers, allow_missing || use_keyring, |k| {
        std::env::var(k).ok()
    })
}

/// Testable core of [`preflight_keys`]: `lookup` supplies each env var's value so
/// the empty-key policy can be exercised without mutating the process
/// environment.
fn preflight_keys_with(
    tiers: &[Tier],
    allow_missing: bool,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    for tier in tiers {
        let Some(key) = &tier.required_key_env else {
            continue;
        };
        let present = lookup(key).is_some_and(|v| !v.trim().is_empty());
        if present {
            continue;
        }
        if allow_missing {
            eprintln!(
                "note: tier {} declares {}, which is not in this process's environment. \
                 The child reads the OS keychain, so this is expected for a keychain-only \
                 provider — but if the credential is missing there too, every run will fail.",
                tier.name, key
            );
        } else {
            bail!(
                "tier {} needs environment variable {} (set it, or pass --allow-missing-keys)",
                tier.name,
                key
            );
        }
    }
    Ok(())
}

/// Best-effort check that the harness binary resolves (path exists, or a bare
/// name is found on PATH) — a no-spawn preflight.
fn binary_resolves(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.components().count() > 1 || p.is_absolute() {
        return p.exists();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(bin).exists())
}

fn resolved_binary(bin: &str) -> String {
    let path = Path::new(bin);
    if path.is_absolute() || path.components().count() <= 1 {
        return bin.to_string();
    }
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent_eval::cost::CostReading;
    use permagent_eval::oracle::OracleOutcome;

    #[test]
    fn relative_binary_paths_are_resolved_before_scratch_cwd_changes() {
        let cwd = std::env::current_dir().unwrap();
        let resolved = resolved_binary("./Cargo.toml");
        assert_eq!(Path::new(&resolved), cwd.join("Cargo.toml"));
        assert_eq!(resolved_binary("permagent"), "permagent");
    }

    fn tier_report(name: &str, solved: usize, total: usize) -> TierReport {
        let results = (0..total)
            .map(|i| {
                let o = if i < solved {
                    OracleOutcome::Pass
                } else {
                    OracleOutcome::Fail
                };
                TaskResult::new(
                    format!("t{i}"),
                    "classic",
                    o,
                    CostReading::known(0.0, false, 1),
                )
            })
            .collect();
        TierReport {
            tier: name.to_string(),
            provider: "p".to_string(),
            model: "m".to_string(),
            pinned_packs: true,
            results,
        }
    }

    /// Regression: `--use-keyring` means the CHILD reads the OS keychain, so this
    /// process's environment is not where the credential lives. Erroring on its
    /// absence forced `--allow-missing-keys` onto every keychain-only provider,
    /// which then also silenced the genuinely-missing case this check exists for
    /// — turning one clear failure up front into 68 identical failures.
    #[test]
    fn preflight_accepts_a_keychain_only_provider_but_still_catches_a_truly_absent_key() {
        let tiers = vec![Tier::builtin("dschat").expect("dschat is a built-in tier")];
        let absent = |_: &str| None;

        // Without the keyring the absence is decisive: fail once, up front.
        assert!(preflight_keys_with(&tiers, false, absent).is_err());

        // The keyring makes the environment non-authoritative, so this proceeds
        // and the child resolves the credential itself.
        assert!(preflight_keys_with(&tiers, true, absent).is_ok());
    }

    // --- Nit 1: preflight rejects a set-but-empty key -----------------------

    #[test]
    fn preflight_treats_empty_key_as_missing() {
        let tiers = vec![Tier::builtin("frontier").unwrap()]; // needs ANTHROPIC_API_KEY
                                                              // Set-but-empty must be rejected (the bug: `var(key).is_ok()` accepted it).
        assert!(preflight_keys_with(&tiers, false, |_| Some(String::new())).is_err());
        // Whitespace-only likewise.
        assert!(preflight_keys_with(&tiers, false, |_| Some("   ".to_string())).is_err());
        // A real value passes.
        assert!(preflight_keys_with(&tiers, false, |_| Some("sk-abc".to_string())).is_ok());
        // Absent + allow_missing => a warning, not an error.
        assert!(preflight_keys_with(&tiers, true, |_| None).is_ok());
    }

    #[test]
    fn preflight_skips_keyless_tiers() {
        let tiers = vec![Tier::builtin("local").unwrap()]; // no required key
        assert!(preflight_keys_with(&tiers, false, |_| None).is_ok());
    }

    // --- Nit 2: --fail-under gates the exit code ----------------------------

    #[test]
    fn fail_under_uses_the_best_tier_and_respects_the_threshold() {
        let reports = vec![tier_report("local", 2, 4), tier_report("frontier", 3, 4)];
        // Best is 75%. A bar of 80% is violated…
        let v = fail_under_violation(&reports, 80.0).unwrap().unwrap();
        assert!((v.0 - 75.0).abs() < 1e-9 && (v.1 - 80.0).abs() < 1e-9);
        // …a bar of 75% is met exactly (no violation)…
        assert!(fail_under_violation(&reports, 75.0).unwrap().is_none());
        // …and a bar of 50% is comfortably met.
        assert!(fail_under_violation(&reports, 50.0).unwrap().is_none());
    }

    #[test]
    fn fail_under_rejects_out_of_range_thresholds() {
        let reports = vec![tier_report("local", 1, 1)];
        assert!(fail_under_violation(&reports, -1.0).is_err());
        assert!(fail_under_violation(&reports, 101.0).is_err());
        assert!(fail_under_violation(&reports, 100.0).unwrap().is_none());
    }

    // --- Nit 3: duplicate --tier is de-duplicated ---------------------------

    #[test]
    fn resolve_dedupes_repeated_tiers_preserving_order() {
        let sel = TierSelection {
            tiers: vec![
                "frontier".to_string(),
                "local".to_string(),
                "frontier".to_string(),
            ],
            provider: None,
            model: None,
            native_routing: false,
        };
        let resolved = sel.resolve().unwrap();
        let names: Vec<&str> = resolved.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["frontier", "local"]);
    }

    #[test]
    fn resolve_applies_native_routing_to_all_tiers() {
        let sel = TierSelection {
            tiers: vec!["local".to_string(), "frontier".to_string()],
            provider: None,
            model: None,
            native_routing: true,
        };
        let resolved = sel.resolve().unwrap();
        assert!(resolved.iter().all(|t| !t.pin_packs));
    }

    // --- --use-keyring / --budget-usd parse and default correctly ----------

    #[test]
    fn use_keyring_and_budget_usd_default_off() {
        let cli = Cli::try_parse_from(["permagent-eval", "run", "--tier", "local"]).unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert!(!args.use_keyring);
        assert_eq!(args.budget_usd, None);
    }

    #[test]
    fn use_keyring_and_budget_usd_parse_on_run() {
        let cli = Cli::try_parse_from([
            "permagent-eval",
            "run",
            "--tier",
            "local",
            "--use-keyring",
            "--budget-usd",
            "12.50",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert!(args.use_keyring);
        assert_eq!(args.budget_usd, Some(12.50));
    }

    // --- Lane R1: `--cassette`/`--cassette-mode` parse, default, and wire
    // into `RunConfig.arm_env` exactly like an A/B arm knob -------------

    #[test]
    fn cassette_defaults_off_and_replay_mode() {
        let cli = Cli::try_parse_from(["permagent-eval", "run", "--tier", "local"]).unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(args.cassette, None);
        assert_eq!(args.cassette_mode, "replay");
    }

    #[test]
    fn cassette_flag_parses_dir_and_mode() {
        let cli = Cli::try_parse_from([
            "permagent-eval",
            "run",
            "--tier",
            "local",
            "--cassette",
            "/tmp/my-cassette",
            "--cassette-mode",
            "record",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected Run");
        };
        assert_eq!(args.cassette, Some(PathBuf::from("/tmp/my-cassette")));
        assert_eq!(args.cassette_mode, "record");
    }

    #[test]
    fn cassette_mode_rejects_unknown_value() {
        let result = Cli::try_parse_from([
            "permagent-eval",
            "run",
            "--tier",
            "local",
            "--cassette",
            "/tmp/x",
            "--cassette-mode",
            "playback",
        ]);
        assert!(
            result.is_err(),
            "an unknown cassette mode must be rejected at parse time"
        );
    }

    #[test]
    fn cassette_arg_becomes_the_provider_cassette_env_var() {
        // Pins the exact seam `cmd_run` uses -- `RunConfig.arm_env`, applied
        // LAST over both the invocation's own defaults and the inherited
        // environment (runner.rs: "The arm's knobs go on LAST so they
        // outrank the invocation's own defaults... as well as the inherited
        // environment"). This is the same seam an A/B arm's `--a-env`/
        // `--b-env` already rides, so `--cassette` needs no new plumbing in
        // `runner.rs`/`invocation.rs`.
        assert_eq!(
            cassette_arm_env(Some(Path::new("/tmp/fx03-cassette")), "replay"),
            vec![(
                "PERMAGENT_PROVIDER_CASSETTE".to_string(),
                "replay:/tmp/fx03-cassette".to_string()
            )]
        );
        assert_eq!(
            cassette_arm_env(Some(Path::new("/tmp/fx03-cassette")), "record"),
            vec![(
                "PERMAGENT_PROVIDER_CASSETTE".to_string(),
                "record:/tmp/fx03-cassette".to_string()
            )]
        );
    }

    /// No `--cassette` must leave the arm environment EMPTY, so an ordinary
    /// run is byte-for-byte what it was before this lane — the same "unset
    /// means never reached" discipline the provider side keeps.
    #[test]
    fn no_cassette_arg_leaves_the_arm_environment_untouched() {
        assert!(cassette_arm_env(None, "replay").is_empty());
        assert!(cassette_arm_env(None, "record").is_empty());
    }

    /// Only `replay` may skip the API-key preflight: a `record` run really
    /// does call the paid provider and must still be refused without a key.
    #[test]
    fn only_replay_waives_the_api_key_preflight() {
        let dir = PathBuf::from("/tmp/fx03-cassette");
        assert!(cassette_is_replaying(Some(dir.as_path()), "replay"));
        assert!(!cassette_is_replaying(Some(dir.as_path()), "record"));
        assert!(!cassette_is_replaying(None, "replay"));
    }

    // --- `ab`: arg parsing, arm resolution, honesty guard, reporting -------

    fn ab_args(argv: &[&str]) -> AbArgs {
        let mut full = vec!["permagent-eval", "ab"];
        full.extend_from_slice(argv);
        let cli = Cli::try_parse_from(full).expect("args should parse");
        let Command::Ab(args) = cli.command else {
            panic!("expected Ab");
        };
        args
    }

    #[test]
    fn ab_parses_a_knob_run_with_repeats_and_defaults_to_one_repeat() {
        let a = ab_args(&["--tier", "haiku", "--knob", "repo-map", "--repeats", "4"]);
        assert_eq!(a.knob.as_deref(), Some("repo-map"));
        assert_eq!(a.repeats, 4);
        assert!(!a.dry_run);
        assert!(!a.require_conclusive);
        assert_eq!(a.baseline, "baseline");
        assert_eq!(a.candidate, "candidate");
        assert_eq!(
            ab_args(&["--tier", "haiku", "--knob", "repo-map"]).repeats,
            1
        );
    }

    #[test]
    fn ab_refuses_a_knob_mixed_with_ad_hoc_env() {
        // clap-level conflict: the two ways of defining an arm are exclusive,
        // so a run can never half-apply both.
        assert!(Cli::try_parse_from([
            "permagent-eval",
            "ab",
            "--tier",
            "haiku",
            "--knob",
            "repo-map",
            "--a-env",
            "X=1",
        ])
        .is_err());
    }

    #[test]
    fn ab_holds_the_model_constant_by_refusing_more_than_one_tier() {
        let tiers = vec![
            Tier::builtin("haiku").unwrap(),
            Tier::builtin("sonnet").unwrap(),
        ];
        let err = single_tier(&tiers).unwrap_err().to_string();
        assert!(err.contains("measures the models"), "{err}");
        assert!(single_tier(&tiers[..1]).is_ok());
    }

    #[test]
    fn resolve_arms_expands_a_registry_knob_into_two_labelled_arms() {
        let args = ab_args(&["--tier", "haiku", "--knob", "premature-done-guard"]);
        let (a, b, knob) = resolve_arms(&args).unwrap();
        assert_eq!(knob.unwrap().name, "premature-done-guard");
        assert_eq!(
            a.env,
            vec![("PERMAGENT_AFTER_TURN_HOLD".into(), "0".into())]
        );
        assert_eq!(
            b.env,
            vec![("PERMAGENT_AFTER_TURN_HOLD".into(), "1".into())]
        );
        assert!(a.label.contains("baseline") && a.label.contains("off"));
        assert!(b.label.contains("candidate") && b.label.contains("on"));
    }

    #[test]
    fn resolve_arms_rejects_an_unknown_knob_and_names_the_real_ones() {
        let args = ab_args(&["--tier", "haiku", "--knob", "turbo-mode"]);
        let err = format!("{:#}", resolve_arms(&args).unwrap_err());
        assert!(err.contains("unknown knob"), "{err}");
        assert!(
            err.contains("repo-map"),
            "the error must list what IS valid: {err}"
        );
    }

    #[test]
    fn resolve_arms_refuses_to_compare_a_configuration_against_itself() {
        let args = ab_args(&["--tier", "haiku", "--a-env", "X=1", "--b-env", "X=1"]);
        let err = format!("{:#}", resolve_arms(&args).unwrap_err());
        assert!(err.contains("against itself"), "{err}");
    }

    #[test]
    fn resolve_arms_requires_something_to_compare() {
        let args = ab_args(&["--tier", "haiku"]);
        let err = format!("{:#}", resolve_arms(&args).unwrap_err());
        assert!(err.contains("nothing to compare"), "{err}");
    }

    #[test]
    fn resolve_arms_accepts_ad_hoc_env_pairs_and_rejects_malformed_ones() {
        let args = ab_args(&[
            "--tier",
            "haiku",
            "--a-env",
            "GOOSE_TEMPERATURE=0",
            "--b-env",
            "GOOSE_TEMPERATURE=1",
        ]);
        let (a, b, knob) = resolve_arms(&args).unwrap();
        assert!(knob.is_none());
        assert_eq!(a.env[0].1, "0");
        assert_eq!(b.env[0].1, "1");

        let bad = ab_args(&["--tier", "haiku", "--a-env", "NOEQUALS"]);
        assert!(resolve_arms(&bad).is_err());
    }

    /// The whole point of the lane: a saved result renders without the run
    /// that produced it, and a file that is not one is refused rather than
    /// half-rendered.
    #[test]
    fn report_renders_a_saved_result_and_rejects_a_foreign_file() {
        let tmp = tempfile::tempdir().unwrap();
        let tier = Tier::builtin("haiku").unwrap();
        let results: Vec<TaskResult> = (0..3)
            .map(|i| {
                TaskResult::new(
                    format!("t{i}"),
                    "classic",
                    OracleOutcome::Pass,
                    CostReading::known(0.01, false, 1),
                )
            })
            .collect();
        let arm_a = ArmConfig::new("a");
        let arm_b = ArmConfig::new("b");
        let report = AbReport::build(&tier, None, 1, 3, &arm_a, &results, &arm_b, &results);

        let path = tmp.path().join("nested").join("ab.json");
        write_ab_result(&path, &report).unwrap();
        assert!(path.exists(), "the parent directory must be created");
        cmd_report(ReportArgs {
            file: path.clone(),
            format: "md".to_string(),
        })
        .unwrap();

        // A file that parses as JSON but is not an AB result is refused.
        let foreign = tmp.path().join("other.json");
        std::fs::write(&foreign, "{\"tiers\":[]}").unwrap();
        let err = format!(
            "{:#}",
            cmd_report(ReportArgs {
                file: foreign,
                format: "md".to_string(),
            })
            .unwrap_err()
        );
        assert!(err.contains("result file"), "{err}");
    }

    /// A result file whose schema string is from another version must be
    /// refused outright rather than rendered with fields silently missing.
    #[test]
    fn report_refuses_a_result_file_from_another_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let tier = Tier::builtin("haiku").unwrap();
        let mut report = AbReport::build(
            &tier,
            None,
            1,
            0,
            &ArmConfig::new("a"),
            &[],
            &ArmConfig::new("b"),
            &[],
        );
        report.schema = "permagent-eval/ab-result@99".to_string();
        let path = tmp.path().join("future.json");
        write_ab_result(&path, &report).unwrap();
        let err = format!(
            "{:#}",
            cmd_report(ReportArgs {
                file: path,
                format: "md".to_string(),
            })
            .unwrap_err()
        );
        assert!(err.contains("@99"), "{err}");
    }

    #[test]
    fn knobs_listing_renders_every_registry_entry() {
        cmd_knobs().unwrap();
        for k in permagent_eval::ab::KNOBS {
            assert!(!render_env_pairs(k.off).is_empty(), "{}", k.name);
            assert!(!render_env_pairs(k.on).is_empty(), "{}", k.name);
        }
    }

    #[test]
    fn the_default_result_path_is_unique_per_tier_and_knob() {
        let root = Path::new("/tmp/runs");
        let p = default_ab_out(root, "haiku", permagent_eval::ab::knob("repo-map"));
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("ab-haiku-repo-map-"), "{name}");
        assert!(name.ends_with(".json"), "{name}");
        let adhoc = default_ab_out(root, "haiku", None);
        assert!(
            adhoc.to_string_lossy().contains("-adhoc-"),
            "{}",
            adhoc.display()
        );
    }

    // --- `tier0` -----------------------------------------------------------

    #[test]
    fn tier0_selects_every_scenario_by_default_and_names_unknown_ids() {
        assert_eq!(
            select_scenarios(&[]).unwrap().len(),
            tier0::scenarios().len()
        );
        let one = select_scenarios(&["tool-result-pruning".to_string()]).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "tool-result-pruning");

        let err = format!(
            "{:#}",
            select_scenarios(&["not-a-scenario".to_string()]).unwrap_err()
        );
        assert!(err.contains("unknown tier-0 scenario"), "{err}");
        assert!(
            err.contains("tool-result-pruning"),
            "the error must list what IS valid: {err}"
        );
    }

    #[test]
    fn tier0_args_default_to_the_permagent_binary_and_no_ci_gate() {
        let cli = Cli::try_parse_from(["permagent-eval", "tier0"]).unwrap();
        let Command::Tier0(args) = cli.command else {
            panic!("expected Tier0");
        };
        assert_eq!(args.permagent_bin, "permagent");
        assert!(args.scenarios.is_empty());
        assert!(!args.fail_on_regression);
        assert!(!args.list);
    }

    /// `--list` must be side-effect free: it is the one Tier-0 path that runs
    /// with no binary present, so it must not try to spawn anything.
    #[test]
    fn tier0_list_runs_without_a_harness_binary() {
        cmd_tier0(Tier0Args {
            permagent_bin: "/nonexistent/permagent".to_string(),
            scenarios: vec![],
            work_dir: None,
            out: None,
            list: true,
            fail_on_regression: true,
        })
        .unwrap();
    }

    #[test]
    fn qualify_subcommand_requires_input_and_accepts_output() {
        let cli = Cli::try_parse_from([
            "permagent-eval",
            "qualify",
            "--input",
            "evidence.json",
            "--out",
            "scorecard.json",
        ])
        .unwrap();
        let Command::Qualify(args) = cli.command else {
            panic!("expected Qualify");
        };
        assert_eq!(args.input, Some(PathBuf::from("evidence.json")));
        assert_eq!(args.out, Some(PathBuf::from("scorecard.json")));
        assert!(args.records.is_empty());
    }

    /// The default `QualifyArgs` this test module reuses for both the
    /// `--input` and `--record` paths — filling in the new X-obs-1 fields
    /// with their empty/default values so each test only sets what it cares
    /// about.
    fn qualify_args(out: Option<PathBuf>) -> QualifyArgs {
        QualifyArgs {
            input: None,
            records: Vec::new(),
            optimizer_tasks: Vec::new(),
            heldout_tasks: Vec::new(),
            benchmark_version: "eval-records-v1".to_string(),
            out,
        }
    }

    #[test]
    fn qualify_command_ingests_retained_json_without_provider_calls() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("evidence.json");
        let out = dir.path().join("scorecard.json");
        let evidence = serde_json::json!({
            "benchmark_version": "heldout-v1",
            "optimizer_task_ids": ["train-1"],
            "heldout_task_ids": ["held-1"],
            "runs": [{"run_id": "r1", "heldout_passed": true, "areas": {}}]
        });
        std::fs::write(&input, serde_json::to_vec(&evidence).unwrap()).unwrap();
        cmd_qualify(QualifyArgs {
            input: Some(input),
            ..qualify_args(Some(out.clone()))
        })
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
        assert_eq!(report["overall"], "Unrated");
        assert_eq!(report["heldout_passed"], false);
    }

    #[test]
    fn qualify_command_requires_either_input_or_record() {
        let err = format!("{:#}", cmd_qualify(qualify_args(None)).unwrap_err());
        assert!(err.contains("--input or --record"), "{err}");
    }

    #[test]
    fn qualify_command_folds_run_records_into_the_nine_areas() {
        let dir = tempfile::tempdir().unwrap();
        let record_path = dir.path().join("run.qual.json");
        let out = dir.path().join("scorecard.json");
        let record = build_run_qualification_record(
            "r0",
            Some("deadbeef".to_string()),
            BinaryProvenance {
                path: "/opt/permagent".to_string(),
                version: Some("1.0.0".to_string()),
                git_sha: Some("cafebabecafebabecafebabecafebabecafebabe".to_string()),
            },
            "anthropic",
            "claude-sonnet-5",
            true,
            &[permagent_eval::metrics::TaskResult::new(
                "held-1",
                "classic",
                permagent_eval::oracle::OracleOutcome::Pass,
                permagent_eval::cost::CostReading::known(0.1, false, 1),
            )],
        );
        std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
        cmd_qualify(QualifyArgs {
            records: vec![record_path],
            heldout_tasks: vec!["held-1".to_string()],
            ..qualify_args(Some(out.clone()))
        })
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
        // A single retained record cannot reach a three-run streak, but the
        // area map must still be rendered for all nine names.
        assert_eq!(report["areas"].as_object().unwrap().len(), 9);
        assert_eq!(report["areas"]["observability"]["rating"], "Unrated");
    }

    #[test]
    fn qualify_command_requires_heldout_task_with_record() {
        let dir = tempfile::tempdir().unwrap();
        let record_path = dir.path().join("run.qual.json");
        std::fs::write(&record_path, "{}").unwrap();
        let err = format!(
            "{:#}",
            cmd_qualify(QualifyArgs {
                records: vec![record_path],
                ..qualify_args(None)
            })
            .unwrap_err()
        );
        assert!(err.contains("--heldout-task"), "{err}");
    }

    #[test]
    fn use_keyring_parses_on_plan() {
        let cli =
            Cli::try_parse_from(["permagent-eval", "plan", "--tier", "local", "--use-keyring"])
                .unwrap();
        let Command::Plan(args) = cli.command else {
            panic!("expected Plan");
        };
        assert!(args.use_keyring);
    }

    #[test]
    fn program_inspection_is_explicitly_read_only() {
        let cli = Cli::try_parse_from([
            "permagent-eval",
            "program",
            "frontier",
            "--manifest",
            "program.yaml",
        ])
        .unwrap();
        let Command::Program {
            command: ProgramCommand::Frontier(args),
        } = cli.command
        else {
            panic!("expected program frontier");
        };
        assert_eq!(args.manifest, PathBuf::from("program.yaml"));
        assert_eq!(args.format, "text");
    }

    #[test]
    fn program_transition_requires_an_explicit_receipt_and_rejects_output_conflicts() {
        assert!(Cli::try_parse_from([
            "permagent-eval",
            "program",
            "transition",
            "--manifest",
            "program.yaml",
            "--node",
            "one",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "permagent-eval",
            "program",
            "transition",
            "--manifest",
            "program.yaml",
            "--node",
            "one",
            "--receipt",
            "check=passed",
            "--out",
            "next.yaml",
            "--in-place",
        ])
        .is_err());
    }

    #[test]
    fn program_reopen_requires_a_reason_and_rejects_output_conflicts() {
        assert!(Cli::try_parse_from([
            "permagent-eval",
            "program",
            "reopen",
            "--manifest",
            "program.yaml",
            "--node",
            "one",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "permagent-eval",
            "program",
            "reopen",
            "--manifest",
            "program.yaml",
            "--node",
            "one",
            "--reason",
            "held-out regression",
            "--out",
            "next.yaml",
            "--in-place",
        ])
        .is_err());
    }

    #[test]
    fn daemon_in_place_retry_uses_durable_manifest_without_reapplying_transition() {
        let source = r#"schema: 1
program_id: daemon-retry-test
objective: exercise pending handoff retry
terminal_node: finish
nodes:
  - id: start
    child_dag: start.md
    status: passed
    depends_on: []
    next_on_pass: [finish]
    entry_gate: [ready]
    exit_gate: [checks]
    worker_policy: cheap
  - id: finish
    child_dag: finish.md
    status: active
    depends_on: [start]
    next_on_pass: []
    entry_gate: [start passed]
    exit_gate: [approved]
    worker_policy: cheap
"#;
        let mut in_place_program = ProgramDag::from_yaml(source).unwrap();
        assert!(in_place_program
            .transition_node("start", &[ExitGateReceipt::passed("checks")])
            .is_err());

        let response = DaemonHandoffResponse {
            status: "pending_dispatch".to_string(),
            program_id: "daemon-retry-test".to_string(),
            node_id: "start".to_string(),
            activated: vec!["finish".to_string()],
            approval_required: Vec::new(),
            manifest: source.to_string(),
        };
        let transition = daemon_response_transition(&response).unwrap();
        assert_eq!(transition.node_id, "start");
        assert_eq!(transition.activated, vec!["finish".to_string()]);
        assert!(!transition.frontier.complete);
        assert!(transition
            .frontier
            .active
            .iter()
            .any(|node| node == "finish"));

        // A second response can be decoded from the same in-place manifest
        // after the durable daemon claim is retried; the CLI never attempts to
        // apply the already-completed local transition again.
        let retry = DaemonHandoffResponse {
            status: "applied".to_string(),
            ..response
        };
        assert_eq!(
            daemon_response_transition(&retry).unwrap().activated,
            vec!["finish".to_string()]
        );
    }

    #[test]
    fn program_receipt_parser_requires_a_known_status() {
        assert_eq!(
            parse_exit_gate_receipt(" tests = passed ").unwrap(),
            ExitGateReceipt::passed("tests")
        );
        assert!(parse_exit_gate_receipt("tests").is_err());
        assert!(parse_exit_gate_receipt("tests=unknown").is_err());
        assert!(parse_exit_gate_receipt("=passed").is_err());
    }

    #[test]
    fn failed_transition_leaves_manifest_untouched_and_success_writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("program.yaml");
        let output = dir.path().join("next.yaml");
        let raw = r#"schema: 1
program_id: cli-test
objective: exercise the program cli
terminal_node: finish
nodes:
  - id: start
    child_dag: start.md
    status: active
    depends_on: []
    next_on_pass: [finish]
    entry_gate: [ready]
    exit_gate: [checks]
    worker_policy: cheap
  - id: finish
    child_dag: finish.md
    status: planned
    depends_on: [start]
    next_on_pass: []
    entry_gate: [start passed]
    exit_gate: [approved]
    worker_policy: integrator
    approval: human
"#;
        fs::write(&manifest, raw).unwrap();

        cmd_program_transition(ProgramTransitionArgs {
            manifest: manifest.clone(),
            node: "start".into(),
            receipts: vec!["checks=failed".into()],
            goal: None,
            daemon: false,
            out: Some(output.clone()),
            in_place: false,
            format: "text".into(),
        })
        .unwrap_err();
        assert_eq!(fs::read_to_string(&manifest).unwrap(), raw);
        assert!(!output.exists());

        cmd_program_transition(ProgramTransitionArgs {
            manifest: manifest.clone(),
            node: "start".into(),
            receipts: vec!["checks=passed".into()],
            goal: None,
            daemon: false,
            out: Some(output.clone()),
            in_place: false,
            format: "json".into(),
        })
        .unwrap();
        let transitioned = ProgramDag::from_yaml(&fs::read_to_string(output).unwrap()).unwrap();
        assert_eq!(
            transitioned
                .nodes
                .iter()
                .find(|node| node.id == "start")
                .unwrap()
                .status,
            permagent_eval::ProgramNodeStatus::Passed
        );
        // The input is still unchanged when --out is used.
        assert_eq!(fs::read_to_string(&manifest).unwrap(), raw);
    }

    #[test]
    fn program_reopen_cli_resets_downstream_nodes_and_preserves_input_with_out() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("program.yaml");
        let output = dir.path().join("reopened.yaml");
        let raw = r#"schema: 1
program_id: reopen-cli-test
objective: reopen a retained defect
terminal_node: finish
nodes:
  - id: start
    child_dag: start.md
    status: passed
    depends_on: []
    next_on_pass: [finish]
    entry_gate: [ready]
    exit_gate: [checks]
    worker_policy: cheap
  - id: finish
    child_dag: finish.md
    status: active
    depends_on: [start]
    next_on_pass: []
    entry_gate: [start passed]
    exit_gate: [approved]
    worker_policy: integrator
    approval: human
"#;
        fs::write(&manifest, raw).unwrap();

        cmd_program_reopen(ProgramReopenArgs {
            manifest: manifest.clone(),
            node: "start".into(),
            reason: "held-out regression".into(),
            out: Some(output.clone()),
            in_place: false,
            format: "json".into(),
        })
        .unwrap();

        let reopened = ProgramDag::from_yaml(&fs::read_to_string(output).unwrap()).unwrap();
        assert_eq!(
            reopened.nodes[0].status,
            permagent_eval::ProgramNodeStatus::Active
        );
        assert_eq!(
            reopened.nodes[1].status,
            permagent_eval::ProgramNodeStatus::Planned
        );
        assert_eq!(fs::read_to_string(&manifest).unwrap(), raw);
    }

    // ── D2: JSON envelope stamping ─────────────────────────────────────────

    #[test]
    fn stamp_json_envelope_adds_run_id_and_generated_at_without_touching_other_fields() {
        let base = serde_json::json!({"tiers": []});
        let when = chrono::DateTime::parse_from_rfc3339("2026-09-09T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let stamped = stamp_json_envelope(base, "run-20260909T100000Z", when);
        assert_eq!(stamped["run_id"], "run-20260909T100000Z");
        assert_eq!(stamped["generated_at"], "2026-09-09T10:00:00+00:00");
        assert_eq!(stamped["tiers"], serde_json::json!([]));
    }

    // ── G3: dag_selection_pct envelope stamping ─────────────────────────────

    #[test]
    fn stamp_dag_selection_pct_inserts_the_field_when_some() {
        let base = serde_json::json!({"tiers": []});
        let stamped = stamp_dag_selection_pct(base, Some(0.97));
        assert!((stamped["dag_selection_pct"].as_f64().unwrap() - 0.97).abs() < 1e-12);
    }

    #[test]
    fn stamp_dag_selection_pct_omits_the_field_when_none() {
        let base = serde_json::json!({"tiers": []});
        let stamped = stamp_dag_selection_pct(base, None);
        assert!(
            stamped
                .as_object()
                .unwrap()
                .get("dag_selection_pct")
                .is_none(),
            "must not fabricate a value when no selection fixture ran: {stamped}"
        );
    }

    // ── D1/D4: `graduate` subcommand ────────────────────────────────────────

    #[test]
    fn graduate_subcommand_parses_run_provider_model_role_and_dry_run() {
        let cli = Cli::try_parse_from([
            "permagent-eval",
            "graduate",
            "--run",
            "a.json",
            "--run",
            "b.json",
            "--provider",
            "anthropic",
            "--model",
            "claude-sonnet-5",
            "--role",
            "edit",
            "--dry-run",
        ])
        .unwrap();
        let Command::Graduate(args) = cli.command else {
            panic!("expected Graduate");
        };
        assert_eq!(
            args.runs,
            vec![PathBuf::from("a.json"), PathBuf::from("b.json")]
        );
        assert_eq!(args.provider.as_deref(), Some("anthropic"));
        assert_eq!(args.role.as_deref(), Some("edit"));
        assert!(args.dry_run);
    }

    /// A retained run report shaped exactly like `render_json`'s output plus
    /// the additive `generated_at`/`dag_selection_pct` fields, with the
    /// fizzbuzz smoke solved and a high pass rate — a qualifying run for the
    /// default ORCHESTRATE role.
    fn qualifying_run_json(day: u32, pass: bool) -> serde_json::Value {
        serde_json::json!({
            "generated_at": format!("2026-09-{day:02}T00:00:00Z"),
            "dag_selection_pct": if pass { 0.97 } else { 0.5 },
            "tiers": [{
                "tier": "sonnet5",
                "provider": "anthropic",
                "model": "claude-sonnet-5",
                "pinned_packs": true,
                "summary": {
                    "attempted": 10,
                    "solved": if pass { 9 } else { 5 },
                    "total_cost_usd": 0.42,
                    "median_duration_secs": 12.0,
                },
                "tasks": [{"task_id": "fizzbuzz", "solved": pass}],
            }],
        })
    }

    #[test]
    fn cmd_graduate_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let directory_path = dir.path().join("capability_directory.json");
        let mut runs = Vec::new();
        for day in 1..=3 {
            let path = dir.path().join(format!("run{day}.json"));
            fs::write(
                &path,
                serde_json::to_vec(&qualifying_run_json(day, true)).unwrap(),
            )
            .unwrap();
            runs.push(path);
        }
        cmd_graduate(GraduateArgs {
            runs,
            provider: None,
            model: None,
            role: None,
            worker: false,
            snapshot_date: None,
            directory: Some(directory_path.clone()),
            dry_run: true,
        })
        .unwrap();
        assert!(
            !directory_path.exists(),
            "dry-run must not write the directory file"
        );
    }

    #[test]
    fn cmd_graduate_three_qualifying_runs_graduate_the_orchestrate_role() {
        let dir = tempfile::tempdir().unwrap();
        let directory_path = dir.path().join("capability_directory.json");
        let mut runs = Vec::new();
        for day in 1..=3 {
            let path = dir.path().join(format!("run{day}.json"));
            fs::write(
                &path,
                serde_json::to_vec(&qualifying_run_json(day, true)).unwrap(),
            )
            .unwrap();
            runs.push(path);
        }
        cmd_graduate(GraduateArgs {
            runs,
            provider: None,
            model: None,
            role: None,
            worker: false,
            snapshot_date: None,
            directory: Some(directory_path.clone()),
            dry_run: false,
        })
        .unwrap();

        let saved = permagent_eval::graduate::load_directory_json(&directory_path)
            .expect("cmd_graduate must have written the directory");
        let evidence = permagent_eval::graduate::existing_role_evidence(
            &saved,
            "anthropic",
            "claude-sonnet-5",
            WorkflowRole::Orchestrate,
        )
        .expect("the graded role must exist");
        assert!(evidence.graduated, "three qualifying runs must graduate");
        assert_eq!(evidence.evidence_run_ids.len(), 3);
        assert!((evidence.micro_suite_pass_pct.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn cmd_graduate_two_qualifying_runs_do_not_graduate() {
        let dir = tempfile::tempdir().unwrap();
        let directory_path = dir.path().join("capability_directory.json");
        let mut runs = Vec::new();
        for day in 1..=2 {
            let path = dir.path().join(format!("run{day}.json"));
            fs::write(
                &path,
                serde_json::to_vec(&qualifying_run_json(day, true)).unwrap(),
            )
            .unwrap();
            runs.push(path);
        }
        cmd_graduate(GraduateArgs {
            runs,
            provider: None,
            model: None,
            role: None,
            worker: false,
            snapshot_date: None,
            directory: Some(directory_path.clone()),
            dry_run: false,
        })
        .unwrap();
        let saved = permagent_eval::graduate::load_directory_json(&directory_path).unwrap();
        let evidence = permagent_eval::graduate::existing_role_evidence(
            &saved,
            "anthropic",
            "claude-sonnet-5",
            WorkflowRole::Orchestrate,
        )
        .unwrap();
        assert!(!evidence.graduated);
    }

    #[test]
    fn cmd_graduate_refuses_a_run_with_no_provider_model_and_no_flags() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.json");
        let raw = serde_json::json!({
            "generated_at": "2026-09-01T00:00:00Z",
            "tiers": [{
                "summary": {"attempted": 1, "solved": 1},
                "tasks": [{"task_id": "fizzbuzz", "solved": true}],
            }],
        });
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let err = cmd_graduate(GraduateArgs {
            runs: vec![path],
            provider: None,
            model: None,
            role: None,
            worker: false,
            snapshot_date: None,
            directory: Some(dir.path().join("capability_directory.json")),
            dry_run: false,
        })
        .expect_err("missing provider/model with no flags must be refused");
        let msg = err.to_string();
        assert!(msg.contains("never guessed"), "{msg}");
    }
}
