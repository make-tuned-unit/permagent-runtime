//! Orchestration: prepare an isolated run, invoke the harness, grade with the
//! oracle, and read the cost — for one task, then a whole tier.
//!
//! The subprocess-touching pieces sit behind the [`HarnessRunner`],
//! [`OracleRunner`] and [`RecipeSource`] traits (alongside
//! [`CostReader`](crate::cost::CostReader)) so the decision logic and the
//! tamper-proofing copy are tested with mocks, while the real
//! [`SubprocessHarnessRunner`] / [`SubprocessOracleRunner`] /
//! [`SubprocessRecipeSource`] are the thin glue exercised for real on the
//! machine that runs the eval.

use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::cost::{CostReader, CostReading};
use crate::harness_log;
use crate::invocation::{build_invocation, is_scrubbed_env, recipe_with_prompt, Invocation};
use crate::metrics::TaskResult;
use crate::oracle::{outcome_from_run, OracleOutcome};
use crate::provider_unavailable;
use crate::seed_state;
use crate::task::Task;
use crate::tier::Tier;

/// The result of running the harness subprocess.
#[derive(Debug, Clone)]
pub struct HarnessRun {
    /// Process exit code (`None` = killed / signalled / timed out).
    pub exit: Option<i32>,
    /// Whether the run was killed for exceeding its wall-clock ceiling.
    pub timed_out: bool,
    /// Wall-clock duration of the run.
    pub duration: Duration,
}

/// Runs the coding harness for one invocation.
pub trait HarnessRunner {
    fn run(&self, inv: &Invocation, timeout: Duration, log_path: &Path) -> Result<HarnessRun>;
}

/// The result of running a task's test oracle process. Mirrors
/// [`HarnessRun`]'s shape — Lane K7: a bare exit code alone could not tell
/// [`run_task`] whether a wall-clock-ceiling breach happened, since a
/// SIGTERM'd process can still exit with a real-looking code during its
/// grace window (see [`crate::oracle::outcome_from_run`]).
#[derive(Debug, Clone)]
pub struct OracleRun {
    /// Process exit code (`None` = killed / signalled / timed out).
    pub exit: Option<i32>,
    /// Whether the oracle was killed for exceeding its wall-clock ceiling
    /// (`task.oracle_timeout_secs()`).
    pub timed_out: bool,
    /// Wall-clock duration the oracle process ran, regardless of outcome.
    pub duration: Duration,
}

/// Runs a task's test oracle in the finished workspace.
pub trait OracleRunner {
    /// `envs` are SET on top of the child's otherwise-inherited environment
    /// — Lane R18c (A17 Class 3): the caller uses this to pin
    /// `CARGO_TARGET_DIR` to a run-scoped directory (see
    /// [`cargo_target_dir`]) rather than let the oracle's `cargo test`
    /// inherit whatever the parent process's ambient environment happens to
    /// carry.
    fn run(
        &self,
        workdir: &Path,
        argv: &[String],
        envs: &[(String, String)],
        timeout: Duration,
        log_path: &Path,
    ) -> Result<OracleRun>;
}

/// Resolves the BASE recipe YAML text for a task — before the per-run
/// `prompt:` key is spliced in by [`recipe_with_prompt`] — so [`run_task`]
/// stays testable with a fake instead of always shelling out.
pub trait RecipeSource {
    /// Return the base recipe YAML for `recipe_name_or_path` (a `TaskSpec::recipe`
    /// value: either a reserved recipe NAME, e.g. the built-in `permagent-coding`,
    /// or a path to a recipe file).
    fn base_recipe_yaml(&self, recipe_name_or_path: &str, permagent_bin: &str) -> Result<String>;
}

/// The four injected dependencies of a run.
pub struct Deps<'a> {
    pub harness: &'a dyn HarnessRunner,
    pub oracle: &'a dyn OracleRunner,
    pub cost: &'a dyn CostReader,
    pub recipe: &'a dyn RecipeSource,
}

/// Static configuration for a run session.
#[derive(Debug, Clone, Default)]
pub struct RunConfig {
    /// The `permagent` binary (name on PATH or an absolute path).
    pub permagent_bin: String,
    /// Where per-run scratch directories are created.
    pub runs_root: PathBuf,
    /// Keep scratch directories after the run (for debugging).
    pub keep: bool,
    /// Let the child read the OS keychain for provider secrets
    /// (`--use-keyring`), instead of forcing `PERMAGENT_DISABLE_KEYRING=1`. See
    /// [`crate::invocation::build_invocation`].
    pub use_keyring: bool,
    /// The A/B **arm**: extra environment SET on the harness child on top of
    /// the invocation's own (see [`crate::ab::ArmConfig`]). Empty for a plain
    /// `run`. Applied last, so an arm knob wins over both the inherited
    /// environment and the invocation's defaults — that is the point: the arm
    /// is the only thing allowed to differ between the two sides of a
    /// comparison.
    pub arm_env: Vec<(String, String)>,
    /// Variable NAMES the arm removes from the child's inherited environment,
    /// merged into [`Invocation::unset_envs`].
    pub arm_unset: Vec<String>,
    /// Lane K6 / K6-b: `--harness-timeout-secs <N>` — a per-INVOCATION
    /// override of the harness wall-clock ceiling that beats every other
    /// source (a task's own `harness_timeout_secs:` in `task.yaml` AND the
    /// tier's own floor; see
    /// [`crate::task::TaskSpec::resolved_harness_timeout_secs`]). `None`
    /// (the default) leaves the task-cap/tier-floor/default resolution
    /// alone. This is a deliberate measurement knob for pinning an EXACT
    /// ceiling on one invocation — the tier floor already scales a slow
    /// provider's ceiling automatically without it — and the resolved
    /// value it produces is retained on
    /// [`crate::metrics::TaskResult::harness_timeout_secs`] and from there on
    /// [`crate::qualification::TaskQualificationEvidence::harness_timeout_secs`]
    /// so latency evidence stays honest about which ceiling was actually used.
    pub harness_timeout_secs: Option<u64>,
}

/// Removes a run's scratch directory when dropped — on success AND on every
/// error path (failed seeding, spawn errors, oracle errors) — unless the run
/// asked to keep it (`--keep`). Errors are best-effort ignored: cleanup must
/// never mask the real failure.
struct ScratchGuard {
    dir: PathBuf,
    keep: bool,
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// Lane R18c (A17 Class 3): a run-scoped `CARGO_TARGET_DIR`, set on BOTH the
/// harness child (so a `shell`/`verify` tool call that runs `cargo build`/
/// `cargo test` inside the workspace is scoped too) and the oracle child.
/// `run_dir` is the run's own scratch directory (the parent of
/// `workspace`/`logs`/`data`, unique per task run and removed with the rest
/// of it by [`ScratchGuard`]).
///
/// Without this, a Rust fixture's `cargo test` falls back to whatever
/// `CARGO_TARGET_DIR` happens to sit in the parent `permagent-eval`
/// process's ambient environment (this driver never scrubs it, unlike the
/// `PERMAGENT_*` secrets `is_scrubbed_env` covers) — M17 round 2 observed
/// every Rust fixture's build land in `/Users/j/Documents/dev/permagent-
/// runtime/target` (the HOST repo's own target dir, not the isolated
/// per-run workspace), so concurrent rounds contended the SAME cargo build
/// lock as each other and as the host repo's own builds: an 11x wall-clock
/// blow-up on at least one task, and lock contention is indistinguishable
/// from a genuine hang until you go read the oracle log.
pub fn cargo_target_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("cargo-target")
}

