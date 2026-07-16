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

#[cfg(test)]
mod tests {
    use super::*;
    use permagent_eval::cost::CostReading;
    use permagent_eval::metrics::TaskResult;
    use permagent_eval::oracle::OracleOutcome;

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
}
