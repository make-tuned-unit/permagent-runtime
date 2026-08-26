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

use anyhow::{Context, Result};
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
use crate::oracle::{outcome_from_exit, OracleOutcome};
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

/// Runs a task's test oracle in the finished workspace.
pub trait OracleRunner {
    /// Returns the oracle process exit code (`None` = killed / timed out).
    fn run(
        &self,
        workdir: &Path,
        argv: &[String],
        timeout: Duration,
        log_path: &Path,
    ) -> Result<Option<i32>>;
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
#[derive(Debug, Clone)]
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

    // Seed the workspace the agent will see and edit.
    if task.has_workspace() {
        copy_dir_contents(&task.workspace_dir(), &workdir).context("seeding task workspace")?;
    }

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

    let inv = build_invocation(
        &task.spec,
        tier,
        &workdir,
        &data_root,
        &recipe_path,
        &cfg.permagent_bin,
        cfg.use_keyring,
    );
    let harness_log_path = logs.join("harness.log");
    let harness_run = deps.harness.run(
        &inv,
        Duration::from_secs(task.spec.harness_timeout_secs()),
        &harness_log_path,
    )?;
    // Best-effort: a missing/unreadable log just yields zeroed signals rather
    // than failing the whole task (the oracle's verdict is authoritative).
    let signals = fs::read_to_string(&harness_log_path)
        .map(|text| harness_log::scan(&text))
        .unwrap_or_default();

    // Tamper-proof grading: overlay the pristine oracle files onto the workspace
    // so the agent cannot have weakened or deleted its own grader.
    if task.has_oracle() {
        copy_dir_contents(&task.oracle_dir(), &workdir).context("overlaying oracle files")?;
    }

    let oracle_code = deps.oracle.run(
        &workdir,
        &task.spec.test,
        Duration::from_secs(task.spec.oracle_timeout_secs()),
        &logs.join("oracle.log"),
    )?;
    let outcome = outcome_from_exit(oracle_code);

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
    result.signals = signals;
    if harness_run.timed_out {
        result.note = Some("harness run hit its wall-clock ceiling".to_string());
    }

    // `_scratch` cleans up the run dir here (and on every early return above).
    Ok(result)
}

/// Tracks cumulative MEASURED spend (from the ledger, not an estimate) across
/// an eval session that may span many tiers and tasks, against an optional
/// `--budget-usd` cap, and decides when the sweep must stop launching further
/// tasks. Pure and side-effect free — the caller decides what to do with a
/// tripped cap (skip the next task, print the stop line).
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetTracker {
    cap_usd: Option<f64>,
    spent_usd: f64,
    attempted: usize,
    /// Set the first time `spent_usd` exceeds `cap_usd`, to the attempted
    /// count at that moment. Stays `Some` for the rest of the session.
    stopped_after: Option<usize>,
    /// Whether [`Self::take_stop_message`] has already handed out its one
    /// message for this trip.
    announced: bool,
}

impl BudgetTracker {
    /// No cap: the sweep never stops early.
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Stop once cumulative measured spend exceeds `cap_usd`.
    pub fn with_cap(cap_usd: f64) -> Self {
        Self {
            cap_usd: Some(cap_usd),
            ..Self::default()
        }
    }

    /// Whether the cap has already tripped — i.e. whether the NEXT task
    /// should be skipped (marked not-run) rather than launched.
    pub fn cap_exceeded(&self) -> bool {
        self.stopped_after.is_some()
    }