/// Run a single task under a tier and return its [`TaskResult`].
pub fn run_task(task: &Task, tier: &Tier, cfg: &RunConfig, deps: &Deps<'_>) -> Result<TaskResult> {
    let run_dir = cfg
        .runs_root
        .join(format!("{}-{}-{}", tier.name, task.spec.id, unique_stamp()));
    // Guard the scratch dir from here on: every `?` below must clean up too.
    let _scratch = ScratchGuard {
        dir: run_dir.clone(),
        keep: cfg.keep,
    };
    let workdir = run_dir.join("workspace");
    let data_root = run_dir.join("data");
    let logs = run_dir.join("logs");
    fs::create_dir_all(&workdir).with_context(|| format!("creating {}", workdir.display()))?;
    fs::create_dir_all(&data_root)?;
    fs::create_dir_all(&logs)?;
    seed_custom_providers(&data_root)?;

    // Seed the workspace the agent will see and edit.
    if task.has_workspace() {
        copy_dir_contents(&task.workspace_dir(), &workdir).context("seeding task workspace")?;
    }

    // Real user projects are git repositories. Internal goal dispatch inside
    // the harness (a worker sub-agent branching off the project) hard-requires
    // a git baseline commit to isolate the worker's worktree from — see
    // `resolve_internal_isolation` in
    // crates/goose/src/agents/platform_extensions/goal_engine.rs — and errors
    // out otherwise. A bare scratch directory would never look like a real
    // project to that check, so give it one.
    let seed_commit = seed_git_repo(&workdir, &task.spec.id).context("seeding git baseline")?;

    // Lane K8: a fixture may ship `seed/harness_state.json` describing
    // pre-existing projects/roadmaps/goal cards a `category: "selection"`
    // fixture's prose depends on (a paused roadmap, a Ready goal card) that
    // the bare `personal` project above can never satisfy. Applied AFTER the
    // seeding above (the workspace + git baseline exist) and BEFORE the
    // harness is invoked below, so the coordinator's very first turn already
    // sees the state the prompt describes. See `seed_state` for why this
    // shells out once rather than duplicating the harness's own schema.
    let seeded_state = match task.seed_state_path() {
        Some(seed_path) => {
            let seed = seed_state::load(&seed_path)
                .with_context(|| format!("loading seed state for task `{}`", task.spec.id))?;
            seed_state::apply(&seed, &cfg.permagent_bin, &data_root, &workdir)
                .with_context(|| format!("seeding harness state for task `{}`", task.spec.id))?;
            true
        }
        None => false,
    };

    // Write the per-run recipe file: `--recipe` and `-t`/`--text` are declared
    // mutually exclusive on `permagent-cli`'s `InputOptions`
    // (crates/goose-cli/src/cli.rs:188-220), so the task's prompt cannot be
    // passed as a separate flag — it must be embedded INSIDE the recipe that
    // `--recipe` points at (see `recipe_with_prompt`).
    let base_recipe = deps
        .recipe
        .base_recipe_yaml(&task.spec.recipe, &cfg.permagent_bin)
        .with_context(|| format!("resolving base recipe `{}`", task.spec.recipe))?;
    let recipe_yaml = recipe_with_prompt(&base_recipe, &task.spec.prompt);
    let recipe_path = run_dir.join("recipe.yaml");
    fs::write(&recipe_path, &recipe_yaml)
        .with_context(|| format!("writing recipe file {}", recipe_path.display()))?;

    let mut inv = build_invocation(
        &task.spec,
        tier,
        &workdir,
        &data_root,
        &recipe_path,
        &cfg.permagent_bin,
        cfg.use_keyring,
    );
    // Lane K6 / K6-b: resolve the harness wall-clock ceiling this run
    // actually uses — `--harness-timeout-secs` (this invocation's `cfg`)
    // overrides everything; otherwise the tier's own floor (e.g. the zai
    // tiers' 900s) is taken as `max(task cap or DEFAULT_HARNESS_TIMEOUT_SECS,
    // tier value)`, never shortening a task's own cap. See
    // `TaskSpec::resolved_harness_timeout_secs` for the full precedence and
    // why: zai (glm47/glm53) rounds z2/z3 on M19 hit the driver's flat 600s
    // per-task ceiling on two fixtures whose task.yaml caps them tighter
    // than zai needs, while zai's own solved-task walls run 150-440s and it
    // solved one of the two in 526s under isolation — the tier floor now
    // raises those two fixtures' effective ceiling to `max(600, 900) = 900`
    // automatically, and the CLI override still exists to pin an exact
    // ceiling for one invocation when that isn't enough.
    let effective_harness_timeout_secs = task
        .spec
        .resolved_harness_timeout_secs(tier, cfg.harness_timeout_secs);
    // The user's ruling (option 3, HARNESS_LOOP_LOG.md "### JESSE RULING"
    // 2026-09-08): headless auto-land stays a policy switch, DEFAULT OFF —
    // ordinary headless runs keep landing human-gated. The eval is exactly
    // the "deliberate unattended run" that turns it on explicitly, alongside
    // GOOSE_MODE=auto and the tier's PACK pins set above by `build_invocation`.
    // The drain window must finish inside this run's own harness timeout, so
    // it is derived from it (minus a minute of margin) rather than fixed —
    // never less than 60s, so a short per-task override can't collapse it to
    // (or below) zero.
    let drain_secs = headless_drain_secs(effective_harness_timeout_secs);
    inv.envs
        .push(("PERMAGENT_HEADLESS_AUTOLAND".to_string(), "1".to_string()));
    inv.envs.push((
        "PERMAGENT_HEADLESS_DRAIN_SECS".to_string(),
        drain_secs.to_string(),
    ));
    // Lane R17a (M17 readiness audit §6.1, gaps 1 & 4): the harness mirrors
    // its RECEIPT CONTRACT v1 lines plus `harness_overhead` into this file as
    // it runs (atomically, on every counter change and turn boundary), so a
    // SIGKILL for exceeding `harness_timeout_secs` below — which skips the
    // clean-exit stdout print entirely — still leaves evidence on disk. See
    // `signals` below, where it is read back and merged in as a fallback.
    let receipts_sidecar_path = run_dir.join("harness-receipts.txt");
    inv.envs.push((
        "PERMAGENT_RECEIPTS_SIDECAR".to_string(),
        receipts_sidecar_path.to_string_lossy().into_owned(),
    ));
    // Lane R18c (A17 Class 3): pin CARGO_TARGET_DIR to this run's own
    // directory — see `cargo_target_dir`'s doc for why. Set on the harness
    // child too (not just the oracle below) so a `shell`/`verify` tool call
    // that itself runs `cargo build`/`cargo test` inside the workspace is
    // scoped the same way.
    let cargo_target_dir_path = cargo_target_dir(&run_dir);
    inv.envs.push((
        "CARGO_TARGET_DIR".to_string(),
        cargo_target_dir_path.to_string_lossy().into_owned(),
    ));
    // The arm's knobs go on LAST so they outrank the invocation's own defaults
    // — including the two headless-autoland defaults just set above — as well
    // as the inherited environment: an A/B arm that could be silently
    // overridden by either would measure nothing.
    inv.unset_envs.extend(cfg.arm_unset.iter().cloned());
    inv.envs.extend(cfg.arm_env.iter().cloned());
    let harness_log_path = logs.join("harness.log");
    let harness_run = deps.harness.run(
        &inv,
        Duration::from_secs(effective_harness_timeout_secs),
        &harness_log_path,
    )?;
    // Best-effort: a missing/unreadable log just yields zeroed signals rather
    // than failing the whole task (the oracle's verdict is authoritative).
    // Lane K10: keep the raw text too — `provider_unavailable::classify_in_run`
    // below scans it (and, if that comes up empty, this run's own
    // `data/logs/llm_request.*.jsonl`) for a billing/auth/quota marker.
    let harness_log_text = fs::read_to_string(&harness_log_path).unwrap_or_default();
    let mut signals = harness_log::scan(&harness_log_text);
    // Lane R17a: fill in any of the five RECEIPT CONTRACT v1 fields the
    // captured stdout log is missing from the sidecar file above — a no-op
    // when the harness exited cleanly and already printed everything (see
    // `fill_receipts_from_sidecar`'s "stdout wins" rule), and best-effort
    // when the sidecar was never written at all (a harness build that
    // predates this lane, or a run that died before its first receipt
    // event).
    let receipt_source = fs::read_to_string(&receipts_sidecar_path)
        .ok()
        .and_then(|text| harness_log::fill_receipts_from_sidecar(&mut signals, &text));

    // Tamper-proof grading: overlay the pristine oracle files onto the workspace
    // so the agent cannot have weakened or deleted its own grader.
    if task.has_oracle() {
        copy_dir_contents(&task.oracle_dir(), &workdir).context("overlaying oracle files")?;
    }

    let oracle_envs = [(
        "CARGO_TARGET_DIR".to_string(),
        cargo_target_dir_path.to_string_lossy().into_owned(),
    )];
    let oracle_run = deps.oracle.run(
        &workdir,
        &task.spec.test,
        &oracle_envs,
        Duration::from_secs(task.spec.oracle_timeout_secs()),
        &logs.join("oracle.log"),
    )?;
    // Lane K7: `timed_out` always wins over the raw exit code — see
    // `outcome_from_run`'s doc for why a SIGTERM'd process's own exit code
    // cannot be trusted as a graded verdict, and why an oracle timeout must
    // be reported `errored`, never `fail`.
    let mut outcome = outcome_from_run(oracle_run.exit, oracle_run.timed_out);

    // Lane K10 (2026-09-14 M22 zai w1 defect): a billing/auth/quota failure
    // means the harness never got a real chance to attempt the task at
    // all — grading it on whatever the oracle happened to see (typically
    // `fail`, since nothing was produced) would be dishonest. Detected
    // AFTER the oracle runs (the oracle's own verdict is otherwise
    // authoritative) but always wins over it, the same way an oracle
    // timeout always wins above.
    //
    // Lane K15: a bare marker is not enough on its own — the harness's
    // planner tries `anthropic` first, by design, before falling back to
    // this tier's own provider, so `classify_in_run` only forces `errored`
    // when the marker's own line names THIS tier's provider (or the run
    // shows a terminal provider-unavailable exit); a marker naming a
    // different, fallback-able provider is kept as evidence on `signals`
    // instead of overriding the oracle's verdict. See
    // `provider_unavailable`'s module docs for the full rule and the
    // M31/M33 `dschat` evidence it was built from.
    let provider_classification =
        provider_unavailable::classify_in_run(&tier.provider, &harness_log_text, &data_root);
    if provider_classification.unavailable {
        outcome = OracleOutcome::Errored;
    }
    signals.provider_unavailable_ignored = provider_classification.ignored_count;
    let provider_error = if provider_classification.unavailable {
        provider_classification.marker
    } else {
        None
    };

    let reading = deps
        .cost
        .read_total(&data_root)
        .unwrap_or_else(|_| CostReading::unknown());

    let mut result = TaskResult::new(
        task.spec.id.as_str(),
        task.spec.category.as_str(),
        outcome,
        reading,
    );
    result.duration_secs = harness_run.duration.as_secs_f64();
    result.harness_exit = harness_run.exit;
    result.harness_timed_out = harness_run.timed_out;
    result.oracle_exit = oracle_run.exit;
    result.oracle_timed_out = oracle_run.timed_out;
    result.oracle_secs = oracle_run.duration.as_secs_f64();
    // Lane K6: retain the resolved ceiling THIS run actually used, so
    // latency evidence in the qualification record stays honest — a
    // duration near a task's own default alone can't say whether it was
    // measured against 600s, a tier's 900s, or a `--harness-timeout-secs`
    // override.
    result.harness_timeout_secs = Some(effective_harness_timeout_secs);
    // G3: a `category: "selection"` fixture is graded on the tool/DAG
    // decision, not (only) the oracle's verdict — evaluate its declared
    // `selection_check` against THIS run's own signals, independent of
    // whether the oracle passed. `None` for every non-selection fixture.
    // Lane R18c (A17 Q6): keep the specific rule(s) it violated too, so a
    // report/`.qual.json` reader can see WHY without re-deriving it.
    let selection_verdict = task
        .spec
        .selection_check
        .as_ref()
        .map(|check| check.evaluate_detailed(&signals));
    result.selection_correct = selection_verdict.as_ref().map(|v| v.passed);
    result.selection_check_failures = selection_verdict.map(|v| v.failures).unwrap_or_default();
    result.signals = signals;
    result.receipt_source = receipt_source;
    result.seed_commit = Some(seed_commit);
    result.seeded_state = seeded_state;
    // Read back the FINAL, post-arm-override values so the report row shows
    // what the child actually received, not just the runner's own default.
    result.headless_autoland = inv.env("PERMAGENT_HEADLESS_AUTOLAND").map(str::to_string);
    result.headless_drain_secs = inv.env("PERMAGENT_HEADLESS_DRAIN_SECS").map(str::to_string);
    // Lane K10: a provider-unavailable marker is the most specific, most
    // actionable note available — prefer it over the generic timeout note
    // (the two can coincide, e.g. a harness that hangs retrying a billing
    // rejection until its wall-clock ceiling trips).
    if let Some(marker) = &provider_error {
        result.note = Some(format!("provider unavailable: {marker}"));
    } else if harness_run.timed_out {
        result.note = Some("harness run hit its wall-clock ceiling".to_string());
    }
    result.provider_error = provider_error;

    // `_scratch` cleans up the run dir here (and on every early return above).
    Ok(result)
}

/// Tracks cumulative MEASURED spend (from the ledger, not an estimate) across
/// an eval session that may span many tiers and tasks, against an optional
/// `--budget-usd` cap, and decides when the sweep must stop launching further
/// tasks. This is post-run accounting, not a hard preauthorization: the task
/// that crosses the cap has already completed before its cost is available.
/// Pure and side-effect free — the caller decides what to do with a tripped
/// cap (skip the next task, print the stop line).
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetTracker {
    cap_usd: Option<f64>,
    spent_usd: f64,
    attempted: usize,
    /// Set when the cap is reached, an untrusted cost is observed (Lane K5:
    /// the ledger recorded provider calls it could not price, or a priced
    /// total is invalid), or the cap configuration is invalid. The value is
    /// the attempted count at that moment (zero for an invalid cap). Stays
    /// `Some` for the session.
    stopped_after: Option<usize>,
    /// Why the posthoc stop occurred. Kept separately so a stop caused by an
    /// unknown cost is not reported as though it were measured spend.
    stop_reason: Option<BudgetStopReason>,
    /// Whether [`Self::take_stop_message`] has already handed out its one
    /// message for this trip.
    announced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetStopReason {
    CapReached,
    UnknownCost,
    InvalidCap,
}

impl BudgetTracker {
    /// No cap: the sweep never stops early.
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Stop once cumulative measured spend reaches `cap_usd`.
    ///
    /// A non-finite or negative cap is invalid and fails closed: the first
    /// task is blocked. A zero cap is valid and permits no measured spend.
    pub fn with_cap(cap_usd: f64) -> Self {
        let mut tracker = Self {
            cap_usd: Some(cap_usd),
            ..Self::default()
        };
        if !cap_usd.is_finite() || cap_usd < 0.0 {
            tracker.stopped_after = Some(0);
            tracker.stop_reason = Some(BudgetStopReason::InvalidCap);
        }
        tracker
    }

    /// Whether the cap has already tripped — i.e. whether the NEXT task
    /// should be skipped (marked not-run) rather than launched.
    pub fn cap_exceeded(&self) -> bool {
        self.stopped_after.is_some()
    }

    /// Record one attempted task's cost reading.
    ///
    /// Lane K5: a task whose ledger has no rows (`ledger_rows == 0` —
    /// the harness errored or failed to spawn before any provider call,
    /// so `reading.usd` reads as [`CostReading::unknown`]) contributes $0 to
    /// measured spend and does NOT fail closed. There is nothing untrustworthy
    /// about zero calls; the guard exists to stop a run whose spend cannot be
    /// trusted, which is only true when the ledger DID record provider calls
    /// that it could not price (`ledger_rows > 0` but `usd` is `None`) or the
    /// summed cost itself is invalid (negative, NaN, or infinite) — a real
    /// accounting problem, not an absence of spend. Either of those still
    /// fails closed for capped runs. A finite, non-negative cost trips when
    /// cumulative spend reaches the cap, so an exact-boundary run cannot
    /// launch one more task.
    pub fn record(&mut self, reading: &CostReading) {
        self.attempted += 1;
        let Some(cost) = reading.usd else {
            if reading.ledger_rows > 0 {
                // Provider calls happened but the ledger could not price
                // them: the measured total is not trustworthy.
                self.stop_unpriced_if_capped();
            }
            // Else: no provider call was ever made for this task (errored
            // before the harness ran, or spawn failure) — $0 spend, no trip.
            return;
        };
        if !cost.is_finite() || cost < 0.0 {
            self.stop_unpriced_if_capped();
            return;
        }

        let next_spent = self.spent_usd + cost;
        if !next_spent.is_finite() {
            self.stop_unpriced_if_capped();
            return;
        }
        self.spent_usd = next_spent;
        if self.stopped_after.is_none() {
            if let Some(cap) = self.cap_usd {
                if self.spent_usd >= cap {
                    self.stopped_after = Some(self.attempted);
                    self.stop_reason = Some(BudgetStopReason::CapReached);
                }
            }
        }
    }

    fn stop_unpriced_if_capped(&mut self) {
        if self.cap_usd.is_some() && self.stopped_after.is_none() {
            self.stopped_after = Some(self.attempted);
            self.stop_reason = Some(BudgetStopReason::UnknownCost);
        }
    }

    /// The `BUDGET STOP: …` line, returned exactly once — the first call
    /// after the cap trips — so a caller printing it after every tier's run
    /// does not repeat it. Later calls (and calls before/without a trip)
    /// return `None`.
    pub fn take_stop_message(&mut self) -> Option<String> {
        if self.announced {
            return None;
        }
        let cap = self.cap_usd?;
        let n = self.stopped_after?;
        self.announced = true;
        Some(
            match self.stop_reason.unwrap_or(BudgetStopReason::CapReached) {
                BudgetStopReason::CapReached => format!(
                    "BUDGET STOP: measured spend ${:.4} reached ${:.4} cap after {n} tasks",
                    self.spent_usd, cap
                ),
                BudgetStopReason::UnknownCost => format!(
                    "BUDGET STOP: unpriced or invalid cost after {n} tasks; measured spend ${:.4} of ${:.4} cap",
                    self.spent_usd, cap
                ),
                BudgetStopReason::InvalidCap => {
                    format!("BUDGET STOP: invalid cap ${cap:?}; no tasks launched")
                }
            },
        )
    }
}

/// Build a [`TaskResult`] for a task that was never launched because the
/// session's `--budget-usd` guard had already tripped. A distinct
/// [`OracleOutcome::NotRun`] state — NOT a fail — so pass-rate (computed over
/// attempted tasks only) is not dragged down by tasks that simply never ran.
///
/// Lane K5: the note distinguishes *why* the guard tripped — a plain cap hit
/// vs. an earlier task's spend being untrusted — rather than always claiming
/// the cap was reached, which was misleading when it was the unpriced-cost
/// guard that actually stopped the run.
fn not_run_result(task: &Task, reason: BudgetStopReason) -> TaskResult {
    let mut r = TaskResult::new(
        task.spec.id.as_str(),
        task.spec.category.as_str(),
        OracleOutcome::NotRun,
        CostReading::unknown(),
    );
    r.note = Some(
        match reason {
            BudgetStopReason::CapReached => "skipped: --budget-usd cap reached",
            BudgetStopReason::UnknownCost => {
                "skipped: measured spend untrusted (ledger had an unpriced provider cost)"
            }
            BudgetStopReason::InvalidCap => "skipped: --budget-usd cap is invalid",
        }
        .to_string(),
    );
    r
}

/// Build a [`TaskResult`] for a task that was never launched because
/// [`provider_unavailable::ConsecutiveTracker`] had already tripped — Lane
/// K10 (2026-09-14 M22 zai w1 defect). Mirrors [`not_run_result`]'s shape
/// (also [`OracleOutcome::NotRun`], not a fail) with its own note prefix so
/// a report reader can immediately tell a provider-unavailable stop apart
/// from a `--budget-usd` stop.
fn provider_unavailable_not_run_result(task: &Task, marker: &str) -> TaskResult {
    let mut r = TaskResult::new(
        task.spec.id.as_str(),
        task.spec.category.as_str(),
        OracleOutcome::NotRun,
        CostReading::unknown(),
    );
    r.note = Some(format!("skipped: provider unavailable ({marker})"));
    r
}

/// Run every task in `tasks` under `tier`, turning a per-task hard error into an
/// errored result so one failure never aborts the whole tier. `budget` is
/// shared across every tier in the session (the caller passes the same
/// tracker to each `run_tier` call): once its cap trips, every remaining task
/// — in this tier and any later one — is recorded as
/// [`OracleOutcome::NotRun`] instead of being launched.
///
/// Lane K10: independently of `budget`, this also tracks CONSECUTIVE
/// provider-unavailable results (billing/auth/quota — see
/// [`provider_unavailable`]) within THIS tier's own round. Once
/// [`provider_unavailable::CONSECUTIVE_STOP_THRESHOLD`] hit in a row, every
/// remaining task in this tier is recorded not-run the same way, and one
/// `PROVIDER UNAVAILABLE: …` line is printed to stderr — the M22 zai w1
/// round's actual failure was exactly this shape (task after task returning
/// the identical billing error) and ran all the way to the end anyway,
/// recording 11 dishonest `fail`s instead of stopping after the second one.
pub fn run_tier(
    tasks: &[Task],
    tier: &Tier,
    cfg: &RunConfig,
    deps: &Deps<'_>,
    budget: &mut BudgetTracker,
) -> Vec<TaskResult> {
    let mut provider_unavailable = provider_unavailable::ConsecutiveTracker::new();
    tasks
        .iter()
        .map(|task| {
            if budget.cap_exceeded() {
                let reason = budget.stop_reason.unwrap_or(BudgetStopReason::CapReached);
                return not_run_result(task, reason);
            }
            if let Some(marker) = provider_unavailable.stopped_marker() {
                return provider_unavailable_not_run_result(task, marker);
            }
            let result = run_task(task, tier, cfg, deps).unwrap_or_else(|e| {
                let mut r = TaskResult::new(
                    task.spec.id.as_str(),
                    task.spec.category.as_str(),
                    OracleOutcome::Errored,
                    CostReading::unknown(),
                );
                r.note = Some(format!("run error: {e}"));
                r
            });
            budget.record(&result.cost);
            if let Some(msg) =
                provider_unavailable.record(&tier.name, result.provider_error.as_deref())
            {
                eprintln!("{msg}");
            }
            result
        })
        .collect()
}

/// Copy the operator's declarative custom providers into this run's isolated
/// data root.
///
/// Every run gets a fresh `PERMAGENT_PATH_ROOT` so sessions and the cost ledger
/// stay isolated and the measured spend is only this run's. The cost of that is
/// that the child cannot see `~/.permagent/custom_providers/`, so a locally
/// hosted model registered there — a llama-server split, an OpenAI-compatible
/// endpoint — simply does not exist as far as the harness is concerned, and
/// could never be benchmarked. Copying the definitions in keeps the isolation
/// (they are provider declarations, not sessions or secrets) while making those
/// providers reachable.
///
/// Missing or unreadable is not an error: most tiers are built-in providers that
/// need none of this.
fn seed_custom_providers(data_root: &Path) -> Result<()> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(());
    };
    let src = home.join(".permagent").join("custom_providers");
    if !src.is_dir() {
        return Ok(());
    }
    let dest = data_root.join("custom_providers");
    fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    for entry in fs::read_dir(&src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let name = entry.file_name();
            fs::copy(entry.path(), dest.join(&name))
                .with_context(|| format!("copying custom provider {}", name.to_string_lossy()))?;
        }
    }
    Ok(())
}

