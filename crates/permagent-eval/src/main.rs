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

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

use permagent_eval::cost::LedgerCostReader;
use permagent_eval::invocation::build_invocation;
use permagent_eval::report::{render_json, render_markdown, render_text, TierReport};
use permagent_eval::runner::{
    run_tier, Deps, RunConfig, SubprocessHarnessRunner, SubprocessOracleRunner,
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
}

#[derive(Args, Debug)]
struct TasksDirArg {
    /// Directory holding the task set (defaults to the bundled `tasks/`).
    #[arg(long, value_name = "DIR")]
    tasks_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct TierSelection {
    /// A built-in tier to run (repeatable): local, kimi, minimax, sonnet, frontier.
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
    /// Resolve the requested tiers, applying the pack-pinning choice.
    fn resolve(&self) -> Result<Vec<Tier>> {
        let mut tiers: Vec<Tier> = Vec::new();
        for name in &self.tiers {
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
    }
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
            let inv = build_invocation(&task.spec, tier, workdir, data_root, &args.permagent_bin);
            println!("## {}", task.spec.id);
            println!("{}\n", inv.display_line());
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

    let runs_root = args
        .runs_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("permagent-eval-runs"));
    let cfg = RunConfig {
        permagent_bin: args.permagent_bin.clone(),
        runs_root,
        keep: args.keep,
    };
    let harness = SubprocessHarnessRunner;
    let oracle = SubprocessOracleRunner;
    let cost = LedgerCostReader;
    let deps = Deps {
        harness: &harness,
        oracle: &oracle,
        cost: &cost,
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
        let results = run_tier(&tasks, tier, &cfg, &deps);
        for r in &results {
            eprintln!(
                "    {:<6} {:<22} {}",
                if r.solved { "PASS" } else { "FAIL" },
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
    Ok(())
}

/// Error (unless allowed) when a selected tier needs an API key that is not set.
fn preflight_keys(tiers: &[Tier], allow_missing: bool) -> Result<()> {
    for tier in tiers {
        let Some(key) = &tier.required_key_env else {
            continue;
        };
        if std::env::var(key).is_ok() {
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
