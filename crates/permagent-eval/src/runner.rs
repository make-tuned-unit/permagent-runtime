//! Orchestration: prepare an isolated run, invoke the harness, grade with the
//! oracle, and read the cost — for one task, then a whole tier.
//!
//! The subprocess-touching pieces sit behind the [`HarnessRunner`] and
//! [`OracleRunner`] traits (alongside [`CostReader`](crate::cost::CostReader)) so
//! the decision logic and the tamper-proofing copy are tested with mocks, while
//! the real [`SubprocessHarnessRunner`] / [`SubprocessOracleRunner`] are the thin
//! glue exercised for real on the machine that runs the eval.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::cost::{CostReader, CostReading};
use crate::invocation::{build_invocation, Invocation};
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

/// The three injected dependencies of a run.
pub struct Deps<'a> {
    pub harness: &'a dyn HarnessRunner,
    pub oracle: &'a dyn OracleRunner,
    pub cost: &'a dyn CostReader,
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
}

/// Run a single task under a tier and return its [`TaskResult`].
pub fn run_task(task: &Task, tier: &Tier, cfg: &RunConfig, deps: &Deps<'_>) -> Result<TaskResult> {
    let run_dir = cfg
        .runs_root
        .join(format!("{}-{}-{}", tier.name, task.spec.id, unique_stamp()));
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

    let inv = build_invocation(&task.spec, tier, &workdir, &data_root, &cfg.permagent_bin);
    let harness_run = deps.harness.run(
        &inv,
        Duration::from_secs(task.spec.harness_timeout_secs()),
        &logs.join("harness.log"),
    )?;

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
    if harness_run.timed_out {
        result.note = Some("harness run hit its wall-clock ceiling".to_string());
    }

    if !cfg.keep {
        let _ = fs::remove_dir_all(&run_dir);
    }
    Ok(result)
}

/// Run every task in `tasks` under `tier`, turning a per-task hard error into an
/// errored result so one failure never aborts the whole tier.
pub fn run_tier(tasks: &[Task], tier: &Tier, cfg: &RunConfig, deps: &Deps<'_>) -> Vec<TaskResult> {
    tasks
        .iter()
        .map(|task| {
            run_task(task, tier, cfg, deps).unwrap_or_else(|e| {
                let mut r = TaskResult::new(
                    task.spec.id.as_str(),
                    task.spec.category.as_str(),
                    OracleOutcome::Errored,
                    CostReading::unknown(),
                );
                r.note = Some(format!("run error: {e}"));
                r
            })
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
        for (k, v) in &inv.envs {
            cmd.env(k, v);
        }
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
        };
        let tier = Tier::builtin("frontier").unwrap();
        let r = run_task(&task, &tier, &cfg, &deps).unwrap();
        assert!(r.solved);
        assert_eq!(r.cost.usd, Some(0.42));
        assert!((r.duration_secs - 1.5).abs() < 1e-9);
        assert_eq!(r.harness_exit, Some(0));
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
        struct BoomCost;
        impl CostReader for BoomCost {
            fn read_total(&self, _d: &Path) -> Result<CostReading> {
                Ok(CostReading::unknown())
            }
        }
        struct BoomHarness;
        impl HarnessRunner for BoomHarness {
            fn run(&self, _i: &Invocation, _t: Duration, _l: &Path) -> Result<HarnessRun> {
                anyhow::bail!("boom")
            }
        }
        struct NeverOracle;
        impl OracleRunner for NeverOracle {
            fn run(
                &self,
                _w: &Path,
                _a: &[String],
                _t: Duration,
                _l: &Path,
            ) -> Result<Option<i32>> {
                Ok(Some(0))
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let task = make_task(&tmp.path().join("demo"), &[], &[]);
        let cfg = RunConfig {
            permagent_bin: "permagent".to_string(),
            runs_root: tmp.path().join("runs"),
            keep: false,
        };
        let deps = Deps {
            harness: &BoomHarness,
            oracle: &NeverOracle,
            cost: &BoomCost,
        };
        let results = run_tier(
            std::slice::from_ref(&task),
            &Tier::builtin("local").unwrap(),
            &cfg,
            &deps,
        );
        assert_eq!(results.len(), 1);
        assert!(!results[0].solved);
        assert_eq!(results[0].oracle, OracleOutcome::Errored);
        assert!(results[0].note.as_deref().unwrap().contains("boom"));
    }
}