/// Recursively copy the *contents* of `src` into `dst`, overwriting files.
pub fn copy_dir_contents(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_contents(&from, &to)?;
        } else {
            fs::copy(&from, &to)
                .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Turns a freshly-seeded task workspace into a git repository with one
/// commit holding exactly the seed files, so it looks like a real user
/// project rather than a bare scratch directory.
///
/// Real user projects are git repositories. The coding harness's internal
/// goal dispatch (a worker sub-agent branching off the project to do a unit
/// of work in an isolated worktree) hard-requires a git baseline commit to
/// branch from — see `resolve_internal_isolation` in
/// crates/goose/src/agents/platform_extensions/goal_engine.rs, which fails
/// the whole dispatch with "requires a git baseline commit, but the project
/// root is not a git repository" when `git rev-parse HEAD` in the working
/// directory comes back empty. Without this seeding, every eval task would
/// dispatch into that error and never exercise the thing being measured.
///
/// Idempotent: if `workspace_dir` already contains a `.git` (a `--keep`
/// rerun, or a task whose own seed ships a repo), it is left untouched and
/// its current `HEAD` is returned rather than creating a second commit.
/// `GIT_CONFIG_NOSYSTEM=1` and a purely local `user.name`/`user.email` keep
/// this from reading or depending on the operator's own git config; `HOME` is
/// left untouched, so `~/.gitconfig` still applies for anything it doesn't
/// override (nothing here needs it to).
pub fn seed_git_repo(workspace_dir: &Path, task_id: &str) -> Result<String> {
    if workspace_dir.join(".git").exists() {
        return git_rev_parse_head(workspace_dir);
    }
    run_git(workspace_dir, &["init", "-q"])?;
    run_git(workspace_dir, &["config", "user.name", "permagent-eval"])?;
    run_git(
        workspace_dir,
        &["config", "user.email", "permagent-eval@localhost"],
    )?;
    run_git(workspace_dir, &["add", "-A"])?;
    run_git(
        workspace_dir,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &format!("permagent-eval seed: {task_id}"),
        ],
    )?;
    git_rev_parse_head(workspace_dir)
}