    /// Record one attempted task's measured spend. An unknown (`None`) cost
    /// counts as $0 spent but still as an attempt. Trips the stop the first
    /// time cumulative spend exceeds the cap.
    pub fn record(&mut self, cost_usd: Option<f64>) {
        self.attempted += 1;
        self.spent_usd += cost_usd.unwrap_or(0.0);
        if self.stopped_after.is_none() {
            if let Some(cap) = self.cap_usd {
                if self.spent_usd > cap {
                    self.stopped_after = Some(self.attempted);
                }
            }
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
        Some(format!(
            "BUDGET STOP: spent ${:.4} of ${:.4} cap after {n} tasks",
            self.spent_usd, cap
        ))
    }
}

/// Build a [`TaskResult`] for a task that was never launched because the
/// session's `--budget-usd` cap had already been exceeded. A distinct
/// [`OracleOutcome::NotRun`] state — NOT a fail — so pass-rate (computed over
/// attempted tasks only) is not dragged down by tasks that simply never ran.
fn not_run_result(task: &Task) -> TaskResult {
    let mut r = TaskResult::new(
        task.spec.id.as_str(),
        task.spec.category.as_str(),
        OracleOutcome::NotRun,
        CostReading::unknown(),
    );
    r.note = Some("skipped: --budget-usd cap reached".to_string());
    r
}

/// Run every task in `tasks` under `tier`, turning a per-task hard error into an
/// errored result so one failure never aborts the whole tier. `budget` is
/// shared across every tier in the session (the caller passes the same
/// tracker to each `run_tier` call): once its cap trips, every remaining task
/// — in this tier and any later one — is recorded as
/// [`OracleOutcome::NotRun`] instead of being launched.
pub fn run_tier(
    tasks: &[Task],
    tier: &Tier,
    cfg: &RunConfig,
    deps: &Deps<'_>,
    budget: &mut BudgetTracker,
) -> Vec<TaskResult> {
    tasks
        .iter()
        .map(|task| {
            if budget.cap_exceeded() {
                return not_run_result(task);
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
            budget.record(result.cost.usd);
            result
        })
        .collect()
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

/// Wait for a child, killing it if it outlives `timeout`. Returns
/// `(exit_code, timed_out)`.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<(Option<i32>, bool)> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status.code(), false));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((None, true));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
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
        let (exit, timed_out) = wait_with_timeout(&mut child, timeout)?;
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
        timeout: Duration,
        log_path: &Path,
    ) -> Result<Option<i32>> {
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
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning oracle `{program}`"))?;
        let (exit, _timed_out) = wait_with_timeout(&mut child, timeout)?;
        Ok(exit)
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
            _t: Duration,
            _l: &Path,
        ) -> Result<Option<i32>> {
            let got = fs::read_to_string(workdir.join(&self.file)).unwrap_or_default();
            Ok(Some(if got == self.expect { 0 } else { 1 }))
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

    struct NeverOracle;
    impl OracleRunner for NeverOracle {
        fn run(&self, _w: &Path, _a: &[String], _t: Duration, _l: &Path) -> Result<Option<i32>> {
            Ok(Some(0))
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

    #[test]
    fn budget_tracker_trips_only_once_spend_strictly_exceeds_the_cap() {
        let mut b = BudgetTracker::with_cap(1.00);
        assert!(!b.cap_exceeded());
        b.record(Some(0.50));
        assert!(!b.cap_exceeded());
        b.record(Some(0.50)); // spent == cap: reaching it is not exceeding it.
        assert!(!b.cap_exceeded());
        b.record(Some(0.01)); // spent > cap: now it trips.
        assert!(b.cap_exceeded());
    }

    #[test]
    fn budget_tracker_unlimited_never_trips() {
        let mut b = BudgetTracker::unlimited();
        for _ in 0..5 {
            b.record(Some(1_000.0));
        }
        assert!(!b.cap_exceeded());
        assert!(b.take_stop_message().is_none());
    }

    #[test]
    fn budget_tracker_treats_unknown_cost_as_zero_spend_but_still_an_attempt() {
        let mut b = BudgetTracker::with_cap(0.0);
        b.record(None);
        assert!(!b.cap_exceeded(), "$0 spent does not exceed a $0 cap");
    }

    #[test]
    fn budget_tracker_stop_message_is_handed_out_exactly_once() {
        let mut b = BudgetTracker::with_cap(1.00);
        b.record(Some(0.40));
        assert!(b.take_stop_message().is_none(), "not tripped yet");
        b.record(Some(2.00)); // spent 2.40 > 1.00 cap, trips on attempt 2.
        let msg = b
            .take_stop_message()
            .expect("must produce a message the moment it trips");
        assert_eq!(
            msg,
            "BUDGET STOP: spent $2.4000 of $1.0000 cap after 2 tasks"
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
        };
        let deps = Deps {
            harness: &FakeHarness {
                drop_file: ("noop.txt".to_string(), "x".to_string()),
                exit: Some(0),
            },
            oracle: &NeverOracle,
            // $0.60/task: task 1 => spent 0.60 (<=1.00), task 2 => spent 1.20
            // (>1.00, trips), task 3 must be skipped.
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

    #[test]
    fn scratch_dir_removed_when_the_harness_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[("seed.txt", "s")], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
            use_keyring: false,
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
}
