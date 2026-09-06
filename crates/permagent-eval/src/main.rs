//! `permagent-eval` — run the curated coding task set against the Permagent
//! coding harness under one or more model tiers, then report pass-rate, $/solved
//! and median $/task.
//!
//! Subcommands:
//! - `run`      — execute tasks under the given tier(s) and print/write a report.
//! - `plan`     — print the exact `permagent run` invocations without executing
//!   (safe anywhere; no models are called).
//! - `list`     — list the bundled tasks.
//! - `validate` — load and validate every task spec.
//! - `qualify`  — validate retained held-out evidence and emit a scorecard.

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use permagent_eval::{
    ExitGateReceipt, ProgramDag, ProgramFrontier, ProgramReopen, ProgramTransition,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use permagent_eval::cost::LedgerCostReader;
use permagent_eval::invocation::build_invocation;
use permagent_eval::qualification::QualificationInput;
use permagent_eval::report::{render_json, render_markdown, render_text, TierReport};
use permagent_eval::runner::{
    run_tier, BudgetTracker, Deps, RunConfig, SubprocessHarnessRunner, SubprocessOracleRunner,
    SubprocessRecipeSource,
};
use permagent_eval::task::{load_task_set, select_tasks, Task};
use permagent_eval::tier::Tier;

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
    /// Print the constructed harness invocations without running them.
    Plan(PlanArgs),
    /// List the bundled tasks.
    List(TasksDirArg),
    /// Validate every task spec (parse + invariants + id/dir coupling).
    Validate(TasksDirArg),
    /// Validate retained held-out evidence and emit its derived scorecard.
    Qualify(QualifyArgs),
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
    /// JSON evidence file produced by the outer evaluation DAG. Use `-` for stdin.
    #[arg(long, value_name = "FILE")]
    input: PathBuf,

    /// Also write the derived scorecard to this file.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
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

    /// Also write the report to this file.
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
        Command::Plan(args) => cmd_plan(args),
        Command::List(arg) => cmd_list(&resolve_tasks_dir(&arg)),
        Command::Validate(arg) => cmd_validate(&resolve_tasks_dir(&arg)),
        Command::Qualify(args) => cmd_qualify(args),
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

fn cmd_qualify(args: QualifyArgs) -> Result<()> {
    let raw = if args.input == Path::new("-") {
        let mut raw = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)
            .context("reading qualification evidence from stdin")?;
        raw
    } else {
        std::fs::read_to_string(&args.input).with_context(|| {
            format!(
                "reading qualification evidence from {}",
                args.input.display()
            )
        })?
    };
    let input: QualificationInput = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parsing qualification evidence JSON from {}",
            args.input.display()
        )
    })?;
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

fn cmd_run(args: RunArgs) -> Result<()> {
    let tiers = args.tiers.resolve()?;
    let tasks_dir = resolve_tasks_dir(&args.tasks_dir);
    let tasks: Vec<Task> = select_tasks(load_task_set(&tasks_dir)?, &args.tasks)?;
    if tasks.is_empty() {
        bail!("no tasks to run in {}", tasks_dir.display());
    }

    preflight_keys(&tiers, args.allow_missing_keys)?;
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
    let cfg = RunConfig {
        permagent_bin,
        runs_root,
        keep: args.keep,
        use_keyring: args.use_keyring,
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

    let rendered = match args.format.as_str() {
        "json" => serde_json::to_string_pretty(&render_json(&reports))?,
        "md" => render_markdown(&reports),
        _ => render_text(&reports),
    };
    println!("{rendered}");
    if let Some(path) = &args.out {
        std::fs::write(path, &rendered)
            .with_context(|| format!("writing report to {}", path.display()))?;
        eprintln!("report written to {}", path.display());
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
fn preflight_keys(tiers: &[Tier], allow_missing: bool) -> Result<()> {
    preflight_keys_with(tiers, allow_missing, |k| std::env::var(k).ok())
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
                "warning: tier {} needs {} but it is not set (continuing)",
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
    use permagent_eval::metrics::TaskResult;
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
        assert_eq!(args.input, PathBuf::from("evidence.json"));
        assert_eq!(args.out, Some(PathBuf::from("scorecard.json")));
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
            input,
            out: Some(out.clone()),
        })
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
        assert_eq!(report["overall"], "Unrated");
        assert_eq!(report["heldout_passed"], false);
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
}