/// Run a git subcommand in `workspace_dir`, isolated from the operator's
/// system-wide git config, failing with the captured stderr on a non-zero
/// exit.
fn run_git(workspace_dir: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Resolve `HEAD` in `workspace_dir` to its commit SHA.
fn git_rev_parse_head(workspace_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_dir)
        .args(["rev-parse", "HEAD"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("running `git rev-parse HEAD`")?;
    if !output.status.success() {
        bail!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The `PERMAGENT_HEADLESS_DRAIN_SECS` this run's headless auto-land gets:
/// `harness_timeout_secs` minus a minute of margin, floored at 60s. The
/// margin exists so the drain phase the harness runs inside its own
/// wall-clock ceiling has a chance to finish and report BEFORE this eval's own
/// `wait_with_timeout` kills the child for exceeding `harness_timeout_secs` —
/// a drain window as long as the timeout itself would race the eval's kill
/// signal and could be truncated mid-drain on every run. The 60s floor keeps a
/// short per-task override (e.g. a fast fixture with a 30s timeout) from
/// collapsing the window to zero or a negative number that `saturating_sub`
/// would otherwise silently clamp to zero.
fn headless_drain_secs(harness_timeout_secs: u64) -> u64 {
    harness_timeout_secs.saturating_sub(60).max(60)
}

/// A monotonically-unique suffix combining wall-clock nanos and a counter, so
/// concurrent or same-nanosecond runs never collide.
fn unique_stamp() -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{n}")
}

/// How long [`wait_with_timeout`] gives a child to exit on its own after
/// SIGTERM before escalating to SIGKILL (Lane R17a).
///
/// Lane R20b (A20 Q1, Mechanism B): `loc-refund`/`refactor-shipping` were
/// both killed by this SIGTERM path while a goal worker was still
/// `in_progress`. The harness's own shutdown-signal handling
/// (`crate::signal::shutdown_signal` in `goose-cli/src/session/mod.rs`,
/// which calls `park_dispatched_goals_now` → `orchestrator::
/// handle_goal_timeout`) now tries to salvage any already-committed goal
/// work before it parks (see that function's own `maybe_salvage_committed_
/// work` call) — a real `git`/verify round trip that needs more than the
/// original 5s to even start, let alone finish, before SIGKILL lands.
/// Widened to 30s so a well-behaved harness has a realistic window to run
/// that salvage attempt and flush its receipts sidecar
/// (`PERMAGENT_RECEIPTS_SIDECAR`) before being forced dead; a hung or
/// unkillable process is still forced dead exactly as before, just up to
/// 25s later.
const SIGTERM_GRACE: Duration = Duration::from_secs(30);

/// Wait for a child, killing it if it outlives `timeout`. Returns
/// `(exit_code, timed_out)`.
///
/// Lane R17a (M17 readiness audit §6.1, gap 1): on timeout, send SIGTERM
/// first and give the process up to `grace` to exit on its own before
/// falling back to the existing hard SIGKILL (`child.kill()`) below. A
/// well-behaved harness gets a chance to flush its receipts sidecar
/// (`PERMAGENT_RECEIPTS_SIDECAR`) and any other pending I/O; a hung or
/// unkillable one is still forced dead exactly as before — this only adds a
/// grace window, it never weakens the guarantee that the child is gone by
/// the time this returns.
///
/// M21 gate fix: `grace` is a parameter, not a hardcoded read of
/// [`SIGTERM_GRACE`] — both production call sites pass that same constant
/// (unchanged behavior), but
/// `wait_with_timeout_sigkills_a_child_that_ignores_sigterm_after_the_grace`
/// needs an independently short grace: with `SIGTERM_GRACE` now 30s, that
/// test's own child (`sleep 30 & wait`, ignoring SIGTERM) finishes its
/// natural sleep at almost the exact same instant the old hardcoded 30s
/// grace would fire SIGKILL — a flaky race, observed as `Some(0)` (the
/// child's own clean exit) instead of the intended `None` (SIGKILL, which
/// reports no exit code). A real parameter — not an env var, which would
/// leak across Rust's parallel test threads within one process — keeps that
/// test's short grace from ever touching the two production call sites.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
    grace: Duration,
) -> Result<(Option<i32>, bool)> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status.code(), false));
        }
        if start.elapsed() >= timeout {
            send_sigterm(child.id());
            let term_deadline = Instant::now() + grace;
            while Instant::now() < term_deadline {
                if let Some(status) = child.try_wait()? {
                    return Ok((status.code(), true));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Ok((None, true));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Best-effort SIGTERM via the `kill` binary — this crate depends on neither
/// `nix` nor `libc` (see Cargo.toml), and adding one just for a single
/// signal send is not worth it. A failure here (the `kill` binary missing,
/// the process already gone) is silently swallowed: the hard SIGKILL
/// fallback in [`wait_with_timeout`] still guarantees the child is dead
/// before that function returns.
fn send_sigterm(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
}

/// Open a file and a clone for combined stdout+stderr capture.
fn capture_to(log_path: &Path) -> Result<(Stdio, Stdio)> {
    let file = fs::File::create(log_path)
        .with_context(|| format!("creating log {}", log_path.display()))?;
    let err = file.try_clone()?;
    Ok((Stdio::from(file), Stdio::from(err)))
}

/// Apply an invocation's environment policy to a command:
///
/// 1. REMOVE every inherited router-family variable (`PERMAGENT_PACK_*`,
///    `PERMAGENT_CHEAP_*`, `PERMAGENT_BUDGET_*` — see
///    [`crate::invocation::SCRUBBED_ENV_PREFIXES`]) so an operator's shell pins
///    can never contaminate a run. This is what makes `--native-routing`
///    actually native.
/// 2. REMOVE every name in [`Invocation::unset_envs`] — e.g.
///    `PERMAGENT_DISABLE_KEYRING` under `--use-keyring` — so an operator's own
///    shell export of it cannot silently override the invocation's intent.
/// 3. SET the invocation's own variables on top — a pack-pinning tier re-sets
///    exactly its own pins; everything else (API keys, PATH, …) is inherited.
///
/// `ambient_keys` is the inherited environment's variable names, injected so
/// the policy is unit-testable without mutating the test process environment.
fn apply_env_policy(
    cmd: &mut Command,
    inv: &Invocation,
    ambient_keys: impl IntoIterator<Item = OsString>,
) {
    for key in ambient_keys {
        if is_scrubbed_env(&key.to_string_lossy()) {
            cmd.env_remove(&key);
        }
    }
    for name in &inv.unset_envs {
        cmd.env_remove(name);
    }
    for (k, v) in &inv.envs {
        cmd.env(k, v);
    }
}

/// The production harness runner: shells out to the `permagent` binary.
#[derive(Debug, Default, Clone, Copy)]
pub struct SubprocessHarnessRunner;

impl HarnessRunner for SubprocessHarnessRunner {
    fn run(&self, inv: &Invocation, timeout: Duration, log_path: &Path) -> Result<HarnessRun> {
        let (out, err) = capture_to(log_path)?;
        let start = Instant::now();
        let mut cmd = Command::new(&inv.program);
        cmd.args(&inv.args)
            .current_dir(&inv.cwd)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        apply_env_policy(&mut cmd, inv, std::env::vars_os().map(|(k, _)| k));
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning harness `{}`", inv.program))?;
        let (exit, timed_out) = wait_with_timeout(&mut child, timeout, SIGTERM_GRACE)?;
        Ok(HarnessRun {
            exit,
            timed_out,
            duration: start.elapsed(),
        })
    }
}

/// The production oracle runner: runs the test argv in the workspace.
#[derive(Debug, Default, Clone, Copy)]
pub struct SubprocessOracleRunner;

impl OracleRunner for SubprocessOracleRunner {
    fn run(
        &self,
        workdir: &Path,
        argv: &[String],
        envs: &[(String, String)],
        timeout: Duration,
        log_path: &Path,
    ) -> Result<OracleRun> {
        let (program, args) = argv
            .split_first()
            .context("oracle argv must not be empty")?;
        let (out, err) = capture_to(log_path)?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(workdir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning oracle `{program}`"))?;
        let start = Instant::now();
        // Lane K7: keep `timed_out` — a SIGTERM'd process can still exit
        // with a real-looking code during its grace window, and that must
        // not be mistaken for a graded verdict (see `OracleRun`'s doc).
        let (exit, timed_out) = wait_with_timeout(&mut child, timeout, SIGTERM_GRACE)?;
        Ok(OracleRun {
            exit,
            timed_out,
            duration: start.elapsed(),
        })
    }
}

/// The production recipe resolver: if `recipe_name_or_path` names an existing
/// file on disk, read it directly. Otherwise it must be a reserved recipe NAME
/// (e.g. the built-in `permagent-coding`) — shell out ONCE to
/// `<permagent_bin> run --recipe <name> --render-recipe`, which prints that
/// binary's own resolved copy of the recipe (the one that will actually run)
/// to stdout as YAML (`crates/goose-cli/src/recipes/recipe.rs`'s
/// `render_recipe_as_yaml`, wired at `crates/goose-cli/src/cli.rs:1489-1493`).
/// Reading the binary's own copy — rather than embedding a local copy of the
/// built-in recipe in this crate — is what keeps this crate from drifting out
/// of sync with whatever `permagent` version is actually under test.
#[derive(Debug, Default, Clone, Copy)]
pub struct SubprocessRecipeSource;

impl RecipeSource for SubprocessRecipeSource {
    fn base_recipe_yaml(&self, recipe_name_or_path: &str, permagent_bin: &str) -> Result<String> {
        let as_path = Path::new(recipe_name_or_path);
        if as_path.is_file() {
            return fs::read_to_string(as_path)
                .with_context(|| format!("reading recipe file {}", as_path.display()));
        }
        let output = Command::new(permagent_bin)
            .args(["run", "--recipe", recipe_name_or_path, "--render-recipe"])
            .output()
            .with_context(|| format!("rendering recipe `{recipe_name_or_path}`"))?;
        if !output.status.success() {
            anyhow::bail!(
                "rendering recipe `{recipe_name_or_path}` via `{permagent_bin} run --recipe {recipe_name_or_path} --render-recipe` \
                 failed (exit {:?}): {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        String::from_utf8(output.stdout).with_context(|| {
            format!("recipe `{recipe_name_or_path}` --render-recipe output was not UTF-8")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskSpec;

    struct FakeHarness {
        /// File to drop into the workspace, simulating agent output.
        drop_file: (String, String),
        exit: Option<i32>,
    }
    impl HarnessRunner for FakeHarness {
        fn run(&self, inv: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
            fs::write(inv.cwd.join(&self.drop_file.0), &self.drop_file.1)?;
            Ok(HarnessRun {
                exit: self.exit,
                timed_out: false,
                duration: Duration::from_secs_f64(1.5),
            })
        }
    }

    /// A harness stand-in that writes fixed lines to the captured log path —
    /// stands in for a real `permagent run` transcript so `run_task`'s
    /// `harness_log::scan` (and, transitively, `selection_check` evaluation)
    /// has something real to read.
    struct HarnessWithLog {
        log_lines: Vec<&'static str>,
        exit: Option<i32>,
    }

    /// Lane R17a: a harness stand-in that writes BOTH the captured stdout
    /// log AND the receipts sidecar file — the sidecar path is read back
    /// from `inv`'s own environment (`PERMAGENT_RECEIPTS_SIDECAR`) exactly
    /// as a real harness process would receive it, proving `run_task` wires
    /// the env var through to the child and reads the file back afterward.
    struct HarnessWithSidecarAndLog {
        log_lines: Vec<&'static str>,
        sidecar_lines: Vec<&'static str>,
        exit: Option<i32>,
    }
    impl HarnessRunner for HarnessWithSidecarAndLog {
        fn run(&self, inv: &Invocation, _t: Duration, log_path: &Path) -> Result<HarnessRun> {
            fs::write(log_path, self.log_lines.join("\n"))?;
            if let Some(sidecar_path) = inv.env("PERMAGENT_RECEIPTS_SIDECAR") {
                fs::write(sidecar_path, self.sidecar_lines.join("\n"))?;
            }
            Ok(HarnessRun {
                exit: self.exit,
                timed_out: false,
                duration: Duration::from_secs_f64(0.5),
            })
        }
    }
    impl HarnessRunner for HarnessWithLog {
        fn run(&self, _inv: &Invocation, _t: Duration, log_path: &Path) -> Result<HarnessRun> {
            fs::write(log_path, self.log_lines.join("\n"))?;
            Ok(HarnessRun {
                exit: self.exit,
                timed_out: false,
                duration: Duration::from_secs_f64(0.5),
            })
        }
    }

    /// An oracle that passes iff `<workdir>/marker` equals the expected content.
    struct MarkerOracle {
        file: String,
        expect: String,
    }
    impl OracleRunner for MarkerOracle {
        fn run(
            &self,
            workdir: &Path,
            _a: &[String],
            _envs: &[(String, String)],
            _t: Duration,
            _l: &Path,
        ) -> Result<OracleRun> {
            let got = fs::read_to_string(workdir.join(&self.file)).unwrap_or_default();
            Ok(OracleRun {
                exit: Some(if got == self.expect { 0 } else { 1 }),
                timed_out: false,
                duration: Duration::ZERO,
            })
        }
    }

    struct FixedCost(CostReading);
    impl CostReader for FixedCost {
        fn read_total(&self, _data_root: &Path) -> Result<CostReading> {
            Ok(self.0.clone())
        }
    }

    /// A harness whose spawn always hard-errors (after the run dir exists).
    struct BoomHarness;
    impl HarnessRunner for BoomHarness {
        fn run(&self, _i: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
            anyhow::bail!("boom")
        }
    }

    /// Lane K7 regression fixture: an oracle that hit its wall-clock ceiling
    /// but still reports a passing-looking exit code — the A17/M20 shape
    /// (a SIGTERM'd/alarm-truncated build step's own wrapper script folds
    /// the truncation into a plain nonzero `exit`, indistinguishable from a
    /// real failure by exit code alone). `run_task` must trust `timed_out`
    /// over `exit` either way.
    struct TimedOutOracle {
        exit: Option<i32>,
    }
    impl OracleRunner for TimedOutOracle {
        fn run(
            &self,
            _w: &Path,
            _a: &[String],
            _envs: &[(String, String)],
            _t: Duration,
            _l: &Path,
        ) -> Result<OracleRun> {
            Ok(OracleRun {
                exit: self.exit,
                timed_out: true,
                duration: Duration::from_secs(7),
            })
        }
    }

    struct NeverOracle;
    impl OracleRunner for NeverOracle {
        fn run(
            &self,
            _w: &Path,
            _a: &[String],
            _envs: &[(String, String)],
            _t: Duration,
            _l: &Path,
        ) -> Result<OracleRun> {
            Ok(OracleRun {
                exit: Some(0),
                timed_out: false,
                duration: Duration::ZERO,
            })
        }
    }

    struct UnknownCost;
    impl CostReader for UnknownCost {
        fn read_total(&self, _d: &Path) -> Result<CostReading> {
            Ok(CostReading::unknown())
        }
    }

    /// A minimal but valid base recipe, returned regardless of the requested
    /// name/path — stands in for the real subprocess `--render-recipe` call.
    struct FakeRecipe;
    impl RecipeSource for FakeRecipe {
        fn base_recipe_yaml(&self, _name_or_path: &str, _bin: &str) -> Result<String> {
            Ok("title: Fake Recipe\ndescription: d\ninstructions: |\n  do stuff\n".to_string())
        }
    }

    /// A recipe source whose resolution always hard-errors.
    struct BoomRecipe;
    impl RecipeSource for BoomRecipe {
        fn base_recipe_yaml(&self, _name_or_path: &str, _bin: &str) -> Result<String> {
            anyhow::bail!("recipe boom")
        }
    }

    /// Assert the runs root holds no leftover per-run scratch directories.
    fn assert_no_leftover_run_dirs(runs_root: &Path) {
        let leftover: Vec<PathBuf> = fs::read_dir(runs_root)
            .map(|it| it.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        assert!(leftover.is_empty(), "leftover run dirs: {leftover:?}");
    }

    fn make_task(dir: &Path, workspace: &[(&str, &str)], oracle: &[(&str, &str)]) -> Task {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("task.yaml"),
            "id: demo\ntitle: Demo\ncategory: classic\nprompt: do it\ntest: [true]\n",
        )
        .unwrap();
        if !workspace.is_empty() {
            let ws = dir.join("workspace");
            fs::create_dir_all(&ws).unwrap();
            for (name, content) in workspace {
                fs::write(ws.join(name), content).unwrap();
            }
        }
        if !oracle.is_empty() {
            let or = dir.join("oracle");
            fs::create_dir_all(&or).unwrap();
            for (name, content) in oracle {
                fs::write(or.join(name), content).unwrap();
            }
        }
        Task {
            spec: TaskSpec::from_yaml(&fs::read_to_string(dir.join("task.yaml")).unwrap()).unwrap(),
            dir: dir.to_path_buf(),
        }
    }

    /// A `category: selection` task whose `selection_check` requires
    /// `decompose_roadmap` before `create_roadmap` — the G3 D3 "runner
    /// support" case: `run_task` must evaluate this from the harness's own
    /// captured log, not the oracle's verdict.
    fn make_selection_task(dir: &Path) -> Task {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("task.yaml"),
            "id: demo\ntitle: Demo\ncategory: selection\nprompt: do it\ntest: [true]\n\
             selection_check:\n  must_call: [decompose_roadmap, create_roadmap]\n  \
             before: [[decompose_roadmap, create_roadmap]]\n",
        )
        .unwrap();
        Task {
            spec: TaskSpec::from_yaml(&fs::read_to_string(dir.join("task.yaml")).unwrap()).unwrap(),
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn copy_dir_contents_recurses_and_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "A").unwrap();
        fs::write(src.join("sub/b.txt"), "B").unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("a.txt"), "OLD").unwrap();

        copy_dir_contents(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "A");
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "B");
    }

    /// `git log --format=%H` for `dir`, oldest last — used to assert the exact
    /// commit count [`seed_git_repo`] produces.
    fn git_log_shas(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["log", "--format=%H"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Files touched by commit `sha` in `dir`.
    fn git_committed_files(dir: &Path, sha: &str) -> Vec<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["show", "--name-only", "--format=", sha])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git show failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn seed_git_repo_creates_one_commit_with_the_seed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("a.txt"), "A").unwrap();
        fs::write(ws.join("b.txt"), "B").unwrap();

        let sha = seed_git_repo(&ws, "demo-task").unwrap();
        assert_eq!(sha.len(), 40, "expected a full git SHA: {sha}");
        assert!(ws.join(".git").is_dir());

        assert_eq!(
            git_log_shas(&ws),
            vec![sha.clone()],
            "seeding must create exactly one commit"
        );
        let mut files = git_committed_files(&ws, &sha);
        files.sort();
        assert_eq!(files, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn seed_git_repo_second_call_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("a.txt"), "A").unwrap();

        let first = seed_git_repo(&ws, "demo-task").unwrap();
        // A rerun over an already-seeded directory (e.g. `--keep` pointed
        // back at the same scratch dir) must not add a second commit.
        let second = seed_git_repo(&ws, "demo-task").unwrap();
        assert_eq!(first, second, "the second call must return the same HEAD");
        assert_eq!(
            git_log_shas(&ws).len(),
            1,
            "the second call must not create another commit"
        );
    }

    #[test]
    fn seed_git_repo_commits_even_an_empty_workspace() {
        // A task with no `workspace/` seed still gets an (empty) scratch dir,
        // which still needs a git baseline commit for goal dispatch to branch
        // off of.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        fs::create_dir_all(&ws).unwrap();

        let sha = seed_git_repo(&ws, "demo-task").unwrap();
        assert_eq!(git_log_shas(&ws), vec![sha]);
    }

    #[test]
    fn run_task_assembles_result_and_reads_cost() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(
            &tmp.path().join("demo"),
            &[("seed.txt", "s")],
            &[("m", "STRICT")],
        );
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("agent.txt".to_string(), "hi".to_string()),
                exit: Some(0),
            },
            oracle: &MarkerOracle {
                file: "m".to_string(),
                expect: "STRICT".to_string(),
            },
            cost: &FixedCost(CostReading::known(0.42, false, 4)),
            recipe: &FakeRecipe,
        };
        let tier = Tier::builtin("frontier").unwrap();
        let r = run_task(&task, &tier, &cfg, &deps).unwrap();
        assert!(r.solved);
        assert_eq!(r.cost.usd, Some(0.42));
        assert!((r.duration_secs - 1.5).abs() < 1e-9);
        assert_eq!(r.harness_exit, Some(0));
        // The runner seeds a git baseline commit for the harness's internal
        // goal dispatch to branch off of, and records its SHA on the result.
        let seed_commit = r.seed_commit.expect("seed_commit must be recorded");
        assert_eq!(
            seed_commit.len(),
            40,
            "expected a full git SHA: {seed_commit}"
        );
        // Lane K8: `make_task` ships no `seed/harness_state.json`, so
        // fixture DB-state seeding must be a no-op — the overwhelming
        // majority case, and unchanged behavior from before Lane K8.
        assert!(
            !r.seeded_state,
            "a task without seed/harness_state.json must not report seeded_state"
        );
        // Without --keep, the successful run leaves no scratch dir behind.
        assert_no_leftover_run_dirs(&cfg.runs_root);
    }

    /// The whole point of the peer-reported bug fix: `run_task` writes the
    /// per-task recipe file (base recipe + embedded prompt) into the run's
    /// scratch dir, and `--recipe` points at THAT file rather than a bare
    /// name paired with a `-t <prompt>` flag.
    #[test]
    fn run_task_writes_a_recipe_file_with_the_prompt_embedded() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: true, // so the recipe file survives for inspection below
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();

        let run_dir = fs::read_dir(&cfg.runs_root)
            .unwrap()
            .next()
            .expect("a run dir was kept")
            .unwrap()
            .path();
        let recipe_content = fs::read_to_string(run_dir.join("recipe.yaml")).unwrap();
        // FakeRecipe's title survives untouched…
        assert!(
            recipe_content.contains("title: Fake Recipe"),
            "{recipe_content}"
        );
        // …and the task's prompt ("do it", from `make_task`) is embedded as a
        // block-literal `prompt:` key, not passed as `-t`.
        assert!(recipe_content.contains("prompt: |-"), "{recipe_content}");
        assert!(recipe_content.contains("  do it"), "{recipe_content}");
    }

    /// A `RecipeSource` failure (e.g. the `--render-recipe` subprocess call
    /// failing) must error the whole task cleanly — including cleaning up the
    /// scratch dir — rather than falling through to a harness run with a
    /// missing/garbage recipe file.
    #[test]
    fn run_task_errors_and_cleans_up_when_the_recipe_cannot_be_resolved() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &BoomRecipe,
        };
        let err = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap_err();
        assert!(format!("{err:#}").contains("recipe boom"));
        assert_no_leftover_run_dirs(&cfg.runs_root);
    }

    #[test]
    fn oracle_overlay_beats_a_tampered_workspace_copy() {
        // The workspace ships a WEAK grader; the oracle dir ships the STRICT one.
        // The overlay must win, so a run that leaves the weak file in place still
        // gets graded strictly.
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(
            &tmp.path().join("demo"),
            &[("m", "WEAK")],
            &[("m", "STRICT")],
        );
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        // Harness leaves the weak grader untouched.
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &MarkerOracle {
                file: "m".to_string(),
                expect: "STRICT".to_string(),
            },
            cost: &FixedCost(CostReading::unknown()),
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert!(
            r.solved,
            "oracle overlay should have replaced the weak grader"
        );
        assert_eq!(r.cost.usd, None);
    }

    #[test]
    fn run_tier_turns_hard_errors_into_errored_results() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let mut budget = BudgetTracker::unlimited();
        let results = run_tier(
            std::slice::from_ref(&task),
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
            &mut budget,
        );
        assert_eq!(results.len(), 1);
        assert!(!results[0].solved);
        assert_eq!(results[0].oracle, OracleOutcome::Errored);
        assert!(results[0].note.as_deref().unwrap().contains("boom"));
        // The errored-run conversion must not leak the scratch dir either.
        assert_no_leftover_run_dirs(&cfg.runs_root);
    }

    /// Lane K7 (A17/M20 shape): an oracle wrapper script's own internal
    /// alarm truncates a build step and folds that into a plain nonzero
    /// exit — a naive exit-code read would call this `fail`, dragging down
    /// pass-rate for what is actually a grading failure, not a wrong
    /// solution. `timed_out` must force `errored` regardless of what exit
    /// code (if any) came back, and the run's diagnostics must say so.
    #[test]
    fn oracle_timeout_is_errored_not_fail_even_with_a_failing_exit_code() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &TimedOutOracle { exit: Some(142) },
            cost: &FixedCost(CostReading::unknown()),
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(r.oracle, OracleOutcome::Errored);
        assert!(!r.solved);
        assert!(r.oracle_timed_out);
        assert_eq!(r.oracle_exit, Some(142));
        assert_eq!(r.oracle_secs, 7.0);
    }

    /// Same shape, but the truncated step happened to exit 0 (e.g. a
    /// SIGTERM'd process that caught the signal cleanly right as it
    /// finished) — still not a trustworthy `pass`.
    #[test]
    fn oracle_timeout_is_errored_not_pass_even_with_a_zero_exit_code() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &TimedOutOracle { exit: Some(0) },
            cost: &FixedCost(CostReading::unknown()),
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(r.oracle, OracleOutcome::Errored);
        assert!(!r.solved);
        assert!(r.oracle_timed_out);
    }

    /// Lane K5 regression: the zai z1 round (2026-09-14) hit a `run error`
    /// before task 9 ever reached a provider — the harness binary's own
    /// directory had been deleted, so `resolving base recipe` failed and
    /// `run_task` returned `Err` with zero ledger rows. The driver then
    /// printed `BUDGET STOP: unpriced or invalid cost` and skipped the
    /// remaining 10 tasks, though measured spend was nowhere near the cap.
    /// A task that never made a provider call must not be treated as
    /// "spend that cannot be trusted": the run has to keep going.
    #[test]
    fn run_tier_keeps_running_after_a_task_errors_before_any_provider_call() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks: Vec<Task> = ["t1", "t2"]
            .iter()
            .map(|id| make_task(&tmp.path().join(id), &[], &[]))
            .collect();
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let mut budget = BudgetTracker::with_cap(8.00);
        let results = run_tier(
            &tasks,
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
            &mut budget,
        );

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].oracle, OracleOutcome::Errored);
        assert_eq!(
            results[1].oracle,
            OracleOutcome::Errored,
            "the second task must still be launched, not skipped as a budget stop"
        );
        assert!(
            !budget.cap_exceeded(),
            "zero-ledger-row errors must not trip the guard"
        );
        assert!(budget.take_stop_message().is_none());
    }

    #[test]
    fn budget_tracker_trips_at_the_exact_boundary() {
        let mut b = BudgetTracker::with_cap(1.00);
        assert!(!b.cap_exceeded());
        b.record(&CostReading::known(0.50, false, 1));
        assert!(!b.cap_exceeded());
        b.record(&CostReading::known(0.50, false, 1)); // spent == cap: no further task may launch.
        assert!(b.cap_exceeded());
        let message = b.take_stop_message().unwrap();
        assert!(message.contains("reached $1.0000 cap"), "{message}");
    }

    #[test]
    fn budget_tracker_unlimited_never_trips() {
        let mut b = BudgetTracker::unlimited();
        for _ in 0..5 {
            b.record(&CostReading::known(1_000.0, false, 1));
        }
        assert!(!b.cap_exceeded());
        assert!(b.take_stop_message().is_none());
    }

    /// Lane K5: a ledger that DID record a provider call but could not price
    /// it (`ledger_rows > 0`, `usd: None`) is a real accounting problem —
    /// unlike a task that made no calls at all, this must still fail closed.
    #[test]
    fn budget_tracker_unpriced_ledger_row_fails_closed_for_capped_runs() {
        let mut b = BudgetTracker::with_cap(1.0);
        b.record(&CostReading {
            ledger_rows: 1,
            ..CostReading::unknown()
        });
        assert!(
            b.cap_exceeded(),
            "a provider call the ledger could not price cannot authorize another task"
        );
        let message = b.take_stop_message().unwrap();
        assert!(message.contains("unpriced or invalid cost"), "{message}");
        assert!(message.contains("after 1 tasks"), "{message}");
    }

    /// Lane K5 (regression for the zai z1 round, 2026-09-14): a task that
    /// never made a provider call at all — `ledger_rows == 0`, the shape
    /// `CostReading::unknown()` takes when a harness errors or fails to spawn
    /// before any call — contributes $0 and must NOT trip the guard. Zero
    /// calls is not untrustworthy spend.
    #[test]
    fn budget_tracker_errored_task_with_no_ledger_rows_does_not_trip() {
        let mut b = BudgetTracker::with_cap(8.0);
        b.record(&CostReading::unknown());
        assert!(
            !b.cap_exceeded(),
            "an errored task with zero ledger rows spent nothing and must not stop the run"
        );
        assert!(b.take_stop_message().is_none());
    }

    #[test]
    fn budget_tracker_zero_cap_blocks_at_zero_spend() {
        let mut b = BudgetTracker::with_cap(0.0);
        b.record(&CostReading::known(0.0, false, 1));
        assert!(b.cap_exceeded());
    }

    #[test]
    fn budget_tracker_invalid_costs_fail_closed_without_fabricating_spend() {
        let readings = [
            CostReading::known(-1.0, false, 1),
            CostReading::known(f64::NAN, false, 1),
            CostReading::known(f64::INFINITY, false, 1),
            // An unpriced row (not merely an absent ledger) is also untrusted.
            CostReading {
                ledger_rows: 1,
                ..CostReading::unknown()
            },
        ];
        for reading in readings {
            let mut b = BudgetTracker::with_cap(10.0);
            b.record(&reading);
            assert!(
                b.cap_exceeded(),
                "reading {reading:?} must stop a capped run"
            );
            let message = b.take_stop_message().unwrap();
            assert!(message.contains("unpriced or invalid cost"), "{message}");
            assert!(message.contains("$0.0000 of $10.0000 cap"), "{message}");
        }
    }

    #[test]
    fn budget_tracker_invalid_caps_fail_closed_before_launch() {
        for cap in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut b = BudgetTracker::with_cap(cap);
            assert!(b.cap_exceeded(), "cap {cap:?} must fail closed");
            let message = b.take_stop_message().unwrap();
            assert!(message.contains("invalid cap"), "{message}");
            assert!(message.contains("no tasks launched"), "{message}");
            // A later result cannot clear an invalid configuration or move its
            // stop point past zero.
            b.record(&CostReading::known(0.01, false, 1));
            assert!(b.cap_exceeded());
            assert!(b.take_stop_message().is_none());
        }
    }

    #[test]
    fn budget_tracker_repeated_attempts_preserve_first_stop_reason_and_point() {
        let mut b = BudgetTracker::with_cap(10.0);
        b.record(&CostReading {
            ledger_rows: 1,
            ..CostReading::unknown()
        }); // unpriced row: trips right here, at attempt 1.
        b.record(&CostReading::known(1.0, false, 1));
        b.record(&CostReading::known(f64::NAN, false, 1));
        assert!(b.cap_exceeded());
        let message = b.take_stop_message().unwrap();
        assert!(message.contains("after 1 tasks"), "{message}");
        assert!(message.contains("unpriced or invalid cost"), "{message}");
    }

    #[test]
    fn budget_tracker_stop_message_is_handed_out_exactly_once() {
        let mut b = BudgetTracker::with_cap(1.00);
        b.record(&CostReading::known(0.40, false, 1));
        assert!(b.take_stop_message().is_none(), "not tripped yet");
        b.record(&CostReading::known(2.00, false, 1)); // spent 2.40 >= 1.00 cap, trips on attempt 2.
        let msg = b
            .take_stop_message()
            .expect("must produce a message the moment it trips");
        assert_eq!(
            msg,
            "BUDGET STOP: measured spend $2.4000 reached $1.0000 cap after 2 tasks"
        );
        assert!(
            b.take_stop_message().is_none(),
            "must not repeat the message on later calls"
        );
    }

    /// A `StepCost` `CostReader` returning a fixed reading, reused across a
    /// multi-task budget-stop test.
    struct FlatCost(f64);
    impl CostReader for FlatCost {
        fn read_total(&self, _d: &Path) -> Result<CostReading> {
            Ok(CostReading::known(self.0, false, 1))
        }
    }

    #[test]
    fn run_tier_stops_launching_once_the_shared_budget_trips_and_marks_the_rest_not_run() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks: Vec<Task> = ["t1", "t2", "t3"]
            .iter()
            .map(|id| make_task(&tmp.path().join(id), &[], &[]))
            .collect();
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &NeverOracle,
            // $0.60/task: task 1 => spent 0.60 (<1.00), task 2 => spent 1.20
            // (>=1.00, trips), task 3 must be skipped.
            cost: &FlatCost(0.60),
            recipe: &FakeRecipe,
        };
        let mut budget = BudgetTracker::with_cap(1.00);
        let results = run_tier(
            &tasks,
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
            &mut budget,
        );

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].oracle, OracleOutcome::Pass);
        assert_eq!(results[1].oracle, OracleOutcome::Pass);
        assert_eq!(results[2].oracle, OracleOutcome::NotRun);
        assert!(!results[2].solved, "not-run is not a pass");
        assert!(results[2].note.as_deref().unwrap().contains("budget"));
        assert_eq!(results[2].cost.usd, None, "a skipped task spends nothing");

        assert!(budget.cap_exceeded());
        let msg = budget.take_stop_message().unwrap();
        assert!(msg.contains("BUDGET STOP"), "{msg}");
        assert!(msg.contains("after 2 tasks"), "{msg}");
    }

    /// A harness stand-in that serves a different captured log (and exit
    /// code) on each successive call, in call order — lets a `run_tier`
    /// test exercise a MIX of provider-unavailable and healthy tasks
    /// without needing a separate `Deps`/`run_tier` call per task.
    struct SequencedHarness {
        runs: std::cell::RefCell<std::vec::IntoIter<(&'static str, Option<i32>)>>,
    }
    impl SequencedHarness {
        fn new(runs: Vec<(&'static str, Option<i32>)>) -> Self {
            Self {
                runs: std::cell::RefCell::new(runs.into_iter()),
            }
        }
    }
    impl HarnessRunner for SequencedHarness {
        fn run(&self, _inv: &Invocation, _t: Duration, log_path: &Path) -> Result<HarnessRun> {
            let (log_text, exit) = self
                .runs
                .borrow_mut()
                .next()
                .expect("SequencedHarness called more times than it was given runs for");
            fs::write(log_path, log_text)?;
            Ok(HarnessRun {
                exit,
                timed_out: false,
                duration: Duration::from_secs_f64(0.5),
            })
        }
    }

    /// Lane K10 (2026-09-14 M22 zai w1 defect): the M22 zai w1 round's
    /// actual shape — task after task returning the identical billing
    /// error — reproduced against `run_tier` directly. The oracle here
    /// (`NeverOracle`) would otherwise PASS every task, proving the
    /// provider-unavailable override — not a real graded failure — is what
    /// produces `Errored`, and that it wins even when the harness's own
    /// exit code looks fine.
    #[test]
    fn run_tier_stops_after_two_consecutive_provider_unavailable_tasks_and_marks_the_rest_not_run()
    {
        let tmp = tempfile::tempdir().unwrap();
        let tasks: Vec<Task> = ["t1", "t2", "t3"]
            .iter()
            .map(|id| make_task(&tmp.path().join(id), &[], &[]))
            .collect();
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        const MARKER_LINE: &str =
            "{\"error\": \"Credits exhausted: Insufficient balance or no resource package. Please recharge.\"}";
        let deps = Deps {
            harness: &SequencedHarness::new(vec![
                (MARKER_LINE, Some(0)),
                (MARKER_LINE, Some(0)),
                // Never reached: the round stops after the second
                // consecutive provider-unavailable task.
                ("unused", Some(0)),
            ]),
            oracle: &NeverOracle,
            cost: &FixedCost(CostReading::unknown()),
            recipe: &FakeRecipe,
        };
        let mut budget = BudgetTracker::unlimited();
        let results = run_tier(
            &tasks,
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
            &mut budget,
        );

        assert_eq!(results.len(), 3);
        for (i, r) in results.iter().take(2).enumerate() {
            assert_eq!(r.oracle, OracleOutcome::Errored, "task {i}: {r:?}");
            assert!(!r.solved, "task {i}");
            assert_eq!(
                r.provider_error.as_deref(),
                Some("credits exhausted"),
                "task {i}"
            );
            assert_eq!(
                r.note.as_deref(),
                Some("provider unavailable: credits exhausted"),
                "task {i}"
            );
        }
        assert_eq!(results[2].oracle, OracleOutcome::NotRun);
        assert!(!results[2].solved, "not-run is not a pass");
        assert_eq!(
            results[2].note.as_deref(),
            Some("skipped: provider unavailable (credits exhausted)")
        );
        assert!(
            results[2].provider_error.is_none(),
            "a never-launched task observed no provider error of its own"
        );
        // The budget guard is untouched — this is a distinct stop condition.
        assert!(!budget.cap_exceeded());
    }

    /// A single provider-unavailable task, followed by a healthy one, must
    /// NOT stop the round — only two in a row does. The third task (marker
    /// again) actually runs and is graded `Errored`, proving the streak was
    /// reset by task 2's success rather than carried over.
    #[test]
    fn run_tier_a_single_transient_provider_unavailable_task_does_not_stop_the_round() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks: Vec<Task> = ["t1", "t2", "t3"]
            .iter()
            .map(|id| make_task(&tmp.path().join(id), &[], &[]))
            .collect();
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        const MARKER_LINE: &str = "insufficient_quota";
        let deps = Deps {
            harness: &SequencedHarness::new(vec![
                (MARKER_LINE, Some(1)),
                ("all clear", Some(0)),
                (MARKER_LINE, Some(1)),
            ]),
            oracle: &NeverOracle,
            cost: &FixedCost(CostReading::unknown()),
            recipe: &FakeRecipe,
        };
        let mut budget = BudgetTracker::unlimited();
        let results = run_tier(
            &tasks,
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
            &mut budget,
        );

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].oracle, OracleOutcome::Errored);
        assert_eq!(
            results[0].provider_error.as_deref(),
            Some("insufficient_quota")
        );
        assert_eq!(
            results[1].oracle,
            OracleOutcome::Pass,
            "the healthy task ran normally"
        );
        assert!(results[1].provider_error.is_none());
        // Task 3 actually ran (not NotRun) — the streak reset at task 2.
        assert_eq!(results[2].oracle, OracleOutcome::Errored);
        assert_eq!(
            results[2].provider_error.as_deref(),
            Some("insufficient_quota")
        );
    }

    #[test]
    fn scratch_dir_removed_when_the_harness_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[("seed.txt", "s")], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let err = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap_err();
        assert!(format!("{err:#}").contains("boom"));
        assert_no_leftover_run_dirs(&cfg.runs_root);
    }

    #[test]
    fn scratch_dir_kept_on_error_when_keep_is_set() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: true,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap_err();
        let kept = fs::read_dir(&cfg.runs_root).unwrap().count();
        assert_eq!(
            kept, 1,
            "--keep must preserve the scratch dir on errors too"
        );
    }

    /// The F10 regression from the #726 review: a task whose seed cannot be
    /// copied errors AFTER the run dir was created — that dir must not leak.
    #[cfg(unix)]
    #[test]
    fn scratch_dir_removed_when_seeding_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[("seed.txt", "s")], &[]);
        // Replace the seed with a dangling symlink: `fs::copy` of it fails, so
        // workspace seeding errors after `run_dir` exists.
        let seed = task.dir.join("workspace").join("seed.txt");
        fs::remove_file(&seed).unwrap();
        std::os::unix::fs::symlink("/nonexistent/permagent-eval-target", &seed).unwrap();
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let err = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap_err();
        assert!(format!("{err:#}").contains("seeding task workspace"));
        assert_no_leftover_run_dirs(&cfg.runs_root);
    }

    fn demo_spec() -> crate::task::TaskSpec {
        crate::task::TaskSpec::from_yaml("id: demo\ntitle: D\nprompt: p\ntest: ['true']\n").unwrap()
    }

    /// A placeholder recipe path for the `build_invocation` env-policy tests
    /// below, which never touch disk.
    fn recipe_path() -> PathBuf {
        PathBuf::from("/r/recipe.yaml")
    }

    fn env_map(cmd: &Command) -> std::collections::BTreeMap<OsString, Option<OsString>> {
        cmd.get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string())))
            .collect()
    }

    /// The F9 regression from the #726 review: a tier that sets no packs
    /// (`--native-routing`) must produce a command that REMOVES the operator's
    /// pre-existing router env instead of inheriting it.
    #[test]
    fn env_policy_scrubs_operator_router_env_for_a_native_routing_tier() {
        let tier = Tier::builtin("local").unwrap().with_pin_packs(false);
        let inv = build_invocation(
            &demo_spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        let mut cmd = Command::new(&inv.program);
        let ambient = [
            "PERMAGENT_PACK_EDIT_PROVIDER",
            "PERMAGENT_PACK_HARD_MODEL",
            "PERMAGENT_CHEAP_PIN_PROVIDER",
            "PERMAGENT_CHEAP_PIN_MODEL",
            "PERMAGENT_CHEAP_ANCHOR_MODEL",
            "PERMAGENT_BUDGET_TASK_HARD_USD",
            "ANTHROPIC_API_KEY",
            "PATH",
        ]
        .map(OsString::from);
        apply_env_policy(&mut cmd, &inv, ambient);

        let envs = env_map(&cmd);
        // Every pre-existing router-family var is explicitly removed…
        for name in [
            "PERMAGENT_PACK_EDIT_PROVIDER",
            "PERMAGENT_PACK_HARD_MODEL",
            "PERMAGENT_CHEAP_PIN_PROVIDER",
            "PERMAGENT_CHEAP_PIN_MODEL",
            "PERMAGENT_CHEAP_ANCHOR_MODEL",
            "PERMAGENT_BUDGET_TASK_HARD_USD",
        ] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(name)),
                Some(&None),
                "{name} must be removed from the child env"
            );
        }
        // …no pack env is SET at all (native routing means native)…
        assert!(
            envs.iter()
                .all(|(k, v)| v.is_none() || !k.to_string_lossy().starts_with("PERMAGENT_PACK_")),
            "a native-routing run must not set any pack env"
        );
        // …API keys and PATH are untouched (inherited, not removed)…
        assert!(!envs.contains_key(std::ffi::OsStr::new("ANTHROPIC_API_KEY")));
        assert!(!envs.contains_key(std::ffi::OsStr::new("PATH")));
        // …and the run isolation env is still set.
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_PATH_ROOT")),
            Some(&Some(OsString::from("/d")))
        );
    }

    #[test]
    fn env_policy_pinned_tier_sets_exactly_its_own_pack_env() {
        let tier = Tier::builtin("local").unwrap();
        let inv = build_invocation(
            &demo_spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        let mut cmd = Command::new(&inv.program);
        let ambient = [
            "PERMAGENT_PACK_EDIT_PROVIDER",
            "PERMAGENT_PACK_EDIT_MODEL",
            "PERMAGENT_CHEAP_PIN_MODEL",
        ]
        .map(OsString::from);
        apply_env_policy(&mut cmd, &inv, ambient);

        let envs = env_map(&cmd);
        // The tier's own pins win over the scrub of the operator's stale values…
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_PACK_EDIT_PROVIDER")),
            Some(&Some(OsString::from("ollama")))
        );
        // …the SET pack env is exactly the tier's own (all 8 role vars)…
        let set_packs: Vec<String> = envs
            .iter()
            .filter(|(k, v)| v.is_some() && k.to_string_lossy().starts_with("PERMAGENT_PACK_"))
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert_eq!(set_packs.len(), tier.pack_env().len());
        for (k, v) in tier.pack_env() {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(&k)),
                Some(&Some(OsString::from(v.as_str()))),
                "{k} must be pinned to the tier's own value"
            );
        }
        // …and the operator's cheap-tier pin is still removed.
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_CHEAP_PIN_MODEL")),
            Some(&None)
        );
    }

    /// Lane R48a: a MIXED tier's split env (worker pair on `MECHANICAL`/
    /// `LOCAL`, coordinator pair on `HARD`/`EDIT`) actually reaches the child
    /// process `build_invocation`/`apply_env_policy` launch the harness
    /// under — not just `Tier::pack_env()` in isolation.
    #[test]
    fn env_policy_mixed_tier_exports_the_split_pack_env_to_the_harness_process() {
        let tier = Tier::builtin("dschat+minicpm").unwrap();
        let inv = build_invocation(
            &demo_spec(),
            &tier,
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        let mut cmd = Command::new(&inv.program);
        apply_env_policy(&mut cmd, &inv, Vec::<OsString>::new());

        let envs = env_map(&cmd);
        // The main loop (`--provider`/`--model`) stays on the coordinator…
        assert!(inv.args.iter().any(|a| a == "custom_deepseek"));
        assert!(inv.args.iter().any(|a| a == "deepseek-chat"));
        // …the worker pack roles are exported as the LOCAL ollama pair…
        for role in ["MECHANICAL", "LOCAL"] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(&format!(
                    "PERMAGENT_PACK_{role}_PROVIDER"
                ))),
                Some(&Some(OsString::from("ollama"))),
                "{role} provider"
            );
            assert_eq!(
                envs.get(std::ffi::OsStr::new(&format!(
                    "PERMAGENT_PACK_{role}_MODEL"
                ))),
                Some(&Some(OsString::from("minicpm5-2b-q8"))),
                "{role} model"
            );
        }
        // …and HARD/EDIT stay pinned to the coordinator, so the escalation
        // ladder and the review role keep a real, independent frontier rung.
        for role in ["HARD", "EDIT"] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(&format!(
                    "PERMAGENT_PACK_{role}_PROVIDER"
                ))),
                Some(&Some(OsString::from("custom_deepseek"))),
                "{role} provider"
            );
            assert_eq!(
                envs.get(std::ffi::OsStr::new(&format!(
                    "PERMAGENT_PACK_{role}_MODEL"
                ))),
                Some(&Some(OsString::from("deepseek-chat"))),
                "{role} model"
            );
        }
        // R48g (A52) asserted here that `Tier::pack_env()`'s blanket
        // `PERMAGENT_WORKER_PROFILE=compact` (R48f) reached the child. R48i
        // REMOVED that blanket export: the profile is now decided per dispatch
        // from the resolved worker model, and the env var is a manual
        // force-override only. So a mixed tier must NOT export it — the
        // honest inverse of the original assertion, and the same thing
        // `tier.rs`'s own R48i test now checks on the producing side.
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_WORKER_PROFILE")),
            None,
            "R48i: the worker profile is per-dispatch; the env var is a force-override, \
             so a mixed tier must not export it"
        );
    }

    /// An A/B arm's knobs must actually reach the child, must survive the
    /// ambient scrub, and must OUTRANK the invocation's own settings — a knob
    /// that could be quietly overridden would make both arms identical while
    /// still printing a delta.
    #[test]
    fn arm_env_reaches_the_child_and_outranks_the_invocations_own_value() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            // GOOSE_MODE is set by `build_invocation`; PERMAGENT_PACK_EDIT_MODEL
            // by the pinned tier; PERMAGENT_BUDGET_X is a scrubbed prefix.
            arm_env: vec![
                ("PERMAGENT_BEST_OF_N".to_string(), "3".to_string()),
                ("GOOSE_MODE".to_string(), "chat".to_string()),
                ("PERMAGENT_BUDGET_X".to_string(), "1".to_string()),
            ],
            arm_unset: vec!["SOME_AMBIENT_VAR".to_string()],
            harness_timeout_secs: None,
        };
        // Capture the invocation the runner hands to the harness.
        struct Capture(std::sync::Mutex<Option<Invocation>>);
        impl HarnessRunner for Capture {
            fn run(&self, inv: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
                *self.0.lock().unwrap() = Some(inv.clone());
                Ok(HarnessRun {
                    exit: Some(0),
                    timed_out: false,
                    duration: Duration::from_secs(1),
                })
            }
        }
        let capture = Capture(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &capture,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        let inv = capture.0.lock().unwrap().clone().unwrap();

        assert_eq!(inv.env("PERMAGENT_BEST_OF_N"), Some("3"));
        assert_eq!(
            inv.env("GOOSE_MODE"),
            Some("chat"),
            "the arm must win over the invocation's own GOOSE_MODE"
        );
        assert!(inv.unset_envs.iter().any(|n| n == "SOME_AMBIENT_VAR"));

        // …and once the env policy runs, the arm's value is what the process
        // gets, even for a name inside a scrubbed prefix.
        let mut cmd = Command::new(&inv.program);
        apply_env_policy(
            &mut cmd,
            &inv,
            ["PERMAGENT_BUDGET_X", "SOME_AMBIENT_VAR"].map(OsString::from),
        );
        let envs = env_map(&cmd);
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_BUDGET_X")),
            Some(&Some(OsString::from("1"))),
            "an arm knob must survive the ambient scrub"
        );
        assert_eq!(
            envs.get(std::ffi::OsStr::new("SOME_AMBIENT_VAR")),
            Some(&None)
        );
    }

    // --- Lane R18c (A17 Class 3): CARGO_TARGET_DIR pinned per run ---------

    /// Both the harness child's invocation AND the oracle child's env must
    /// carry a `CARGO_TARGET_DIR` scoped to THIS run's own scratch
    /// directory (not the ambient process environment) — and both must
    /// agree on the SAME path, since they build/test the same workspace.
    #[test]
    fn cargo_target_dir_reaches_both_the_harness_and_the_oracle_child() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };

        struct CaptureHarness(std::sync::Mutex<Option<Invocation>>);
        impl HarnessRunner for CaptureHarness {
            fn run(&self, inv: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
                *self.0.lock().unwrap() = Some(inv.clone());
                Ok(HarnessRun {
                    exit: Some(0),
                    timed_out: false,
                    duration: Duration::from_secs(1),
                })
            }
        }
        struct CaptureOracle(std::sync::Mutex<Option<Vec<(String, String)>>>);
        impl OracleRunner for CaptureOracle {
            fn run(
                &self,
                _w: &Path,
                _a: &[String],
                envs: &[(String, String)],
                _t: Duration,
                _l: &Path,
            ) -> Result<OracleRun> {
                *self.0.lock().unwrap() = Some(envs.to_vec());
                Ok(OracleRun {
                    exit: Some(0),
                    timed_out: false,
                    duration: Duration::ZERO,
                })
            }
        }

        let harness = CaptureHarness(std::sync::Mutex::new(None));
        let oracle = CaptureOracle(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &harness,
            oracle: &oracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();

        let inv = harness.0.lock().unwrap().clone().unwrap();
        let harness_target_dir = inv
            .env("CARGO_TARGET_DIR")
            .expect("harness invocation must carry CARGO_TARGET_DIR")
            .to_string();

        let oracle_envs = oracle.0.lock().unwrap().clone().unwrap();
        let oracle_target_dir = oracle_envs
            .iter()
            .find(|(k, _)| k == "CARGO_TARGET_DIR")
            .map(|(_, v)| v.clone())
            .expect("oracle envs must carry CARGO_TARGET_DIR");

        assert_eq!(
            harness_target_dir, oracle_target_dir,
            "the harness and the oracle must build/test the SAME workspace \
             into the SAME target dir"
        );
        // Scoped to THIS run — under `runs_root`, not the ambient
        // environment or a fixed/shared path like the host repo's own
        // `target/`.
        assert!(
            Path::new(&harness_target_dir).starts_with(&cfg.runs_root),
            "{harness_target_dir} must be under {}",
            cfg.runs_root.display()
        );
        assert!(harness_target_dir.ends_with("cargo-target"));
    }

    #[test]
    fn cargo_target_dir_is_a_subdirectory_of_the_run_dir() {
        let run_dir = Path::new("/tmp/runs/local-fizzbuzz-123");
        assert_eq!(cargo_target_dir(run_dir), run_dir.join("cargo-target"));
    }

    // --- Lane K3: headless auto-land turned on explicitly by the eval -----

    #[test]
    fn headless_drain_secs_subtracts_a_minute_of_margin_and_floors_at_60() {
        assert_eq!(headless_drain_secs(900), 840);
        assert_eq!(headless_drain_secs(120), 60);
        // Below the floor: 90 - 60 = 30, which is under the 60s floor.
        assert_eq!(headless_drain_secs(90), 60);
        assert_eq!(headless_drain_secs(60), 60);
        // Shorter than the margin itself: saturating_sub must not go negative.
        assert_eq!(headless_drain_secs(10), 60);
        assert_eq!(headless_drain_secs(0), 60);
    }

    /// The user's ruling (option 3): the eval is the "deliberate unattended run"
    /// that turns headless auto-land on explicitly. Every task run must set
    /// both env vars on the child, with the drain window computed from that
    /// task's own `harness_timeout_secs`, and both values must also land on
    /// the per-task report row.
    #[test]
    fn run_task_sets_headless_autoland_and_the_computed_drain_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        // No `harness_timeout_secs` override in `make_task`'s task.yaml, so
        // the default (900s) applies: drain = 900 - 60 = 840.
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        struct Capture(std::sync::Mutex<Option<Invocation>>);
        impl HarnessRunner for Capture {
            fn run(&self, inv: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
                *self.0.lock().unwrap() = Some(inv.clone());
                Ok(HarnessRun {
                    exit: Some(0),
                    timed_out: false,
                    duration: Duration::from_secs(1),
                })
            }
        }
        let capture = Capture(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &capture,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        let inv = capture.0.lock().unwrap().clone().unwrap();

        assert_eq!(inv.env("PERMAGENT_HEADLESS_AUTOLAND"), Some("1"));
        assert_eq!(inv.env("PERMAGENT_HEADLESS_DRAIN_SECS"), Some("840"));
        // …and the report row records exactly what the child received.
        assert_eq!(result.headless_autoland.as_deref(), Some("1"));
        assert_eq!(result.headless_drain_secs.as_deref(), Some("840"));
        // Lane K6: the resolved ceiling that produced these numbers is
        // retained on the result too.
        assert_eq!(result.harness_timeout_secs, Some(900));
    }

    /// Lane R60a: the child's daemon bridge (`brain_sync` recall, turn
    /// persistence, spend announce) resolves its bearer token and port through
    /// `PERMAGENT_PATH_ROOT`, and fails closed when the root holds no token.
    /// That is the whole isolation from the operator's live daemon and private
    /// Brain — so the root this runner hands the child must be run-scoped and
    /// must never be seeded with a daemon credential or the operator's config
    /// (which carries the daemon port). Checked at spawn time, from the env the
    /// child actually receives.
    #[test]
    fn run_task_hands_the_child_a_root_with_no_daemon_credential() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        struct InspectRoot(std::sync::Mutex<Option<(PathBuf, bool, bool, bool)>>);
        impl HarnessRunner for InspectRoot {
            fn run(&self, inv: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
                let root = PathBuf::from(inv.env("PERMAGENT_PATH_ROOT").expect("root is set"));
                *self.0.lock().unwrap() = Some((
                    root.clone(),
                    root.is_dir(),
                    root.join("secrets").join("daemon_token.json").exists(),
                    root.join("config.yaml").exists(),
                ));
                Ok(HarnessRun {
                    exit: Some(0),
                    timed_out: false,
                    duration: Duration::from_secs(1),
                })
            }
        }
        let inspect = InspectRoot(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &inspect,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        let (root, exists, has_token, has_config) = inspect.0.lock().unwrap().clone().unwrap();

        assert!(exists, "the child's root must exist at spawn time");
        assert!(
            root.starts_with(&cfg.runs_root),
            "the child's root must be run-scoped, not the operator's: {}",
            root.display()
        );
        assert!(
            !has_token,
            "a daemon token in the run root would reconnect the child to a live daemon's Brain"
        );
        assert!(
            !has_config,
            "the operator's config (daemon port) must not be seeded into the run root"
        );
    }

    // --- Lane K6 / K6-b: per-tier harness timeout floor, retained in the
    // record -----------------------------------------------------------

    /// A harness stand-in that records the `timeout` it was invoked with, so
    /// a test can assert `run_task` actually handed the RESOLVED ceiling to
    /// the subprocess wait — not just derived the drain-secs env from it.
    struct CaptureTimeout(std::sync::Mutex<Option<Duration>>);
    impl HarnessRunner for CaptureTimeout {
        fn run(&self, _inv: &Invocation, t: Duration, _l: &Path) -> Result<HarnessRun> {
            *self.0.lock().unwrap() = Some(t);
            Ok(HarnessRun {
                exit: Some(0),
                timed_out: false,
                duration: Duration::from_secs(1),
            })
        }
    }

    /// A task with no `harness_timeout_secs:` of its own, run under the
    /// `glm47` tier (900s floor — see `Tier::builtin`), must resolve to the
    /// TIER's floor rather than `DEFAULT_HARNESS_TIMEOUT_SECS` (also 900s
    /// here, so this alone would not distinguish the two — the precedence
    /// tests below do that; this one just pins the wiring: the resolved
    /// value reaches the subprocess timeout, the derived drain-secs env,
    /// AND the retained record).
    #[test]
    fn run_task_applies_the_tier_default_harness_timeout_when_task_has_none() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let harness = CaptureTimeout(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &harness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("glm47").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(harness.0.lock().unwrap().unwrap(), Duration::from_secs(900));
        assert_eq!(result.harness_timeout_secs, Some(900));
        assert_eq!(result.headless_drain_secs.as_deref(), Some("840"));
    }

    /// Lane K6-b: the tier's own floor (glm47's 900s) beats a TIGHTER
    /// task-level cap (600s here) — `max(600, 900) = 900`. This is exactly
    /// the case Lane K6's evidence found (zai's z2/z3 rounds on M19 hit the
    /// driver's flat 600s ceiling on two fixtures whose task.yaml caps them
    /// tighter than zai needs) — under the K6-b floor it now runs with room
    /// to spare, with no `--harness-timeout-secs` override required.
    #[test]
    fn run_task_tier_floor_beats_a_tighter_task_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("task.yaml"),
            "id: demo\ntitle: Demo\ncategory: classic\nprompt: do it\ntest: [true]\n\
             harness_timeout_secs: 600\n",
        )
        .unwrap();
        let task = Task {
            spec: TaskSpec::from_yaml(&fs::read_to_string(dir.join("task.yaml")).unwrap()).unwrap(),
            dir,
        };
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let harness = CaptureTimeout(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &harness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("glm47").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(harness.0.lock().unwrap().unwrap(), Duration::from_secs(900));
        assert_eq!(result.harness_timeout_secs, Some(900));
        assert_eq!(result.headless_drain_secs.as_deref(), Some("840"));
    }

    /// Lane K6-b: a task-level cap LOOSER than the tier's floor (1200s here
    /// vs glm47's 900s) is left alone — `max(1200, 900) = 1200`. The tier
    /// value is a floor, never a shortening override.
    #[test]
    fn run_task_a_looser_task_cap_beats_the_tier_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("task.yaml"),
            "id: demo\ntitle: Demo\ncategory: classic\nprompt: do it\ntest: [true]\n\
             harness_timeout_secs: 1200\n",
        )
        .unwrap();
        let task = Task {
            spec: TaskSpec::from_yaml(&fs::read_to_string(dir.join("task.yaml")).unwrap()).unwrap(),
            dir,
        };
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            ..Default::default()
        };
        let harness = CaptureTimeout(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &harness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("glm47").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(
            harness.0.lock().unwrap().unwrap(),
            Duration::from_secs(1200)
        );
        assert_eq!(result.harness_timeout_secs, Some(1200));
        assert_eq!(result.headless_drain_secs.as_deref(), Some("1140"));
    }

    /// Precedence: `RunConfig::harness_timeout_secs` (the `--harness-
    /// timeout-secs` CLI flag) beats BOTH the task's own cap (600s) AND the
    /// tier's floor (glm47's 900s) — 700s here is below the tier floor, so
    /// this also proves the CLI override is not itself floored by the tier;
    /// it is the knob Lane K6/K6-b adds so one invocation can pin an EXACT
    /// ceiling regardless of task/tier.
    #[test]
    fn run_task_cli_override_beats_both_task_cap_and_tier_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("task.yaml"),
            "id: demo\ntitle: Demo\ncategory: classic\nprompt: do it\ntest: [true]\n\
             harness_timeout_secs: 600\n",
        )
        .unwrap();
        let task = Task {
            spec: TaskSpec::from_yaml(&fs::read_to_string(dir.join("task.yaml")).unwrap()).unwrap(),
            dir,
        };
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            harness_timeout_secs: Some(700),
            ..Default::default()
        };
        let harness = CaptureTimeout(std::sync::Mutex::new(None));
        let deps = Deps {
            harness: &harness,
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("glm47").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(harness.0.lock().unwrap().unwrap(), Duration::from_secs(700));
        assert_eq!(result.harness_timeout_secs, Some(700));
        assert_eq!(result.headless_drain_secs.as_deref(), Some("640"));
    }

    /// An A/B arm must be able to override either headless-autoland default —
    /// e.g. to measure a run with landing left OFF as a control arm — and the
    /// override must be what both the child and the report row see.
    #[test]
    fn arm_env_overrides_the_headless_autoland_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: vec![
                ("PERMAGENT_HEADLESS_AUTOLAND".to_string(), "0".to_string()),
                (
                    "PERMAGENT_HEADLESS_DRAIN_SECS".to_string(),
                    "77".to_string(),
                ),
            ],
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let result = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(
            result.headless_autoland.as_deref(),
            Some("0"),
            "an arm override must win over the eval's own default"
        );
        assert_eq!(result.headless_drain_secs.as_deref(), Some("77"));
    }

    /// `--use-keyring` must not just omit setting `PERMAGENT_DISABLE_KEYRING`
    /// — it must also strip an operator's own shell export of it, or the
    /// export would silently defeat the flag.
    #[test]
    fn env_policy_removes_an_operator_exported_disable_keyring_var_under_use_keyring() {
        let inv = build_invocation(
            &demo_spec(),
            &Tier::builtin("local").unwrap(),
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            true,
        );
        let mut cmd = Command::new(&inv.program);
        let ambient = ["PERMAGENT_DISABLE_KEYRING", "ANTHROPIC_API_KEY"].map(OsString::from);
        apply_env_policy(&mut cmd, &inv, ambient);

        let envs = env_map(&cmd);
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_DISABLE_KEYRING")),
            Some(&None),
            "the operator's export must be explicitly removed"
        );
        assert!(!envs.contains_key(std::ffi::OsStr::new("ANTHROPIC_API_KEY")));
    }

    /// Without `--use-keyring`, today's behaviour is unchanged: the var is SET
    /// (not merely left alone), and nothing is removed.
    #[test]
    fn env_policy_sets_disable_keyring_without_use_keyring() {
        let inv = build_invocation(
            &demo_spec(),
            &Tier::builtin("local").unwrap(),
            Path::new("/w"),
            Path::new("/d"),
            &recipe_path(),
            "permagent",
            false,
        );
        let mut cmd = Command::new(&inv.program);
        apply_env_policy(&mut cmd, &inv, std::iter::empty());

        let envs = env_map(&cmd);
        assert_eq!(
            envs.get(std::ffi::OsStr::new("PERMAGENT_DISABLE_KEYRING")),
            Some(&Some(OsString::from("1")))
        );
    }

    // ── G3: selection_correct is evaluated from the harness's own log ──────

    #[test]
    fn run_task_evaluates_selection_correct_from_the_captured_log() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_selection_task(&tmp.path().join("demo"));
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            harness: &HarnessWithLog {
                log_lines: vec!["  ▸ decompose_roadmap", "  ▸ create_roadmap"],
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(r.selection_correct, Some(true));
        assert!(r.selection_check_failures.is_empty());
        assert_eq!(
            r.signals.tool_names,
            vec![
                "decompose_roadmap".to_string(),
                "create_roadmap".to_string()
            ]
        );
    }

    #[test]
    fn run_task_marks_selection_correct_false_on_the_wrong_decision() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_selection_task(&tmp.path().join("demo"));
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            // Wrong order: create_roadmap before decompose_roadmap.
            harness: &HarnessWithLog {
                log_lines: vec!["  ▸ create_roadmap", "  ▸ decompose_roadmap"],
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(r.selection_correct, Some(false));
        assert!(!r.selection_check_failures.is_empty());
    }

    // ── Lane R17a: receipts survive a timeout via the sidecar fallback ────

    /// A harness that gets SIGKILLed before its clean-exit print never gets
    /// to write ANY `receipt ...`/`harness_overhead ...` line to stdout —
    /// simulated here by a captured log with no such lines at all — but it
    /// DID keep the sidecar file up to date throughout the run. `run_task`
    /// must read that fallback and mark the source `"sidecar"`.
    #[test]
    fn run_task_fills_receipts_from_the_sidecar_when_stdout_has_none() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            harness: &HarnessWithSidecarAndLog {
                log_lines: vec!["  ▸ shell", "(killed before it could print receipts)"],
                sidecar_lines: vec![
                    "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0",
                    "receipt verification_integrity goals=1 review_reached=1 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0",
                    "receipt safety_and_trust jail_denials=0 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=0",
                    "receipt cost_routing dispatches=1 cheapest_capable=1 pack_honored=1 planner=openrouter/glm",
                    "harness_overhead p50=10 p95=20 model=30 tools=40 wall=50",
                ],
                exit: None,
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();

        assert_eq!(r.receipt_source, Some("sidecar".to_string()));
        assert_eq!(r.signals.dag_enforcement.unwrap().roadmap_dispatched, 1);
        assert_eq!(r.signals.verification_integrity.unwrap().goals, 1);
        assert_eq!(r.signals.safety_and_trust.unwrap().jail_denials, 0);
        assert_eq!(r.signals.cost_routing.unwrap().dispatches, 1);
        assert_eq!(r.signals.harness_overhead.unwrap().p50_ms, 10);
    }

    /// A clean exit that already printed its own receipts to stdout must
    /// keep those values — the sidecar (deliberately carrying DIFFERENT
    /// numbers here) must never override an authoritative stdout print, and
    /// the recorded source must read `"stdout"`.
    #[test]
    fn run_task_prefers_stdout_receipts_over_the_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            harness: &HarnessWithSidecarAndLog {
                log_lines: vec![
                    "receipt dag_enforcement roadmap_dispatched=9 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0",
                    "receipt verification_integrity goals=9 review_reached=9 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0",
                    "receipt safety_and_trust jail_denials=9 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=0",
                    "receipt cost_routing dispatches=9 cheapest_capable=9 pack_honored=9 planner=z.ai/glm",
                    "harness_overhead p50=99 p95=99 model=99 tools=99 wall=99",
                ],
                // The sidecar was still written (it always is while a run is
                // live) but with different numbers, from earlier in the same
                // run — it must lose to the final stdout print.
                sidecar_lines: vec![
                    "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0",
                    "harness_overhead p50=1 p95=1 model=1 tools=1 wall=1",
                ],
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();

        assert_eq!(r.receipt_source, Some("stdout".to_string()));
        assert_eq!(
            r.signals.dag_enforcement.unwrap().roadmap_dispatched,
            9,
            "stdout's own value must survive, not the sidecar's"
        );
        assert_eq!(r.signals.harness_overhead.unwrap().p50_ms, 99);
    }

    // ── Lane R17b (G20 review MED-2): SIGTERM-before-SIGKILL escalation ───
    //
    // Both tests use the `sleep N & wait` shell idiom rather than a bare
    // foreground `sleep N`. Verified by hand against this machine's `/bin/sh`
    // before writing these: a bare foreground `sleep 30` defers a TERM trap
    // until the `sleep` itself finishes (~29.5s observed) — POSIX shells only
    // check for pending traps between commands / at syscall-interrupt points
    // that `wait` provides but a foreground child's own blocking wait4() does
    // not. Backgrounding `sleep` and blocking the shell in its OWN `wait`
    // builtin makes the trap fire within milliseconds of the signal, which is
    // what these tests need to tell "SIGTERM path" apart from "grace timeout
    // path" reliably rather than by accident of a fast machine.

    /// A child that traps SIGTERM and exits cleanly must be reaped during the
    /// grace window — `wait_with_timeout` must return promptly, nowhere near
    /// waiting out the full `SIGTERM_GRACE`, let alone the child's own
    /// (deliberately much longer) sleep.
    #[test]
    fn wait_with_timeout_reaps_a_child_that_traps_sigterm_within_the_grace() {
        let mut child = Command::new("sh")
            .args(["-c", "trap 'exit 0' TERM; sleep 30 & wait"])
            .spawn()
            .unwrap();
        let start = Instant::now();
        let (exit, timed_out) =
            wait_with_timeout(&mut child, Duration::from_millis(300), SIGTERM_GRACE).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(exit, Some(0));
        assert!(timed_out);
        assert!(
            elapsed < SIGTERM_GRACE,
            "took {elapsed:?}; a trapping child should be reaped well before \
             the {SIGTERM_GRACE:?} grace elapses, not right at its edge"
        );
    }

    /// A child that ignores SIGTERM must still be dead by the time this
    /// returns, via the existing hard SIGKILL fallback — but only AFTER
    /// waiting out the full grace window first, never sooner.
    ///
    /// M21 gate fix: uses a short, test-local `grace` (2s) rather than the
    /// production `SIGTERM_GRACE` (30s) — with the child's own `sleep 30`
    /// and a 30s production grace, this test raced its own child's natural,
    /// clean exit against the SIGKILL deadline (both ~30s), observed as a
    /// flaky `Some(0)` instead of the intended `None`. The child's sleep
    /// (10s) still comfortably outlasts the short grace, so the kill path
    /// this test exists to exercise is still the one that fires.
    #[test]
    fn wait_with_timeout_sigkills_a_child_that_ignores_sigterm_after_the_grace() {
        let grace = Duration::from_secs(2);
        let mut child = Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 10 & wait"])
            .spawn()
            .unwrap();
        let start = Instant::now();
        let (exit, timed_out) =
            wait_with_timeout(&mut child, Duration::from_millis(300), grace).unwrap();
        let elapsed = start.elapsed();

        // SIGKILL cannot be trapped or reported: no exit code survives it.
        assert_eq!(exit, None);
        assert!(timed_out);
        assert!(
            elapsed >= grace,
            "took {elapsed:?}; must not escalate to SIGKILL before the full {grace:?} grace \
             has elapsed"
        );
        // ...but nowhere near the child's own 10s sleep — proves SIGKILL
        // actually fired rather than this falling through to waiting the
        // child out directly.
        assert!(
            elapsed < Duration::from_secs(5),
            "took {elapsed:?}; expected well under the child's own 10s sleep"
        );
    }

    /// An ordinary (non-selection) fixture must never carry a
    /// `selection_correct` verdict, regardless of what the log contains.
    #[test]
    fn run_task_leaves_selection_correct_none_for_a_non_selection_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
            arm_env: Vec::new(),
            arm_unset: Vec::new(),
            harness_timeout_secs: None,
        };
        let deps = Deps {
            harness: &HarnessWithLog {
                log_lines: vec!["  ▸ decompose_roadmap", "  ▸ create_roadmap"],
                exit: Some(0),
            },
            oracle: &NeverOracle,
            cost: &UnknownCost,
            recipe: &FakeRecipe,
        };
        let r = run_task(&task, &Tier::builtin("local").unwrap(), &cfg, &deps).unwrap();
        assert_eq!(r.selection_correct, None);
        assert!(r.selection_check_failures.is_empty());
    }
}
