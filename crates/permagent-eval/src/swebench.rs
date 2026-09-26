//! SWE-bench as a task source: a pinned slice of SWE-bench Verified (e.g.
//! `benchmarks/swe-arm-30.yaml`) turned into ordinary eval tasks, judged in the
//! instance's arm64 image on a remote Docker host over SSH.
//!
//! The pieces, in the order a run meets them:
//!
//! 1. **Slice** ([`LoadedSlice`]): the pinned YAML (dataset revision, instance
//!    ids, image refs + digests) plus its `instances_file` JSONL holding each
//!    instance's `problem_statement`, `eval_script`, `log_parser` and the
//!    `FAIL_TO_PASS` / `PASS_TO_PASS` lists. Gold patches are never stored.
//! 2. **Materialize** ([`materialize`]): each instance becomes a normal task
//!    directory — `workspace/` is the repo checked out at `base_commit` (no
//!    `.git`; the runner seeds its own baseline commit), `task.yaml`'s prompt is
//!    the issue, and its `test` argv calls back into this binary's `swe-judge`
//!    subcommand. So `run --tasks-dir <out>` drives it through the same
//!    tier/driver/record path as every other task (cost ledger, report,
//!    `--record`, `qualify`).
//! 3. **Judge** ([`judge_instance`]): the oracle. It diffs the finished
//!    workspace against the seed commit ([`workspace_patch`]), sends that diff
//!    plus the dataset's own `eval_script` into the pinned image on the remote
//!    host ([`judge_script`]), and grades the log exactly the way the upstream
//!    harness does ([`LogParser`], [`evaluate_log`], [`grade`]). Resolved means
//!    every `FAIL_TO_PASS` test passes and every `PASS_TO_PASS` test still
//!    passes. Exit 0 = resolved, 1 = not resolved, 2 = infrastructure error
//!    (the runner records 2 as `errored`, not a fail: [`is_judge_infra_exit`]).
//!    Which images the judges pulled, and which judge is using which, lives
//!    in a ledger on the Docker host (`HostLedger`), so an interrupted or
//!    overlapping judge never strands a multi-GB image.
//! 4. **Summary** ([`summarize_report`]): per-task cost, model calls (the cost
//!    ledger's row count, one per provider response), wall time and the tier's
//!    $/solved, from a `run --format json` report.
//!
//! The remote side sits behind [`RemoteExec`] so the judge's decisions (disk
//! floor, pull failure, apply failure, grading) are unit-tested with a fake,
//! and the ledger's shell scripts run in tests against a stand-in `docker`.
//! The SSH conventions (BatchMode, a validated target, the Colima
//! `DOCKER_HOST`) follow `crates/goose-server/src/strix.rs`, without depending
//! on it: this crate must stay independent of the app crates.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// `category:` of every materialized SWE-bench task.
pub const SWE_CATEGORY: &str = "swe-bench";

/// The Colima Docker socket on the Mac mini, expanded by the REMOTE shell.
pub const DEFAULT_DOCKER_HOST: &str = "unix://$HOME/.colima/default/docker.sock";

/// PATH prefix for the remote non-login shell (Homebrew docker, pipx tools).
pub const DEFAULT_REMOTE_PATH_PREFIX: &str = "/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin";

/// Harness-private directories that must never enter the judged diff. Goal
/// workers keep their own git worktrees under `.permagent-goal-worktrees`,
/// which `git add -A` would otherwise record as embedded repositories.
pub const HARNESS_PRIVATE_PATHS: &[&str] = &[".permagent", ".permagent-goal-worktrees", ".goose"];

// Upstream log markers (swebench.harness.constants).
pub const APPLY_PATCH_FAIL: &str = ">>>>> Patch Apply Failed";
pub const APPLY_PATCH_PASS: &str = ">>>>> Applied Patch";
pub const RESET_FAILED: &str = ">>>>> Reset Failed";
pub const TESTS_ERROR: &str = ">>>>> Tests Errored";
pub const TESTS_TIMEOUT: &str = ">>>>> Tests Timed Out";
pub const START_TEST_OUTPUT: &str = ">>>>> Start Test Output";
pub const END_TEST_OUTPUT: &str = ">>>>> End Test Output";
pub const TEST_EXIT_CODE: &str = ">>>>> Test Exit Code";

const PASSED: &str = "PASSED";
const FAILED: &str = "FAILED";
const SKIPPED: &str = "SKIPPED";
const ERROR: &str = "ERROR";
const XFAIL: &str = "XFAIL";
/// Upstream `TestStatus` values, in its declaration order.
const STATUSES: [&str; 5] = [FAILED, PASSED, SKIPPED, ERROR, XFAIL];

// ---------------------------------------------------------------------------
// Slice + instances
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SliceRole {
    Core,
    Spare,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DatasetPin {
    pub repo: String,
    pub revision: String,
    #[serde(default)]
    pub file: Option<String>,
}

/// One pinned instance as listed in the slice YAML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SliceEntry {
    pub instance_id: String,
    pub role: SliceRole,
    pub repo: String,
    #[serde(default)]
    pub difficulty: String,
    pub base_commit: String,
    /// Image repository without tag or digest, e.g.
    /// `ghcr.io/epoch-research/swe-bench.eval.arm64.django__django-11880`.
    pub image: String,
    /// `sha256:<64 hex>` — the judge always runs `image@image_digest`.
    pub image_digest: String,
    /// Sum of the compressed layer sizes, for the judge's disk-floor check.
    #[serde(default)]
    pub image_compressed_bytes: Option<u64>,
}

impl SliceEntry {
    /// The immutable reference the judge pulls and runs.
    pub fn image_ref(&self) -> String {
        format!("{}@{}", self.image, self.image_digest)
    }
}

/// The slice YAML (see `benchmarks/swe-arm-30.yaml`). Unknown keys (the
/// selection metadata and validation results) are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SliceManifest {
    pub id: String,
    /// Path of the instances JSONL, relative to the YAML file.
    pub instances_file: String,
    pub dataset: DatasetPin,
    pub instances: Vec<SliceEntry>,
}

impl SliceManifest {
    pub fn from_yaml(text: &str) -> Result<Self> {
        let manifest: SliceManifest =
            serde_yaml::from_str(text).context("parsing SWE-bench slice YAML")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Pinning rules: an immutable dataset revision, digest-pinned images,
    /// unique filesystem-safe ids, and at least one core instance.
    pub fn validate(&self) -> Result<()> {
        if !is_hex(&self.dataset.revision, 40) {
            bail!(
                "slice {:?}: dataset revision {:?} is not a 40-hex commit SHA (a moving branch is not a pin)",
                self.id,
                self.dataset.revision
            );
        }
        let mut seen = HashSet::new();
        for e in &self.instances {
            if !is_safe_task_id(&e.instance_id) {
                bail!("instance id {:?} is not usable as a task id", e.instance_id);
            }
            if !seen.insert(e.instance_id.as_str()) {
                bail!("instance {:?} is listed twice", e.instance_id);
            }
            let digest_ok = e
                .image_digest
                .strip_prefix("sha256:")
                .is_some_and(|hex| is_hex(hex, 64));
            if !digest_ok {
                bail!(
                    "instance {:?}: image_digest {:?} is not sha256:<64 hex>",
                    e.instance_id,
                    e.image_digest
                );
            }
            if e.image.contains('@') || e.image.trim().is_empty() {
                bail!(
                    "instance {:?}: image must be a bare repository (the digest pins it)",
                    e.instance_id
                );
            }
            if !is_hex(&e.base_commit, 40) {
                bail!(
                    "instance {:?}: base_commit is not a 40-hex SHA",
                    e.instance_id
                );
            }
        }
        if !self.instances.iter().any(|e| e.role == SliceRole::Core) {
            bail!("slice {:?} has no core instances", self.id);
        }
        Ok(())
    }

    /// The entries to run: the explicit `only` ids (in that order, any role),
    /// else every core entry, plus the spares when `include_spares`.
    pub fn select(&self, include_spares: bool, only: &[String]) -> Result<Vec<&SliceEntry>> {
        if !only.is_empty() {
            return only
                .iter()
                .map(|id| {
                    self.instances
                        .iter()
                        .find(|e| &e.instance_id == id)
                        .with_context(|| format!("instance {id:?} is not in slice {:?}", self.id))
                })
                .collect();
        }
        Ok(self
            .instances
            .iter()
            .filter(|e| e.role == SliceRole::Core || include_spares)
            .collect())
    }
}

/// One row of the instances JSONL (the SWE-bench dataset row, minus the gold
/// patch). Extra columns are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SweInstance {
    pub instance_id: String,
    pub repo: String,
    pub base_commit: String,
    pub problem_statement: String,
    #[serde(default)]
    pub difficulty: String,
    #[serde(default)]
    pub version: String,
    /// Already embedded in `eval_script`; kept for inspection.
    #[serde(default)]
    pub test_patch: String,
    /// The dataset's own per-instance evaluation script (checks out the test
    /// files, applies the test patch, runs the tests between the markers).
    pub eval_script: String,
    /// Upstream parser name, e.g. `parse_log_django`.
    pub log_parser: String,
    #[serde(rename = "FAIL_TO_PASS")]
    pub fail_to_pass: Vec<String>,
    #[serde(rename = "PASS_TO_PASS")]
    pub pass_to_pass: Vec<String>,
}

/// Parse an instances JSONL (blank lines skipped).
pub fn parse_instances_jsonl(text: &str) -> Result<Vec<SweInstance>> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| {
            serde_json::from_str::<SweInstance>(l)
                .with_context(|| format!("instances JSONL line {}", i + 1))
        })
        .collect()
}

/// A slice with its instance rows, cross-checked.
#[derive(Debug, Clone)]
pub struct LoadedSlice {
    pub manifest: SliceManifest,
    /// Absolute path of the slice YAML.
    pub path: PathBuf,
    pub instances: BTreeMap<String, SweInstance>,
}

impl LoadedSlice {
    pub fn load(path: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(path)
            .with_context(|| format!("resolving slice path {}", path.display()))?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let manifest =
            SliceManifest::from_yaml(&text).with_context(|| format!("in {}", path.display()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let jsonl_path = dir.join(&manifest.instances_file);
        let jsonl = std::fs::read_to_string(&jsonl_path)
            .with_context(|| format!("reading {}", jsonl_path.display()))?;
        let rows = parse_instances_jsonl(&jsonl)?;
        Self::from_parts(manifest, path, rows)
    }

    /// Cross-check the manifest against its rows (pure; used by `load`).
    pub fn from_parts(
        manifest: SliceManifest,
        path: PathBuf,
        rows: Vec<SweInstance>,
    ) -> Result<Self> {
        let mut instances = BTreeMap::new();
        for row in rows {
            instances.insert(row.instance_id.clone(), row);
        }
        for e in &manifest.instances {
            let row = instances
                .get(&e.instance_id)
                .with_context(|| format!("instance {:?} has no row in the JSONL", e.instance_id))?;
            if row.repo != e.repo || row.base_commit != e.base_commit {
                bail!(
                    "instance {:?}: YAML and JSONL disagree on repo/base_commit",
                    e.instance_id
                );
            }
            if LogParser::from_name(&row.log_parser).is_none() {
                bail!(
                    "instance {:?}: unsupported log parser {:?}",
                    e.instance_id,
                    row.log_parser
                );
            }
            if row.fail_to_pass.is_empty() {
                bail!("instance {:?}: FAIL_TO_PASS is empty", e.instance_id);
            }
        }
        Ok(Self {
            manifest,
            path,
            instances,
        })
    }

    pub fn entry(&self, id: &str) -> Result<&SliceEntry> {
        self.manifest
            .instances
            .iter()
            .find(|e| e.instance_id == id)
            .with_context(|| format!("instance {id:?} is not in slice {:?}", self.manifest.id))
    }

    pub fn instance(&self, id: &str) -> Result<&SweInstance> {
        self.instances
            .get(id)
            .with_context(|| format!("instance {id:?} has no row"))
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Mirrors `task::is_safe_id` (private there): the instance id becomes the
/// task id and its directory name.
fn is_safe_task_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// ---------------------------------------------------------------------------
// Instance -> task
// ---------------------------------------------------------------------------

/// The prompt handed to the harness: the issue text plus the ground rules.
pub fn task_prompt(inst: &SweInstance) -> String {
    format!(
        "You are working in a checkout of the {repo} repository at commit {commit}.\n\
         Resolve the GitHub issue below by editing the repository's source code.\n\
         \n\
         <issue>\n{issue}\n</issue>\n\
         \n\
         Guidelines:\n\
         - Change non-test source files only; hidden tests will be run against your change.\n\
         - Keep the change minimal and consistent with the surrounding code.\n\
         - The project's full test environment may not be installed here, so its test \
         suite may not run locally.\n",
        repo = inst.repo,
        commit = inst.base_commit,
        issue = inst.problem_statement.trim_end(),
    )
}

/// `"<repo>: <first non-empty issue line, at most 80 chars>"`.
pub fn task_title(inst: &SweInstance) -> String {
    let first = inst
        .problem_statement
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(inst.instance_id.as_str());
    let mut short: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        short.push('…');
    }
    format!("{}: {}", inst.repo, short)
}

/// Time limits written into a materialized `task.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskLimits {
    pub harness_timeout_secs: u64,
    pub oracle_timeout_secs: u64,
    pub max_turns: Option<u32>,
}

#[derive(Serialize)]
struct TaskYaml<'a> {
    id: &'a str,
    title: String,
    category: &'a str,
    prompt: String,
    test: &'a [String],
    harness_timeout_secs: u64,
    oracle_timeout_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
    deliverables: Vec<String>,
}

/// Render the `task.yaml` for one instance; `judge_argv` is its oracle.
pub fn render_task_yaml(
    inst: &SweInstance,
    judge_argv: &[String],
    limits: TaskLimits,
) -> Result<String> {
    let doc = TaskYaml {
        id: &inst.instance_id,
        title: task_title(inst),
        category: SWE_CATEGORY,
        prompt: task_prompt(inst),
        test: judge_argv,
        harness_timeout_secs: limits.harness_timeout_secs,
        oracle_timeout_secs: limits.oracle_timeout_secs,
        max_turns: limits.max_turns,
        deliverables: Vec::new(),
    };
    let mut out = format!(
        "# Generated by `permagent-eval swe-materialize` from a pinned SWE-bench slice.\n\
         # instance {} ({}, {}); do not edit — re-materialize instead.\n",
        inst.instance_id, inst.repo, inst.difficulty
    );
    out.push_str(&serde_yaml::to_string(&doc).context("serializing task.yaml")?);
    Ok(out)
}

/// Where and how the judge runs its containers.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeConfig {
    /// SSH target (`m1`, `user@host`).
    pub ssh_target: String,
    /// `DOCKER_HOST` for the remote shell (expanded remotely).
    pub docker_host: String,
    /// Prepended to the remote PATH (expanded remotely).
    pub remote_path_prefix: String,
    /// Refuse to pull when the remote `$HOME` volume would drop below this.
    pub min_free_gib: f64,
    /// Wall-clock cap on one container run.
    pub run_timeout_secs: u64,
    /// Keep a pulled image instead of removing it after the run.
    pub keep_image: bool,
    /// After removing an image, `colima ssh -- sudo fstrim` so the VM hands
    /// the freed blocks back to the host disk.
    pub trim_colima: bool,
    /// `docker run --memory` (protects the services sharing the Docker VM).
    pub container_memory: Option<String>,
    /// Where each verdict JSON is written (None = stdout only).
    pub verdict_dir: Option<PathBuf>,
    /// The Docker host ledger's lock timing (not a CLI flag; tests shorten it).
    pub ledger: LedgerTiming,
}

impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            ssh_target: "m1".to_string(),
            docker_host: DEFAULT_DOCKER_HOST.to_string(),
            remote_path_prefix: DEFAULT_REMOTE_PATH_PREFIX.to_string(),
            min_free_gib: 15.0,
            run_timeout_secs: 1800,
            keep_image: false,
            trim_colima: true,
            container_memory: Some("2g".to_string()),
            verdict_dir: None,
            ledger: LedgerTiming::default(),
        }
    }
}

impl JudgeConfig {
    /// Reject values that would be spliced into a remote shell unsafely.
    pub fn validate(&self) -> Result<()> {
        validate_ssh_target(&self.ssh_target)?;
        let shell_safe = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/_.:-$".contains(c))
        };
        if !shell_safe(&self.docker_host) {
            bail!(
                "docker host {:?} has characters unsafe for a remote shell",
                self.docker_host
            );
        }
        if !shell_safe(&self.remote_path_prefix) {
            bail!(
                "remote PATH prefix {:?} is unsafe for a remote shell",
                self.remote_path_prefix
            );
        }
        if let Some(m) = &self.container_memory {
            if m.is_empty() || !m.chars().all(|c| c.is_ascii_alphanumeric()) {
                bail!("container memory {m:?} must look like 2g / 2048m");
            }
        }
        if self.min_free_gib.is_nan() || self.min_free_gib < 0.0 {
            bail!("min free GiB must be non-negative");
        }
        Ok(())
    }

    /// The `swe-judge` flags that reproduce this config.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "--ssh".to_string(),
            self.ssh_target.clone(),
            "--docker-host".to_string(),
            self.docker_host.clone(),
            "--remote-path-prefix".to_string(),
            self.remote_path_prefix.clone(),
            "--min-free-gib".to_string(),
            format_gib(self.min_free_gib),
            "--run-timeout-secs".to_string(),
            self.run_timeout_secs.to_string(),
        ];
        match &self.container_memory {
            Some(m) => args.extend(["--container-memory".to_string(), m.clone()]),
            None => args.extend(["--container-memory".to_string(), "none".to_string()]),
        }
        if self.keep_image {
            args.push("--keep-image".to_string());
        }
        if !self.trim_colima {
            args.push("--no-trim-colima".to_string());
        }
        if let Some(dir) = &self.verdict_dir {
            args.extend([
                "--verdict-dir".to_string(),
                dir.to_string_lossy().into_owned(),
            ]);
        }
        args
    }

    /// Anchor a relative `verdict_dir` at `base` (the operator's cwd). The
    /// judge runs with each run's scratch workspace as its cwd, which the
    /// runner deletes after the task, so a relative dir baked into task.yaml
    /// would put every verdict (and judged patch) there.
    pub fn anchor_verdict_dir(&mut self, base: &Path) {
        if let Some(dir) = self.verdict_dir.as_mut() {
            if dir.is_relative() {
                *dir = base.join(dir.as_path());
            }
        }
    }

    fn prelude(&self) -> String {
        format!(
            "export PATH={}:$PATH; export DOCKER_HOST={}; ",
            self.remote_path_prefix, self.docker_host
        )
    }

    /// SSH cap on one ledger step (claim/reap, release): the lock wait plus
    /// the work done while holding it.
    pub fn ledger_step_timeout_secs(&self) -> u64 {
        self.ledger.lock_wait_secs.saturating_add(LEDGER_WORK_SECS)
    }

    /// The longest one `swe-judge` can take with every remote step at its
    /// cap: claim/reap, pull, container run, release, plus local overhead.
    pub fn worst_case_secs(&self) -> u64 {
        self.ledger_step_timeout_secs()
            .saturating_mul(2)
            .saturating_add(PULL_TIMEOUT_SECS)
            .saturating_add(self.run_timeout_secs)
            .saturating_add(JUDGE_OVERHEAD_SECS)
    }

    /// The runner-side oracle ceiling a materialized task gets by default:
    /// past the judge's worst case, so the runner never kills `swe-judge`
    /// (which cannot catch the signal) before its release step has run.
    pub fn default_oracle_timeout_secs(&self) -> u64 {
        self.worst_case_secs()
            .saturating_add(ORACLE_TIMEOUT_MARGIN_SECS)
    }

    /// Refuse an explicit oracle ceiling the judge could outlive.
    pub fn check_oracle_timeout(&self, oracle_timeout_secs: u64) -> Result<()> {
        let worst = self.worst_case_secs();
        if oracle_timeout_secs <= worst {
            bail!(
                "an oracle timeout of {} s is not above the judge's worst case ({} s: pull {} s + \
                 run {} s + two ledger steps of {} s + {} s overhead), so the runner could kill \
                 swe-judge before it releases its container and image; lower --run-timeout-secs \
                 instead",
                oracle_timeout_secs,
                worst,
                PULL_TIMEOUT_SECS,
                self.run_timeout_secs,
                self.ledger_step_timeout_secs(),
                JUDGE_OVERHEAD_SECS
            );
        }
        Ok(())
    }
}

fn format_gib(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as u64)
    } else {
        format!("{v}")
    }
}

/// The oracle argv for one instance: `<bin> swe-judge --slice <abs> --instance <id> …`.
pub fn judge_argv(
    bin: &Path,
    slice_path: &Path,
    instance_id: &str,
    cfg: &JudgeConfig,
) -> Vec<String> {
    let mut argv = vec![
        bin.to_string_lossy().into_owned(),
        "swe-judge".to_string(),
        "--slice".to_string(),
        slice_path.to_string_lossy().into_owned(),
        "--instance".to_string(),
        instance_id.to_string(),
    ];
    argv.extend(cfg.to_args());
    argv
}

/// Same rule as the Guard's `strix_docker_ssh`: `user@host` or a Host alias,
/// no shell metacharacters.
pub fn validate_ssh_target(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.matches('@').count() <= 1
        && !s.starts_with('@')
        && !s.ends_with('@')
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        bail!("{s:?} is not a usable SSH target")
    }
}

fn posix_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Materialize (git + filesystem)
// ---------------------------------------------------------------------------

/// Options for [`materialize`].
#[derive(Debug, Clone)]
pub struct MaterializeOptions {
    pub out_dir: PathBuf,
    /// Bare, blob-less mirrors of each repository live here.
    pub repo_cache: PathBuf,
    /// The binary the oracle calls (`swe-judge`).
    pub judge_bin: PathBuf,
    pub judge: JudgeConfig,
    pub limits: TaskLimits,
    /// Re-create workspaces that already exist.
    pub force: bool,
}

/// Write one task directory per selected entry. Returns the task ids written.
pub fn materialize(
    slice: &LoadedSlice,
    entries: &[&SliceEntry],
    opts: &MaterializeOptions,
) -> Result<Vec<String>> {
    opts.judge.validate()?;
    // The oracle runs with the run's scratch workspace as its cwd, which the
    // runner deletes after the task: a relative verdict dir would land there.
    if let Some(dir) = opts
        .judge
        .verdict_dir
        .as_deref()
        .filter(|d| d.is_relative())
    {
        bail!(
            "verdict dir {} is relative; the judge runs inside each run's scratch workspace, so \
             it must be absolute",
            dir.display()
        );
    }
    std::fs::create_dir_all(&opts.out_dir)
        .with_context(|| format!("creating {}", opts.out_dir.display()))?;
    std::fs::create_dir_all(&opts.repo_cache)
        .with_context(|| format!("creating {}", opts.repo_cache.display()))?;
    let mut written = Vec::new();
    for entry in entries {
        let inst = slice.instance(&entry.instance_id)?;
        let task_dir = opts.out_dir.join(&inst.instance_id);
        let workspace = task_dir.join("workspace");
        let populated = workspace.is_dir()
            && std::fs::read_dir(&workspace)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        if opts.force && workspace.exists() {
            std::fs::remove_dir_all(&workspace)
                .with_context(|| format!("removing {}", workspace.display()))?;
        }
        if opts.force || !populated {
            eprintln!(
                "swe-materialize: {} @ {}",
                inst.repo,
                short_sha(&inst.base_commit)
            );
            let mirror = ensure_mirror(&opts.repo_cache, &inst.repo)?;
            checkout_at(&mirror, &inst.base_commit, &workspace)
                .with_context(|| format!("checking out {}", inst.instance_id))?;
        }
        let argv = judge_argv(&opts.judge_bin, &slice.path, &inst.instance_id, &opts.judge);
        let yaml = render_task_yaml(inst, &argv, opts.limits)?;
        std::fs::write(task_dir.join("task.yaml"), yaml)
            .with_context(|| format!("writing {}/task.yaml", task_dir.display()))?;
        written.push(inst.instance_id.clone());
    }
    Ok(written)
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

fn git(args: &[&str], envs: &[(&str, &Path)]) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    Ok(out)
}

/// A successful git command's stdout, byte for byte (a diff of a non-UTF-8
/// file must reach `git apply` unaltered).
fn git_ok_bytes(args: &[&str], envs: &[(&str, &Path)]) -> Result<Vec<u8>> {
    let out = git(args, envs)?;
    if !out.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// [`git_ok_bytes`] for commands whose output is text (SHAs, names).
fn git_ok(args: &[&str], envs: &[(&str, &Path)]) -> Result<String> {
    git_ok_bytes(args, envs).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// A bare, blob-less mirror of `https://github.com/<repo>` under `cache`.
pub fn ensure_mirror(cache: &Path, repo: &str) -> Result<PathBuf> {
    if !repo.split('/').all(|p| {
        !p.is_empty()
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    }) || repo.matches('/').count() != 1
    {
        bail!("repo {repo:?} is not an owner/name GitHub slug");
    }
    let dir = cache.join(format!("{}.git", repo.replace('/', "__")));
    if !dir.join("HEAD").is_file() {
        let url = format!("https://github.com/{repo}.git");
        let dir_s = dir.to_string_lossy().into_owned();
        git_ok(
            &[
                "clone",
                "--bare",
                "--filter=blob:none",
                "--quiet",
                url.as_str(),
                dir_s.as_str(),
            ],
            &[],
        )?;
    }
    Ok(dir)
}

/// Populate `dest` with the tree of `commit` (no `.git`, symlinks as plain
/// files so the runner's copy never trips over a dangling link).
pub fn checkout_at(mirror: &Path, commit: &str, dest: &Path) -> Result<()> {
    let git_dir = mirror.to_string_lossy().into_owned();
    let spec = format!("{commit}^{{commit}}");
    if !git(
        &[
            "--git-dir",
            git_dir.as_str(),
            "cat-file",
            "-e",
            spec.as_str(),
        ],
        &[],
    )?
    .status
    .success()
    {
        git_ok(
            &[
                "--git-dir",
                git_dir.as_str(),
                "fetch",
                "--quiet",
                "origin",
                commit,
            ],
            &[],
        )?;
    }
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let index = dest
        .parent()
        .unwrap_or(dest)
        .join(format!(".swe-index-{}", std::process::id()));
    let work_tree = dest.to_string_lossy().into_owned();
    let result = git_ok(
        &[
            "-c",
            "core.symlinks=false",
            "--git-dir",
            git_dir.as_str(),
            "--work-tree",
            work_tree.as_str(),
            "checkout",
            "-f",
            commit,
            "--",
            ".",
        ],
        &[("GIT_INDEX_FILE", index.as_path())],
    );
    let _ = std::fs::remove_file(&index);
    result.map(|_| ())
}

/// The agent's change, as [`workspace_patch`] found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePatch {
    /// `git diff --binary` output, byte for byte: a text file in another
    /// encoding (Latin-1 fixtures, say) is diffed raw, and any re-encoding
    /// would stop the patch applying to `/testbed`.
    pub bytes: Vec<u8>,
    /// The runner's seed commit the diff is against — also the run's key in
    /// the report (`seed_commit`), which is how `swe-summary` ties a verdict
    /// to exactly one run.
    pub seed_commit: String,
}

/// The runner's seed commit in `workspace`: the repository's root commit
/// (the oldest one, should the harness have made another root).
pub fn workspace_seed_commit(workspace: &Path) -> Result<String> {
    let ws = workspace.to_string_lossy().into_owned();
    let roots = git_ok(
        &["-C", ws.as_str(), "rev-list", "--max-parents=0", "HEAD"],
        &[],
    )?;
    Ok(roots
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .context("workspace has no root commit")?
        .to_string())
}

/// The runner's scratch directory name (`<tier>-<task>-<stamp>`) for a
/// workspace at `<runs_root>/<that>/workspace`.
pub fn workspace_run_dir(workspace: &Path) -> Option<String> {
    let ws = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    ws.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

/// The agent's change: everything in `workspace` that differs from the
/// runner's seed commit (the repository's root commit), untracked files
/// included, harness-private directories excluded. Stages into the
/// workspace's index, which is safe: grading is the last thing that touches
/// the scratch workspace.
pub fn workspace_patch(workspace: &Path) -> Result<WorkspacePatch> {
    let ws = workspace.to_string_lossy().into_owned();
    let excludes: Vec<String> = HARNESS_PRIVATE_PATHS
        .iter()
        .map(|p| format!(":(exclude){p}"))
        .collect();
    let mut add: Vec<&str> = vec!["-C", ws.as_str(), "add", "-A", "--", "."];
    add.extend(excludes.iter().map(String::as_str));
    git_ok(&add, &[])?;
    let root = workspace_seed_commit(workspace)?;
    let mut diff: Vec<&str> = vec![
        "-C",
        ws.as_str(),
        "-c",
        "diff.noprefix=false",
        "-c",
        "diff.mnemonicPrefix=false",
        "-c",
        "core.quotePath=false",
        "diff",
        "--cached",
        "--binary",
        "--no-color",
        "--no-ext-diff",
        "--no-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        root.as_str(),
        "--",
        ".",
    ];
    diff.extend(excludes.iter().map(String::as_str));
    let bytes = git_ok_bytes(&diff, &[])?;
    Ok(WorkspacePatch {
        bytes,
        seed_commit: root.clone(),
    })
}

// ---------------------------------------------------------------------------
// Log parsing (ports of swebench.harness.log_parsers.python)
// ---------------------------------------------------------------------------

/// Test id -> upstream status token (`PASSED`, `FAILED`, …). Raw tokens are
/// kept, as upstream does, so grading compares exactly the same strings.
pub type StatusMap = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogParser {
    Pytest,
    PytestOptions,
    PytestV2,
    Django,
    Sympy,
    Seaborn,
    Matplotlib,
}

impl LogParser {
    /// Map an upstream parser name (the dataset's `log_parser` column).
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "parse_log_pytest"
            | "parse_log_astroid"
            | "parse_log_flask"
            | "parse_log_marshmallow"
            | "parse_log_pvlib"
            | "parse_log_pyvista"
            | "parse_log_sqlfluff"
            | "parse_log_xarray" => Self::Pytest,
            "parse_log_pytest_options"
            | "parse_log_pydicom"
            | "parse_log_requests"
            | "parse_log_pylint" => Self::PytestOptions,
            "parse_log_pytest_v2"
            | "parse_log_astropy"
            | "parse_log_scikit"
            | "parse_log_sphinx" => Self::PytestV2,
            "parse_log_django" => Self::Django,
            "parse_log_sympy" => Self::Sympy,
            "parse_log_seaborn" => Self::Seaborn,
            "parse_log_matplotlib" => Self::Matplotlib,
            _ => return None,
        })
    }

    pub fn parse(self, log: &str) -> StatusMap {
        match self {
            Self::Pytest => parse_pytest(log, false),
            Self::Matplotlib => parse_pytest(log, true),
            Self::PytestOptions => parse_pytest_options(log),
            Self::PytestV2 => parse_pytest_v2(log),
            Self::Django => parse_django(log),
            Self::Sympy => parse_sympy(log),
            Self::Seaborn => parse_seaborn(log),
        }
    }
}

fn starts_with_status(line: &str) -> bool {
    STATUSES.iter().any(|s| line.starts_with(s))
}

/// pytest's `SKIPPED [N] path:line: reason` summary, where `[N]` is a count.
fn is_skip_summary(status: &str, name: &str) -> bool {
    status == SKIPPED
        && name
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn parse_pytest(log: &str, matplotlib: bool) -> StatusMap {
    let mut map = StatusMap::new();
    for raw in log.split('\n') {
        let mut line = raw.to_string();
        if matplotlib {
            line = line
                .replace("MouseButton.LEFT", "1")
                .replace("MouseButton.RIGHT", "3");
        }
        if !starts_with_status(&line) {
            continue;
        }
        if line.starts_with(FAILED) {
            line = line.replace(" - ", " ");
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() <= 1 || is_skip_summary(parts[0], parts[1]) {
            continue;
        }
        map.insert(parts[1].to_string(), parts[0].to_string());
    }
    map
}

fn parse_pytest_options(log: &str) -> StatusMap {
    let mut map = StatusMap::new();
    for raw in log.split('\n') {
        if !starts_with_status(raw) {
            continue;
        }
        let line = if raw.starts_with(FAILED) {
            raw.replace(" - ", " ")
        } else {
            raw.to_string()
        };
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() <= 1 || is_skip_summary(parts[0], parts[1]) {
            continue;
        }
        // Upstream: re.search(r"(.*?)\[(.*)\]") — up to the first `[`, then
        // everything to the last `]`.
        let name = match parts[1]
            .split_once('[')
            .and_then(|(main, rest)| rest.rsplit_once(']').map(|(opt, _)| (main, opt)))
        {
            Some((main, option)) => {
                let option = if option.starts_with('/')
                    && !option.starts_with("//")
                    && !option.contains('*')
                {
                    format!("/{}", option.rsplit('/').next().unwrap_or(""))
                } else {
                    option.to_string()
                };
                format!("{main}[{option}]")
            }
            None => parts[1].to_string(),
        };
        map.insert(name, parts[0].to_string());
    }
    map
}

static ANSI_COLOR_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\[(\d+)m").expect("static regex"));

fn parse_pytest_v2(log: &str) -> StatusMap {
    let mut map = StatusMap::new();
    for raw in log.split('\n') {
        let stripped = ANSI_COLOR_RE.replace_all(raw, "");
        // Upstream deletes every control character 1..=31 (tabs included).
        let mut line: String = stripped
            .chars()
            .filter(|c| !(('\u{1}'..='\u{1f}').contains(c)))
            .collect();
        if starts_with_status(&line) {
            if line.starts_with(FAILED) {
                if let Some((head, _)) = line.split_once(" - ") {
                    line = head.to_string();
                }
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && !is_skip_summary(parts[0], parts[1]) {
                map.insert(parts[1..].join(" "), parts[0].to_string());
            }
        } else if STATUSES.iter().any(|s| line.ends_with(s)) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let (last, rest) = parts.split_last().expect("len >= 2");
                map.insert(rest.join(" "), (*last).to_string());
            }
        }
    }
    map
}

static DJANGO_MULTILINE_RES: std::sync::LazyLock<Vec<regex::Regex>> =
    std::sync::LazyLock::new(|| {
        [
            r"(?m)^(.*?)\s\.\.\.\sTesting against Django installed in ((?s:.*?)) silenced\)\.\nok$",
            r"(?m)^(.*?)\s\.\.\.\sInternal Server Error: /(.*)/\nok$",
            r"(?m)^(.*?)\s\.\.\.\sSystem check identified no issues \(0 silenced\)\nok$",
        ]
        .iter()
        .map(|p| regex::Regex::new(p).expect("static regex"))
        .collect()
    });

fn parse_django(log: &str) -> StatusMap {
    let mut map = StatusMap::new();
    let mut prev_test: Option<String> = None;
    for raw in log.split('\n') {
        let mut line = raw.trim().to_string();
        if line.contains("--version is equivalent to version") {
            map.insert(
                "--version is equivalent to version".to_string(),
                PASSED.to_string(),
            );
        }
        if let Some((head, _)) = line.split_once(" ... ") {
            prev_test = Some(head.to_string());
        }
        for suffix in [" ... ok", " ... OK", " ...  OK"] {
            if line.ends_with(suffix) {
                // Upstream special case for django__django-7188.
                if line.starts_with("Applying sites.0002_alter_domain_unique...test_no_migrations")
                {
                    line = line
                        .split_once("...")
                        .map(|(_, r)| r.trim().to_string())
                        .unwrap_or(line);
                }
                if let Some(test) = line.strip_suffix(suffix) {
                    map.insert(test.to_string(), PASSED.to_string());
                }
                break;
            }
        }
        if let Some((test, _)) = line.split_once(" ... skipped") {
            map.insert(test.to_string(), SKIPPED.to_string());
        }
        if line.ends_with(" ... FAIL") {
            if let Some((test, _)) = line.split_once(" ... FAIL") {
                map.insert(test.to_string(), FAILED.to_string());
            }
        }
        if line.starts_with("FAIL:") {
            if let Some(test) = line.split_whitespace().nth(1) {
                map.insert(test.to_string(), FAILED.to_string());
            }
        }
        if line.ends_with(" ... ERROR") {
            if let Some((test, _)) = line.split_once(" ... ERROR") {
                map.insert(test.to_string(), ERROR.to_string());
            }
        }
        if line.starts_with("ERROR:") {
            if let Some(test) = line.split_whitespace().nth(1) {
                map.insert(test.to_string(), ERROR.to_string());
            }
        }
        if line.starts_with("ok") {
            if let Some(prev) = &prev_test {
                map.insert(prev.clone(), PASSED.to_string());
            }
        }
    }
    for re in DJANGO_MULTILINE_RES.iter() {
        for caps in re.captures_iter(log) {
            if let Some(name) = caps.get(1) {
                map.insert(name.as_str().to_string(), PASSED.to_string());
            }
        }
    }
    map
}

static SYMPY_FAILURE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(_*) (.*)\.py:(.*) (_*)").expect("static regex")
});

fn parse_sympy(log: &str) -> StatusMap {
    let mut map = StatusMap::new();
    for caps in SYMPY_FAILURE_RE.captures_iter(log) {
        let file = caps.get(2).map_or("", |m| m.as_str());
        let test = caps.get(3).map_or("", |m| m.as_str());
        map.insert(format!("{file}.py:{test}"), FAILED.to_string());
    }
    for raw in log.split('\n') {
        let line = raw.trim();
        if !line.starts_with("test_") {
            continue;
        }
        let name = line.split_whitespace().next().unwrap_or("").to_string();
        if line.ends_with(" E") {
            map.insert(name.clone(), ERROR.to_string());
        }
        if line.ends_with(" F") {
            map.insert(name.clone(), FAILED.to_string());
        }
        if line.ends_with(" ok") {
            map.insert(name, PASSED.to_string());
        }
    }
    map
}

fn parse_seaborn(log: &str) -> StatusMap {
    let mut map = StatusMap::new();
    for line in log.split('\n') {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        if line.starts_with(FAILED) {
            map.insert(parts[1].to_string(), FAILED.to_string());
        } else if line.contains(" PASSED ") {
            if parts[1] == PASSED {
                map.insert(parts[0].to_string(), PASSED.to_string());
            }
        } else if line.starts_with(PASSED) {
            map.insert(parts[1].to_string(), PASSED.to_string());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Log evaluation + grading (ports of swebench.harness.grading)
// ---------------------------------------------------------------------------

static SUITE_RAN_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?m)Executed [1-9]\d* of \d+|TOTAL: [1-9]\d* (?:SUCCESS|FAILED)|[1-9]\d* passing|Tests:\s+[1-9]\d*|Test Suites:\s+(?:\d+ \w+, )*[1-9]\d* total|^# tests [1-9]\d*|[1-9]\d* specs?, \d+ failures?|': ok$",
    )
    .expect("static regex")
});

static TEST_EXIT_CODE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(&format!(r"{}:\s*(-?\d+)", regex::escape(TEST_EXIT_CODE)))
        .expect("static regex")
});

/// Turn a judge log into a status map, or say why the log cannot be graded
/// (upstream `get_logs_eval`).
pub fn evaluate_log(log: &str, parser: LogParser) -> std::result::Result<StatusMap, String> {
    for bad in [APPLY_PATCH_FAIL, RESET_FAILED, TESTS_ERROR, TESTS_TIMEOUT] {
        if log.contains(bad) {
            return Err(format!("log contains {bad:?}"));
        }
    }
    let sliced = match log.split_once(START_TEST_OUTPUT) {
        Some((_, after)) if after.contains(END_TEST_OUTPUT) => {
            after.split(END_TEST_OUTPUT).next().unwrap_or("")
        }
        _ => return Err("test output markers missing".to_string()),
    };
    let mut map = parser.parse(sliced);
    if map.is_empty() {
        map = parser.parse(log);
    }
    if map.is_empty() && !SUITE_RAN_RE.is_match(log) {
        return Err("no test results could be parsed".to_string());
    }
    if let Some(code) = TEST_EXIT_CODE_RE
        .captures(log)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok())
    {
        let any_failure = map.values().any(|s| s == FAILED || s == ERROR);
        if code != 0 && !map.is_empty() && !any_failure {
            return Err(format!(
                "test command exited {code} but the log reports no failure"
            ));
        }
    }
    Ok(map)
}

/// Upstream `_resolve_case`: exact id, or — only for ids truncated inside a
/// pytest parameter (more `[` than `]`) — a unique-outcome prefix match.
fn resolve_case<'a>(case: &str, map: &'a StatusMap) -> Option<&'a str> {
    if let Some((k, _)) = map.get_key_value(case) {
        return Some(k.as_str());
    }
    if case.matches('[').count() > case.matches(']').count() {
        let matches: Vec<&String> = map.keys().filter(|k| k.starts_with(case)).collect();
        let passing = |k: &String| matches!(map[k].as_str(), PASSED | XFAIL);
        if let Some(first) = matches.first() {
            if matches.iter().all(|k| passing(k) == passing(first)) {
                return Some(first.as_str());
            }
        }
    }
    None
}

fn test_passed(case: &str, map: &StatusMap) -> bool {
    resolve_case(case, map).is_some_and(|k| matches!(map[k].as_str(), PASSED | XFAIL))
}

/// P2P: a skipped test is not a regression.
fn test_maintained(case: &str, map: &StatusMap) -> bool {
    test_passed(case, map) || resolve_case(case, map).is_some_and(|k| map[k] == SKIPPED)
}

/// The per-list outcome of grading one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeReport {
    pub f2p_success: Vec<String>,
    pub f2p_failure: Vec<String>,
    pub p2p_success: Vec<String>,
    pub p2p_failure: Vec<String>,
    pub resolved: bool,
}

/// Grade a status map against the instance's test lists.
///
/// Resolved = every `FAIL_TO_PASS` test passed (or xfailed) AND every
/// `PASS_TO_PASS` test passed or was skipped. Stricter than upstream in one
/// corner: a test whose status token is none of the known ones counts as a
/// failure here, where upstream leaves it out of both lists.
pub fn grade(map: &StatusMap, fail_to_pass: &[String], pass_to_pass: &[String]) -> GradeReport {
    let mut r = GradeReport::default();
    for t in fail_to_pass {
        if test_passed(t, map) {
            r.f2p_success.push(t.clone());
        } else {
            r.f2p_failure.push(t.clone());
        }
    }
    for t in pass_to_pass {
        if test_maintained(t, map) {
            r.p2p_success.push(t.clone());
        } else {
            r.p2p_failure.push(t.clone());
        }
    }
    r.resolved = !fail_to_pass.is_empty() && r.f2p_failure.is_empty() && r.p2p_failure.is_empty();
    r
}

// ---------------------------------------------------------------------------
// The in-container script
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, wrapped at 76 columns (GNU `base64 -d`
/// accepts the newlines).
pub fn base64_wrapped(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4 / 3 + data.len() / 57 + 4);
    for (i, chunk) in data.chunks(3).enumerate() {
        if i > 0 && i.is_multiple_of(19) {
            out.push('\n');
        }
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The script piped to `docker run -i <image> bash -s`: write the patch and
/// the dataset's eval script from base64 (no heredoc delimiter can collide),
/// apply the patch in `/testbed` (git apply, then `patch --fuzz` like
/// upstream), then run the eval script with stderr folded into the log.
/// `benchmarks/tools/validate_swe_arm.py` sends the same script.
/// `patch` is raw bytes: base64 carries them into the container unaltered.
pub fn judge_script(patch: &[u8], eval_script: &str) -> String {
    format!(
        "set -u\nexec 2>&1\n\
         base64 -d > /tmp/permagent.patch <<'PERMAGENT_B64_EOF'\n{p}\nPERMAGENT_B64_EOF\n\
         base64 -d > /tmp/permagent_eval.sh <<'PERMAGENT_B64_EOF'\n{e}\nPERMAGENT_B64_EOF\n\
         cd /testbed\n\
         if [ ! -s /tmp/permagent.patch ]; then\n  echo '{pass} (empty)'\n\
         elif git apply --verbose /tmp/permagent.patch || patch --batch --fuzz=5 -p1 -i /tmp/permagent.patch; then\n  echo '{pass}'\n\
         else\n  echo '{fail}'\n  exit 3\nfi\n\
         bash /tmp/permagent_eval.sh\n",
        p = base64_wrapped(patch),
        e = base64_wrapped(eval_script.as_bytes()),
        pass = APPLY_PATCH_PASS,
        fail = APPLY_PATCH_FAIL,
    )
}

// ---------------------------------------------------------------------------
// Remote execution + the judge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteOutput {
    /// `None` = killed (timeout) or no exit status.
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Runs one shell command on the Docker host.
pub trait RemoteExec {
    fn run(&self, command: &str, stdin: Option<&str>, timeout: Duration) -> Result<RemoteOutput>;
}

/// `ssh -o BatchMode=yes -o ConnectTimeout=8 <target> <command>`.
pub struct SshExec {
    pub target: String,
}

impl RemoteExec for SshExec {
    fn run(&self, command: &str, stdin: Option<&str>, timeout: Duration) -> Result<RemoteOutput> {
        validate_ssh_target(&self.target)?;
        let mut child = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                self.target.as_str(),
                command,
            ])
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawning ssh")?;
        let writer = match (stdin, child.stdin.take()) {
            (Some(text), Some(mut pipe)) => {
                let text = text.to_string();
                Some(std::thread::spawn(move || {
                    let _ = pipe.write_all(text.as_bytes());
                }))
            }
            _ => None,
        };
        let mut out_pipe = child.stdout.take().context("ssh stdout")?;
        let mut err_pipe = child.stderr.take().context("ssh stderr")?;
        let out_t = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out_pipe.read_to_end(&mut buf);
            buf
        });
        let err_t = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err_pipe.read_to_end(&mut buf);
            buf
        });
        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().context("waiting for ssh")? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
        let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
        Ok(RemoteOutput {
            exit: status.and_then(|s| s.code()),
            stdout,
            stderr,
            timed_out,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeOutcome {
    Resolved,
    Unresolved,
    /// The judge could not produce a verdict (disk floor, pull failure, SSH or
    /// Docker failure, a workspace it could not diff). Not the agent's fault:
    /// the runner records it as `errored` ([`is_judge_infra_exit`]) and the
    /// summary reports it separately.
    InfraError,
}

impl JudgeOutcome {
    /// Process exit code for the oracle: 0 / 1 / 2.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Resolved => 0,
            Self::Unresolved => 1,
            Self::InfraError => 2,
        }
    }
}

/// Whether a task's oracle exit means the SWE-bench judge could not grade:
/// `swe-judge` exits 2 for its [`JudgeOutcome::InfraError`], and clap exits 2
/// on a bad argv. The runner records such a run as `errored` (the oracle
/// produced no verdict), never as the agent's `fail`; every other category's
/// oracle keeps the plain zero / non-zero reading.
pub fn is_judge_infra_exit(category: &str, exit: Option<i32>) -> bool {
    category == SWE_CATEGORY && exit == Some(JudgeOutcome::InfraError.exit_code())
}

/// What `swe-judge` decided, written as JSON next to the other verdicts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub instance_id: String,
    pub image: String,
    pub outcome: JudgeOutcome,
    pub reason: Option<String>,
    pub patch_bytes: usize,
    pub patch_applied: Option<bool>,
    pub f2p_passed: usize,
    pub f2p_total: usize,
    pub p2p_passed: usize,
    pub p2p_total: usize,
    /// At most 20 of each, for the report.
    pub f2p_failures: Vec<String>,
    pub p2p_failures: Vec<String>,
    pub pulled: bool,
    pub pull_secs: Option<f64>,
    pub test_secs: Option<f64>,
    pub remote_free_gib: Option<f64>,
    /// The runner's scratch directory name (`<tier>-<task>-<stamp>`), when
    /// judged from a workspace — how `swe-summary` ties a verdict to a tier.
    pub run_dir: Option<String>,
    /// The workspace's seed commit, when judged from a workspace. With
    /// `run_dir` it keys the verdict to exactly one run of a report (whose
    /// rows carry the same `seed_commit`), so a verdict an earlier run left in
    /// a shared verdict dir never labels a later run.
    #[serde(default)]
    pub seed_commit: Option<String>,
    /// Leftovers of earlier judges (a dead judge's container, an image no
    /// judge claims any more) this judge removed from the Docker host first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reaped: Vec<String>,
}

impl JudgeVerdict {
    fn new(inst: &SweInstance, entry: &SliceEntry, patch: &[u8]) -> Self {
        Self {
            instance_id: inst.instance_id.clone(),
            image: entry.image_ref(),
            outcome: JudgeOutcome::InfraError,
            reason: None,
            patch_bytes: patch.len(),
            patch_applied: None,
            f2p_passed: 0,
            f2p_total: inst.fail_to_pass.len(),
            p2p_passed: 0,
            p2p_total: inst.pass_to_pass.len(),
            f2p_failures: Vec::new(),
            p2p_failures: Vec::new(),
            pulled: false,
            pull_secs: None,
            test_secs: None,
            remote_free_gib: None,
            run_dir: None,
            seed_commit: None,
            reaped: Vec::new(),
        }
    }

    /// The verdict of a judge that failed before it could judge anything (the
    /// slice would not load, the workspace would not diff): an InfraError, so
    /// that failure is never read as the agent's.
    pub fn infra_error(instance_id: &str, reason: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            image: String::new(),
            outcome: JudgeOutcome::InfraError,
            reason: Some(reason.into()),
            patch_bytes: 0,
            patch_applied: None,
            f2p_passed: 0,
            f2p_total: 0,
            p2p_passed: 0,
            p2p_total: 0,
            f2p_failures: Vec::new(),
            p2p_failures: Vec::new(),
            pulled: false,
            pull_secs: None,
            test_secs: None,
            remote_free_gib: None,
            run_dir: None,
            seed_commit: None,
            reaped: Vec::new(),
        }
    }

    /// [`Self::infra_error`] for a judge pointed at a runner workspace, keyed
    /// to that run (`run_dir`, `seed_commit`) as far as the workspace still
    /// allows, like every verdict of a judge that got further.
    pub fn infra_error_for_workspace(
        instance_id: &str,
        reason: impl Into<String>,
        workspace: &Path,
    ) -> Self {
        let mut v = Self::infra_error(instance_id, reason);
        v.run_dir = workspace_run_dir(workspace);
        v.seed_commit = workspace_seed_commit(workspace).ok();
        v
    }

    fn with(mut self, outcome: JudgeOutcome, reason: impl Into<String>) -> Self {
        self.outcome = outcome;
        self.reason = Some(reason.into());
        self
    }
}

/// Per-judge-run options not tied to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JudgeRunOptions {
    /// Run the tests even for an empty patch (validation's "empty must fail"
    /// check). Otherwise an empty diff is Unresolved without a container.
    pub run_empty_patch: bool,
}

/// Rough on-disk cost of pulling an image on a containerd-snapshotter
/// Docker: compressed content plus the unpacked snapshot.
pub const PULL_DISK_FACTOR: f64 = 4.0;

// ---------------------------------------------------------------------------
// The Docker host's ledger: which images judges pulled, and who uses them
// ---------------------------------------------------------------------------

/// Where the judges' shared state lives on the Docker host (expanded by the
/// remote shell); one directory per `DOCKER_HOST` below it.
pub const LEDGER_ROOT: &str = "$HOME/.permagent-swe";

/// Label on every judge container: the remote epoch second after which it is
/// a leftover (its judge died or overran) that the next judge removes.
pub const DEADLINE_LABEL: &str = "permagent.swe.deadline";

/// Label naming the judge that started a container, for `docker ps`.
pub const JUDGE_LABEL: &str = "permagent.swe.judge";

/// Wall-clock cap on one image pull.
pub const PULL_TIMEOUT_SECS: u64 = 3600;

/// What a ledger step may do once it holds the lock: remove leftover
/// containers and images, then `fstrim`.
pub const LEDGER_WORK_SECS: u64 = 540;

/// How long a judge container outlives its own run cap before the next judge
/// treats it as a leftover.
pub const CONTAINER_REAP_GRACE_SECS: u64 = 300;

/// A judge's time outside its remote steps: diffing the workspace, SSH
/// connection setup, process start and exit.
pub const JUDGE_OVERHEAD_SECS: u64 = 300;

/// How far past the judge's worst case the default oracle ceiling sits.
pub const ORACLE_TIMEOUT_MARGIN_SECS: u64 = 300;

const COLIMA_TRIM: &str = "colima ssh -- sudo fstrim /mnt/lima-colima >/dev/null 2>&1";

/// First line (a shell comment) of each ledger script, so a log or a test
/// fake can tell the steps apart.
const BEGIN_TAG: &str = "# permagent-swe-judge: begin";
const RELEASE_TAG: &str = "# permagent-swe-judge: release";

/// The ledger's per-host lock timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerTiming {
    /// How long a ledger step waits for the lock before the judge gives up
    /// (an InfraError).
    pub lock_wait_secs: u64,
    /// A lock older than this many minutes belongs to a judge that died
    /// holding it, and is broken. Shorter than the wait, so a dead holder
    /// never outlasts one; longer than any step's real work (a few `docker
    /// rmi` and one `fstrim`), so a live holder is not broken.
    pub lock_stale_mins: u64,
}

impl Default for LedgerTiming {
    fn default() -> Self {
        Self {
            lock_wait_secs: 360,
            lock_stale_mins: 5,
        }
    }
}

/// `s` as a file name: ASCII alphanumerics kept, everything else `_`.
fn file_key(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A per-judge id for its claim file and container label: pid, wall-clock
/// nanos and an in-process counter, so two judges in one process (as in the
/// tests) never share one.
fn judge_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}-{n}", std::process::id())
}

/// POSIX `sh`: does any live judge claim image `$1`?
const CLAIMED_FN: &str = r#"claimed() {
  for c in "$D"/claims/*; do
    [ -f "$c" ] || continue
    _dl=
    _img=
    read -r _dl _img < "$c" || true
    if [ "$_img" = "$1" ]; then return 0; fi
  done
  return 1
}
"#;

/// One judge's handle on the Docker host's ledger, a directory per
/// `DOCKER_HOST` under [`LEDGER_ROOT`]:
///
/// - `images/<key>` — an image some judge pulled (content: the image ref).
///   Only listed images are ever removed; one the host already had is never
///   listed, so no judge touches it.
/// - `claims/<judge id>` — `<deadline> <image ref>`: that judge uses the image
///   until the remote epoch second `<deadline>` (its whole worst case).
/// - `lock/` — the per-host lock (`mkdir`), held only for the short begin and
///   release steps, never across a pull or a test run.
///
/// So image ownership outlives the judge process: an image is removed by the
/// last judge to release it, whoever pulled it, and a killed judge's
/// leftovers (its container, an image it pulled) are reaped by the next judge
/// to begin once the dead judge's deadline has passed. The begin and release
/// steps are POSIX `sh` scripts (the remote login shell may be zsh).
struct HostLedger {
    /// `$HOME/.permagent-swe/<docker host key>`, expanded remotely.
    dir: String,
    judge_id: String,
    container: String,
    image: String,
    image_key: String,
}

/// What the begin step found.
struct LedgerBegin {
    present: bool,
    free_gib: Option<f64>,
    reaped: Vec<String>,
}

impl HostLedger {
    fn new(cfg: &JudgeConfig, instance_id: &str, image: &str) -> Self {
        Self {
            dir: format!("{}/{}", LEDGER_ROOT, file_key(&cfg.docker_host)),
            judge_id: judge_id(),
            container: container_name(instance_id),
            image: image.to_string(),
            image_key: file_key(image),
        }
    }

    /// `sh -c` of: the tag, the prelude, `before_lock`, then `body` holding the
    /// per-host lock (`mkdir`; a lock older than the stale age is broken, and
    /// the lock is dropped on every exit, signals included).
    fn script(&self, cfg: &JudgeConfig, tag: &str, before_lock: &str, body: &str) -> String {
        let script = format!(
            r#"{tag}
{prelude}
{before_lock}D="{dir}"
{claimed}mkdir -p "$D/claims" "$D/images" || {{ echo ledger=unwritable; exit 73; }}
waited=0
until mkdir "$D/lock" 2>/dev/null; do
  waited=$((waited + 1))
  if [ "$waited" -gt {wait} ]; then echo lock=busy; exit 75; fi
  if [ -n "$(find "$D/lock" -maxdepth 0 -mmin +{stale} 2>/dev/null)" ]; then
    rmdir "$D/lock" 2>/dev/null && continue
  fi
  sleep 1
done
trap 'rmdir "$D/lock" 2>/dev/null' EXIT
trap 'exit 129' HUP INT TERM PIPE
{body}"#,
            tag = tag,
            prelude = cfg.prelude(),
            before_lock = before_lock,
            dir = self.dir,
            claimed = CLAIMED_FN,
            wait = cfg.ledger.lock_wait_secs,
            stale = cfg.ledger.lock_stale_mins,
            body = body,
        );
        format!("sh -c {}", posix_single_quote(&script))
    }

    /// Claim the image, reap earlier judges' leftovers, probe the host. The
    /// claim comes first, so no other judge removes this image between this
    /// probe and this judge's release.
    fn begin_command(&self, cfg: &JudgeConfig) -> String {
        let body = format!(
            r#"docker info >/dev/null 2>&1 || {{ echo docker=down; exit 69; }}
now=$(date +%s)
for c in "$D"/claims/*; do
  [ -f "$c" ] || continue
  dl=
  read -r dl _rest < "$c" || true
  case "$dl" in ''|*[!0-9]*) dl=0 ;; esac
  if [ "$dl" -lt "$now" ]; then rm -f "$c"; fi
done
printf '%s %s\n' "$((now + {claim_secs}))" {img} > "$D/claims/{judge}"
docker ps -a --filter label={label} --format '{{{{.Names}}}} {{{{.Label "{label}"}}}}' 2>/dev/null |
while read -r n dl; do
  case "$dl" in ''|*[!0-9]*) continue ;; esac
  if [ -n "$n" ] && [ "$dl" -lt "$now" ] && docker rm -f "$n" >/dev/null 2>&1; then
    echo "reaped=container $n"
  fi
done
freed=0
for f in "$D"/images/*; do
  [ -f "$f" ] || continue
  ref=$(cat "$f")
  if [ -z "$ref" ]; then rm -f "$f"; continue; fi
  if claimed "$ref"; then continue; fi
  if ! docker image inspect "$ref" >/dev/null 2>&1; then rm -f "$f"; continue; fi
  if docker rmi "$ref" >/dev/null 2>&1; then
    rm -f "$f"
    freed=1
    echo "reaped=image $ref"
  fi
done
if [ "$freed" = 1 ]; then {trim}; fi
if docker image inspect {img} >/dev/null 2>&1; then echo present=1; else echo present=0; fi
echo "free_kb=$(df -k "$HOME" | tail -1 | awk '{{print $4}}')"
echo ledger=ok
"#,
            claim_secs = cfg.worst_case_secs(),
            img = posix_single_quote(&self.image),
            judge = self.judge_id,
            label = DEADLINE_LABEL,
            trim = if cfg.trim_colima { COLIMA_TRIM } else { ":" },
        );
        self.script(cfg, BEGIN_TAG, "", &body)
    }

    /// Run [`Self::begin_command`] and read what it found.
    fn begin(
        &self,
        cfg: &JudgeConfig,
        exec: &dyn RemoteExec,
    ) -> std::result::Result<LedgerBegin, String> {
        let out = exec
            .run(
                &self.begin_command(cfg),
                None,
                Duration::from_secs(cfg.ledger_step_timeout_secs()),
            )
            .map_err(|e| format!("{e:#}"))?;
        let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
        let mut reaped = Vec::new();
        for line in out.stdout.lines() {
            match line.trim().split_once('=') {
                Some(("reaped", what)) => reaped.push(what.to_string()),
                Some((key, value)) => {
                    fields.insert(key, value);
                }
                None => {}
            }
        }
        if fields.get("ledger") != Some(&"ok") {
            return Err(if fields.contains_key("docker") {
                format!("docker unreachable on {}", cfg.ssh_target)
            } else if fields.contains_key("lock") {
                format!(
                    "the judge ledger on {} stayed locked for {} s",
                    cfg.ssh_target, cfg.ledger.lock_wait_secs
                )
            } else if fields.contains_key("ledger") {
                format!("cannot write the judge ledger on {}", cfg.ssh_target)
            } else {
                format!(
                    "docker unreachable on {}: {}",
                    cfg.ssh_target,
                    last_line(&out.stderr)
                )
            });
        }
        Ok(LedgerBegin {
            present: fields.get("present") == Some(&"1"),
            free_gib: fields
                .get("free_kb")
                .and_then(|kb| kb.trim().parse::<f64>().ok())
                .map(|kb| kb / 1024.0 / 1024.0),
            reaped,
        })
    }

    /// Pull the image, listing it in the ledger FIRST (unless images are
    /// kept): a judge killed mid-pull then leaves a listed image, which the
    /// next judge reaps, rather than an unlisted one nobody ever would.
    fn pull_command(&self, cfg: &JudgeConfig) -> String {
        let img = posix_single_quote(&self.image);
        let record = if cfg.keep_image {
            String::new()
        } else {
            format!(
                "mkdir -p \"{dir}/images\" && printf '%s\\n' {img} > \"{dir}/images/{key}\" && ",
                dir = self.dir,
                img = img,
                key = self.image_key,
            )
        };
        format!("{}{record}docker pull -q {img}", cfg.prelude())
    }

    /// The test container: never pulls on its own (`--pull=never`, so a
    /// vanished image is an error, not an unrecorded pull that skips the disk
    /// floor), labelled with the moment it becomes a leftover.
    fn run_command(&self, cfg: &JudgeConfig) -> String {
        let memory = cfg
            .container_memory
            .as_ref()
            .map(|m| format!("--memory {m} "))
            .unwrap_or_default();
        format!(
            "{prelude}docker run --rm -i --pull=never --name {name} --label {judge_label}={id} \
             --label {deadline_label}=$(($(date +%s) + {secs})) {memory}{img} bash -s",
            prelude = cfg.prelude(),
            name = self.container,
            judge_label = JUDGE_LABEL,
            id = self.judge_id,
            deadline_label = DEADLINE_LABEL,
            secs = cfg
                .run_timeout_secs
                .saturating_add(CONTAINER_REAP_GRACE_SECS),
            memory = memory,
            img = posix_single_quote(&self.image),
        )
    }

    /// Remove the container (a timeout leaves it running), drop this judge's
    /// claim, then remove the image if the ledger lists it and no other judge
    /// still claims it, and hand the freed blocks back to the host.
    fn release_command(&self, cfg: &JudgeConfig) -> String {
        let img = posix_single_quote(&self.image);
        let key = &self.image_key;
        let trim = if cfg.trim_colima { COLIMA_TRIM } else { ":" };
        // Outside the lock: removing the container needs no ledger state, and
        // a busy lock must not leave it running.
        let before_lock = format!("docker rm -f {} >/dev/null 2>&1\n", self.container);
        let remove = if cfg.keep_image {
            String::new()
        } else {
            format!(
                r#"if docker info >/dev/null 2>&1 && [ -f "$D/images/{key}" ] && ! claimed {img}; then
  if ! docker image inspect {img} >/dev/null 2>&1; then
    rm -f "$D/images/{key}"
  elif docker rmi {img} >/dev/null 2>&1; then
    rm -f "$D/images/{key}"
    echo removed=1
    {trim}
  fi
fi
"#
            )
        };
        let body = format!(
            "rm -f \"$D/claims/{}\"\n{remove}echo ledger=ok\n",
            self.judge_id
        );
        self.script(cfg, RELEASE_TAG, &before_lock, &body)
    }
}

/// Judge one patch for one instance on the remote host.
///
/// Remote steps, each capped (their sum is [`JudgeConfig::worst_case_secs`]):
/// begin (claim the image in the host ledger, reap earlier judges'
/// leftovers, probe), pull when absent, run the container, and release
/// (remove the container, drop the claim, remove the image when the ledger
/// lists it and no other judge claims it).
pub fn judge_instance(
    inst: &SweInstance,
    entry: &SliceEntry,
    patch: &[u8],
    cfg: &JudgeConfig,
    run: JudgeRunOptions,
    exec: &dyn RemoteExec,
    image_compressed_bytes: Option<u64>,
) -> JudgeVerdict {
    let mut v = JudgeVerdict::new(inst, entry, patch);
    if patch.iter().all(u8::is_ascii_whitespace) && !run.run_empty_patch {
        v.patch_applied = Some(false);
        return v.with(JudgeOutcome::Unresolved, "the agent produced no change");
    }
    let Some(parser) = LogParser::from_name(&inst.log_parser) else {
        return v.with(
            JudgeOutcome::InfraError,
            format!("unsupported log parser {}", inst.log_parser),
        );
    };
    if let Err(e) = cfg.validate() {
        return v.with(JudgeOutcome::InfraError, format!("{e:#}"));
    }
    let ledger = HostLedger::new(cfg, &inst.instance_id, &entry.image_ref());
    let begin = match ledger.begin(cfg, exec) {
        Ok(b) => b,
        Err(why) => return v.with(JudgeOutcome::InfraError, why),
    };
    v.reaped = begin.reaped;

    // This judge holds a claim from here on, so every path ends at release.
    let run_result: std::result::Result<RemoteOutput, String> = 'claimed: {
        if !begin.present {
            let Some(free) = begin.free_gib else {
                break 'claimed Err("could not read free disk on the Docker host".to_string());
            };
            v.remote_free_gib = Some(round1(free));
            let need = image_compressed_bytes
                .map(|b| b as f64 * PULL_DISK_FACTOR / 1024f64.powi(3))
                .unwrap_or(4.0);
            if free - need < cfg.min_free_gib {
                break 'claimed Err(format!(
                    "disk floor: {:.1} GiB free, pull needs ~{:.1} GiB, floor {} GiB",
                    free,
                    need,
                    format_gib(cfg.min_free_gib)
                ));
            }
            let t = Instant::now();
            let pull = exec.run(
                &ledger.pull_command(cfg),
                None,
                Duration::from_secs(PULL_TIMEOUT_SECS),
            );
            v.pull_secs = Some(round1(t.elapsed().as_secs_f64()));
            match pull {
                Ok(o) if o.exit == Some(0) => v.pulled = true,
                Ok(o) => {
                    break 'claimed Err(format!("docker pull failed: {}", last_line(&o.stderr)))
                }
                Err(e) => break 'claimed Err(format!("{e:#}")),
            }
        }
        let script = judge_script(patch, &inst.eval_script);
        let t = Instant::now();
        let out = exec.run(
            &ledger.run_command(cfg),
            Some(script.as_str()),
            Duration::from_secs(cfg.run_timeout_secs),
        );
        v.test_secs = Some(round1(t.elapsed().as_secs_f64()));
        out.map_err(|e| format!("{e:#}"))
    };

    let _ = exec.run(
        &ledger.release_command(cfg),
        None,
        Duration::from_secs(cfg.ledger_step_timeout_secs()),
    );
    match run_result {
        Ok(out) => decide(v, &out, parser, inst),
        Err(why) => v.with(JudgeOutcome::InfraError, why),
    }
}

/// The judge's verdict from one container run (pure).
fn decide(
    mut v: JudgeVerdict,
    out: &RemoteOutput,
    parser: LogParser,
    inst: &SweInstance,
) -> JudgeVerdict {
    if out.timed_out {
        return v.with(JudgeOutcome::Unresolved, "tests timed out");
    }
    // 255 = ssh itself failed; 125-127 = docker could not start the container;
    // 137 = the container was SIGKILLed (the --memory cap or the host).
    if matches!(
        out.exit,
        Some(255) | Some(125) | Some(126) | Some(127) | Some(137)
    ) || out.exit.is_none()
    {
        return v.with(
            JudgeOutcome::InfraError,
            format!(
                "container run failed (exit {:?}): {}",
                out.exit,
                last_line(&out.stderr)
            ),
        );
    }
    if out.stdout.contains(APPLY_PATCH_FAIL) {
        v.patch_applied = Some(false);
        return v.with(JudgeOutcome::Unresolved, "the patch did not apply");
    }
    v.patch_applied = Some(out.stdout.contains(APPLY_PATCH_PASS));
    let map = match evaluate_log(&out.stdout, parser) {
        Ok(m) => m,
        Err(why) => return v.with(JudgeOutcome::Unresolved, format!("ungradable log: {why}")),
    };
    let report = grade(&map, &inst.fail_to_pass, &inst.pass_to_pass);
    v.f2p_passed = report.f2p_success.len();
    v.p2p_passed = report.p2p_success.len();
    v.f2p_failures = report.f2p_failure.iter().take(20).cloned().collect();
    v.p2p_failures = report.p2p_failure.iter().take(20).cloned().collect();
    if report.resolved {
        v.outcome = JudgeOutcome::Resolved;
        v.reason = None;
        v
    } else {
        let why = format!(
            "{}/{} FAIL_TO_PASS, {}/{} PASS_TO_PASS",
            v.f2p_passed, v.f2p_total, v.p2p_passed, v.p2p_total
        );
        v.with(JudgeOutcome::Unresolved, why)
    }
}

fn container_name(instance_id: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe: String = instance_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("permagent-swe-{safe}-{}-{millis}", std::process::id())
}

fn last_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Write a verdict to `<dir>/<instance>-<millis>.json`.
pub fn write_verdict(dir: &Path, v: &JudgeVerdict) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!(
        "{}-{millis}-{}.json",
        v.instance_id,
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_string_pretty(v)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Load every verdict JSON in `dir` (unreadable files are skipped).
pub fn load_verdicts(dir: &Path) -> Vec<JudgeVerdict> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryRow {
    pub task_id: String,
    pub repo: String,
    pub difficulty: String,
    pub solved: bool,
    pub oracle: String,
    pub cost_usd: Option<f64>,
    /// Cost-ledger rows = provider responses (the harness's model turns,
    /// sub-agents included).
    pub model_calls: u64,
    /// Harness wall time plus judge wall time.
    pub wall_secs: f64,
    pub judge_secs: f64,
    /// This run's judge, not the agent, failed: the report row says so (see
    /// `report_row_is_judge_error`), or this run's own verdict does.
    pub infra_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TierSummary {
    pub tier: String,
    pub model: String,
    pub rows: Vec<SummaryRow>,
    pub pass_rate: Option<f64>,
    pub dollars_per_solved: Option<f64>,
    pub total_cost_usd: Option<f64>,
    pub median_model_calls: Option<f64>,
    pub median_wall_secs: Option<f64>,
    /// difficulty -> (solved, attempted)
    pub by_difficulty: BTreeMap<String, (usize, usize)>,
    pub infra_errors: usize,
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = xs.len();
    Some(if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    })
}

/// Whether a report row records the judge failing rather than the agent:
/// `swe-judge` exits 2 when it could not grade ([`is_judge_infra_exit`]), and
/// a runner-side oracle timeout means the judge ran past its whole worst case
/// without a verdict. Both are facts about this row's own run, so nothing
/// from another run can leak in.
fn report_row_is_judge_error(row: &serde_json::Value) -> bool {
    let timed_out = row
        .get("oracle_timed_out")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let exit = row
        .get("oracle_exit")
        .and_then(|x| x.as_i64())
        .and_then(|x| i32::try_from(x).ok());
    timed_out || is_judge_infra_exit(SWE_CATEGORY, exit)
}

/// Whether the verdict this row's own judge wrote says InfraError. Keyed by
/// the run, not just the task: same instance, same tier (the run dir is
/// `<tier>-<task>-<stamp>`) and the same seed commit. The runner seeds every
/// run's workspace with a fresh root commit (its hash carries the seeding
/// time) and the report records it, so a verdict an earlier run left in the
/// shared verdict dir never matches. A row without a seed commit matches no
/// verdict.
fn verdict_marks_run_infra(
    verdicts: &[JudgeVerdict],
    tier: &str,
    task_id: &str,
    seed_commit: Option<&str>,
) -> bool {
    let Some(seed_commit) = seed_commit else {
        return false;
    };
    let prefix = format!("{tier}-{task_id}-");
    verdicts.iter().any(|v| {
        v.instance_id == task_id
            && v.outcome == JudgeOutcome::InfraError
            && v.seed_commit.as_deref() == Some(seed_commit)
            && v.run_dir.as_deref().is_some_and(|d| d.starts_with(&prefix))
    })
}

/// Summarize a `run --format json` report (only `swe-bench` tasks). `slice`
/// supplies repo/difficulty; each row's own oracle exit, plus the verdict its
/// own run wrote in `verdicts`, marks judge infrastructure errors.
pub fn summarize_report(
    report: &serde_json::Value,
    slice: Option<&LoadedSlice>,
    verdicts: &[JudgeVerdict],
) -> Vec<TierSummary> {
    let tiers = report
        .get("tiers")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for tier in tiers {
        let name = tier
            .get("tier")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let model = tier
            .get("model")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let tasks = tier
            .get("tasks")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut rows = Vec::new();
        for t in tasks {
            if t.get("category").and_then(|x| x.as_str()) != Some(SWE_CATEGORY) {
                continue;
            }
            let task_id = t
                .get("task_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let oracle = t
                .get("oracle")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if oracle == "not_run" {
                continue;
            }
            let (repo, difficulty) = slice
                .and_then(|s| s.instances.get(&task_id))
                .map(|i| (i.repo.clone(), i.difficulty.clone()))
                .unwrap_or_default();
            let solved = t.get("solved").and_then(|x| x.as_bool()).unwrap_or(false);
            let seed_commit = t.get("seed_commit").and_then(|x| x.as_str());
            let infra_error = !solved
                && (report_row_is_judge_error(&t)
                    || verdict_marks_run_infra(verdicts, &name, &task_id, seed_commit));
            let duration = t
                .get("duration_secs")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let judge_secs = t.get("oracle_secs").and_then(|x| x.as_f64()).unwrap_or(0.0);
            rows.push(SummaryRow {
                task_id,
                repo,
                difficulty,
                solved,
                oracle,
                cost_usd: t.get("cost_usd").and_then(|x| x.as_f64()),
                model_calls: t.get("ledger_rows").and_then(|x| x.as_u64()).unwrap_or(0),
                wall_secs: duration + judge_secs,
                judge_secs,
                infra_error,
            });
        }
        if rows.is_empty() {
            continue;
        }
        // $/solved over every attempted task (failures amortised into the
        // price of a solve, as the crate's own metric does); None when any
        // cost is unknown or nothing was solved.
        let solved = rows.iter().filter(|r| r.solved).count();
        let total_cost = rows
            .iter()
            .map(|r| r.cost_usd)
            .collect::<Option<Vec<f64>>>()
            .map(|v| v.iter().sum::<f64>());
        let mut by_difficulty: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for r in &rows {
            let e = by_difficulty.entry(r.difficulty.clone()).or_default();
            e.1 += 1;
            if r.solved {
                e.0 += 1;
            }
        }
        out.push(TierSummary {
            tier: name,
            model,
            pass_rate: Some(solved as f64 / rows.len() as f64),
            dollars_per_solved: total_cost.filter(|_| solved > 0).map(|c| c / solved as f64),
            total_cost_usd: total_cost,
            median_model_calls: median(rows.iter().map(|r| r.model_calls as f64).collect()),
            median_wall_secs: median(rows.iter().map(|r| r.wall_secs).collect()),
            by_difficulty,
            infra_errors: rows.iter().filter(|r| r.infra_error).count(),
            rows,
        });
    }
    out
}

fn money(v: Option<f64>) -> String {
    v.map_or_else(|| "unknown".to_string(), |x| format!("${x:.4}"))
}

/// Markdown rendering of [`summarize_report`].
pub fn render_summary_markdown(summaries: &[TierSummary]) -> String {
    let mut s = String::new();
    s.push_str("| tier | model | resolved | pass-rate | $/solved | total $ | median calls | median wall s | judge infra errors |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for t in summaries {
        let solved = t.rows.iter().filter(|r| r.solved).count();
        s.push_str(&format!(
            "| {} | {} | {}/{} | {} | {} | {} | {} | {} | {} |\n",
            t.tier,
            t.model,
            solved,
            t.rows.len(),
            t.pass_rate
                .map_or("-".to_string(), |p| format!("{:.1}%", p * 100.0)),
            money(t.dollars_per_solved),
            money(t.total_cost_usd),
            t.median_model_calls
                .map_or("-".to_string(), |x| format!("{x:.0}")),
            t.median_wall_secs
                .map_or("-".to_string(), |x| format!("{x:.0}")),
            t.infra_errors,
        ));
    }
    for t in summaries {
        s.push_str(&format!("\n### {} ({})\n\n", t.tier, t.model));
        let buckets: Vec<String> = t
            .by_difficulty
            .iter()
            .map(|(d, (ok, n))| {
                let label = if d.is_empty() {
                    "unlabelled"
                } else {
                    d.as_str()
                };
                format!("{label}: {ok}/{n}")
            })
            .collect();
        s.push_str(&format!("By difficulty: {}\n\n", buckets.join("; ")));
        s.push_str("| task | repo | difficulty | result | $ | model calls | wall s | judge s |\n");
        s.push_str("|---|---|---|---|---|---|---|---|\n");
        for r in &t.rows {
            let result = if r.solved {
                "resolved".to_string()
            } else if r.infra_error {
                "judge error".to_string()
            } else {
                r.oracle.clone()
            };
            s.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {:.0} | {:.0} |\n",
                r.task_id,
                r.repo,
                r.difficulty,
                result,
                money(r.cost_usd),
                r.model_calls,
                r.wall_secs,
                r.judge_secs
            ));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst() -> SweInstance {
        SweInstance {
            instance_id: "django__django-11880".to_string(),
            repo: "django/django".to_string(),
            base_commit: "06909fe084f87a65459a83bd69d7cdbe4fce9a7c".to_string(),
            problem_statement:
                "\nForm Field’s __deepcopy__ does not (deep)copy the error messages.\nDescription\n"
                    .to_string(),
            difficulty: "<15 min fix".to_string(),
            version: "3.1".to_string(),
            test_patch: String::new(),
            eval_script: "#!/bin/bash\necho hi\n".to_string(),
            log_parser: "parse_log_django".to_string(),
            fail_to_pass: vec![
                "test_field_deep_copy_error_messages (forms_tests.tests.test_forms.FormsTestCase)"
                    .to_string(),
            ],
            pass_to_pass: vec![
                "test_attribute_class (forms_tests.tests.test_forms.RendererTests)".to_string(),
                "test_skipped_one (forms_tests.tests.test_forms.FormsTestCase)".to_string(),
            ],
        }
    }

    fn entry() -> SliceEntry {
        SliceEntry {
            instance_id: "django__django-11880".to_string(),
            role: SliceRole::Core,
            repo: "django/django".to_string(),
            difficulty: "<15 min fix".to_string(),
            base_commit: "06909fe084f87a65459a83bd69d7cdbe4fce9a7c".to_string(),
            image: "ghcr.io/epoch-research/swe-bench.eval.arm64.django__django-11880".to_string(),
            image_digest: format!("sha256:{}", "a".repeat(64)),
            image_compressed_bytes: None,
        }
    }

    const SLICE_YAML: &str = "id: swe-arm-30\n\
instances_file: x.jsonl\n\
dataset:\n  repo: SWE-bench/SWE-bench_Verified\n  revision: 78f471bf655a3137b2e8a75af1501690ec009ec3\n\
instances:\n\
- instance_id: django__django-11880\n  role: core\n  repo: django/django\n  difficulty: <15 min fix\n  base_commit: 06909fe084f87a65459a83bd69d7cdbe4fce9a7c\n  image: ghcr.io/epoch-research/swe-bench.eval.arm64.django__django-11880\n  image_digest: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n  in_verified_mini: true\n\
- instance_id: sympy__sympy-13877\n  role: spare\n  repo: sympy/sympy\n  difficulty: 15 min - 1 hour\n  base_commit: 1659712001810f5fc563a443949f8e3bb38af4bd\n  image: ghcr.io/epoch-research/swe-bench.eval.arm64.sympy__sympy-13877\n  image_digest: sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n";

    // ---- the bundled slice ----

    #[test]
    fn bundled_swe_arm_30_slice_is_pinned_and_stratified() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/swe-arm-30.yaml");
        let slice = LoadedSlice::load(&path).unwrap();
        let core = slice.manifest.select(false, &[]).unwrap();
        assert_eq!(core.len(), 30);
        assert_eq!(slice.manifest.select(true, &[]).unwrap().len(), 36);
        let easy = core
            .iter()
            .filter(|e| e.difficulty == "<15 min fix")
            .count();
        let medium = core
            .iter()
            .filter(|e| e.difficulty == "15 min - 1 hour")
            .count();
        assert_eq!((easy, medium), (12, 18));
        let mut per_repo: BTreeMap<&str, usize> = BTreeMap::new();
        for e in &slice.manifest.instances {
            assert!(
                ![
                    "scikit-learn/scikit-learn",
                    "matplotlib/matplotlib",
                    "pydata/xarray"
                ]
                .contains(&e.repo.as_str()),
                "{} is from an excluded repo",
                e.instance_id
            );
            assert!(e
                .image
                .starts_with("ghcr.io/epoch-research/swe-bench.eval.arm64."));
            assert!(e.image_compressed_bytes.is_some());
            if e.role == SliceRole::Core {
                *per_repo.entry(e.repo.as_str()).or_default() += 1;
            }
            let inst = slice.instance(&e.instance_id).unwrap();
            assert!(
                inst.eval_script.contains(START_TEST_OUTPUT),
                "{}",
                e.instance_id
            );
            assert!(
                inst.eval_script.contains(END_TEST_OUTPUT),
                "{}",
                e.instance_id
            );
            assert!(!inst.problem_statement.trim().is_empty());
        }
        assert!(per_repo.values().all(|&n| n <= 8), "{per_repo:?}");
    }

    // ---- slice ----

    #[test]
    fn slice_parses_selects_core_and_spares() {
        let m = SliceManifest::from_yaml(SLICE_YAML).unwrap();
        let core: Vec<&str> = m
            .select(false, &[])
            .unwrap()
            .iter()
            .map(|e| e.instance_id.as_str())
            .collect();
        assert_eq!(core, vec!["django__django-11880"]);
        assert_eq!(m.select(true, &[]).unwrap().len(), 2);
        let only = m
            .select(false, &["sympy__sympy-13877".to_string()])
            .unwrap();
        assert_eq!(only[0].role, SliceRole::Spare);
        assert!(m.select(false, &["nope".to_string()]).is_err());
        assert_eq!(
            m.instances[0].image_ref(),
            format!(
                "ghcr.io/epoch-research/swe-bench.eval.arm64.django__django-11880@sha256:{}",
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn slice_rejects_unpinned_revision_and_digest() {
        let moving = SLICE_YAML.replace("78f471bf655a3137b2e8a75af1501690ec009ec3", "main");
        assert!(SliceManifest::from_yaml(&moving)
            .unwrap_err()
            .to_string()
            .contains("revision"));
        let tagged = SLICE_YAML.replace(&format!("sha256:{}", "a".repeat(64)), "latest");
        assert!(SliceManifest::from_yaml(&tagged).is_err());
        let dup = SLICE_YAML.replace("sympy__sympy-13877", "django__django-11880");
        assert!(SliceManifest::from_yaml(&dup).is_err());
    }

    #[test]
    fn loaded_slice_cross_checks_rows() {
        let m = SliceManifest::from_yaml(SLICE_YAML).unwrap();
        let mut sympy = inst();
        sympy.instance_id = "sympy__sympy-13877".to_string();
        sympy.repo = "sympy/sympy".to_string();
        sympy.base_commit = "1659712001810f5fc563a443949f8e3bb38af4bd".to_string();
        sympy.log_parser = "parse_log_sympy".to_string();
        assert!(LoadedSlice::from_parts(
            m.clone(),
            PathBuf::from("/s.yaml"),
            vec![inst(), sympy.clone()]
        )
        .is_ok());
        // Missing row.
        assert!(
            LoadedSlice::from_parts(m.clone(), PathBuf::from("/s.yaml"), vec![inst()]).is_err()
        );
        // Unknown parser.
        let mut bad = sympy.clone();
        bad.log_parser = "parse_log_cobol".to_string();
        assert!(
            LoadedSlice::from_parts(m.clone(), PathBuf::from("/s.yaml"), vec![inst(), bad])
                .is_err()
        );
        // Commit disagreement.
        let mut moved = sympy;
        moved.base_commit = "0".repeat(40);
        assert!(LoadedSlice::from_parts(m, PathBuf::from("/s.yaml"), vec![inst(), moved]).is_err());
    }

    #[test]
    fn jsonl_row_parses_upstream_column_names() {
        let row = serde_json::json!({
            "instance_id": "psf__requests-1142", "repo": "psf/requests",
            "base_commit": "22623bd8c265b78b161542663ee980738441c307",
            "problem_statement": "p", "eval_script": "e", "log_parser": "parse_log_requests",
            "FAIL_TO_PASS": ["a"], "PASS_TO_PASS": [], "hints_text": "ignored",
            "created_at": "2013-01-24T00:00:00Z"
        });
        let rows = parse_instances_jsonl(&format!("{row}\n\n")).unwrap();
        assert_eq!(rows[0].fail_to_pass, vec!["a"]);
        assert!(rows[0].pass_to_pass.is_empty());
    }

    // ---- instance -> task ----

    #[test]
    fn instance_maps_to_a_valid_task_spec() {
        let cfg = JudgeConfig {
            verdict_dir: Some(PathBuf::from("/tmp/verdicts")),
            ..JudgeConfig::default()
        };
        let argv = judge_argv(
            Path::new("/bin/permagent-eval"),
            Path::new("/b/swe-arm-30.yaml"),
            "django__django-11880",
            &cfg,
        );
        let yaml = render_task_yaml(
            &inst(),
            &argv,
            TaskLimits {
                harness_timeout_secs: 1800,
                oracle_timeout_secs: 3000,
                max_turns: Some(80),
            },
        )
        .unwrap();
        let spec = crate::task::TaskSpec::from_yaml(&yaml).unwrap();
        assert_eq!(spec.id, "django__django-11880");
        assert_eq!(spec.category, SWE_CATEGORY);
        assert_eq!(spec.recipe, crate::task::DEFAULT_RECIPE);
        assert_eq!(spec.harness_timeout_secs, Some(1800));
        assert_eq!(spec.oracle_timeout_secs, Some(3000));
        assert_eq!(spec.max_turns, Some(80));
        assert!(spec.deliverables.is_empty());
        assert!(spec.prompt.contains("<issue>\n\nForm Field’s __deepcopy__"));
        assert!(spec.prompt.contains(
            "django/django repository at commit 06909fe084f87a65459a83bd69d7cdbe4fce9a7c"
        ));
        assert_eq!(
            spec.title,
            "django/django: Form Field’s __deepcopy__ does not (deep)copy the error messages."
        );
        assert_eq!(
            &spec.test[..6],
            &[
                "/bin/permagent-eval",
                "swe-judge",
                "--slice",
                "/b/swe-arm-30.yaml",
                "--instance",
                "django__django-11880",
            ]
        );
        let joined = spec.test.join(" ");
        assert!(joined.contains("--ssh m1"));
        assert!(joined.contains("--min-free-gib 15"));
        assert!(joined.contains("--verdict-dir /tmp/verdicts"));
        assert!(joined.contains("--container-memory 2g"));
    }

    /// The judge runs inside each run's scratch workspace, which the runner
    /// deletes after the task: a relative `--verdict-dir` baked into task.yaml
    /// sent every verdict there. `swe-materialize` anchors it at the
    /// operator's cwd, and `materialize` refuses one that was not anchored.
    #[test]
    fn a_relative_verdict_dir_never_reaches_task_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sympy = inst();
        sympy.instance_id = "sympy__sympy-13877".to_string();
        sympy.repo = "sympy/sympy".to_string();
        sympy.base_commit = "1659712001810f5fc563a443949f8e3bb38af4bd".to_string();
        sympy.log_parser = "parse_log_sympy".to_string();
        let slice = LoadedSlice::from_parts(
            SliceManifest::from_yaml(SLICE_YAML).unwrap(),
            PathBuf::from("/s.yaml"),
            vec![inst(), sympy],
        )
        .unwrap();
        let opts = |judge: JudgeConfig| MaterializeOptions {
            out_dir: tmp.path().join("tasks"),
            repo_cache: tmp.path().join("repos"),
            judge_bin: PathBuf::from("/bin/permagent-eval"),
            judge,
            limits: TaskLimits {
                harness_timeout_secs: 1800,
                oracle_timeout_secs: 9000,
                max_turns: None,
            },
            force: false,
        };
        let mut judge = JudgeConfig {
            verdict_dir: Some(PathBuf::from("verdicts")),
            ..JudgeConfig::default()
        };
        let err = materialize(&slice, &[], &opts(judge.clone())).unwrap_err();
        assert!(format!("{err:#}").contains("relative"), "{err:#}");

        judge.anchor_verdict_dir(Path::new("/home/op/swe"));
        assert_eq!(
            judge.verdict_dir.as_deref(),
            Some(Path::new("/home/op/swe/verdicts"))
        );
        assert!(materialize(&slice, &[], &opts(judge.clone()))
            .unwrap()
            .is_empty());
        let argv = judge_argv(
            Path::new("/bin/permagent-eval"),
            &slice.path,
            "django__django-11880",
            &judge,
        );
        assert!(argv
            .join(" ")
            .contains("--verdict-dir /home/op/swe/verdicts"));
        // An absolute dir is left as given.
        judge.anchor_verdict_dir(Path::new("/elsewhere"));
        assert_eq!(
            judge.verdict_dir.as_deref(),
            Some(Path::new("/home/op/swe/verdicts"))
        );
    }

    #[test]
    fn long_titles_are_truncated_by_chars() {
        let mut i = inst();
        i.problem_statement = "é".repeat(200);
        let t = task_title(&i);
        assert_eq!(t.chars().count(), "django/django: ".chars().count() + 81);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn judge_config_rejects_shell_injection() {
        assert!(JudgeConfig::default().validate().is_ok());
        for bad in ["m1; rm -rf /", "-oProxyCommand=x", "a@b@c", ""] {
            let cfg = JudgeConfig {
                ssh_target: bad.to_string(),
                ..JudgeConfig::default()
            };
            assert!(cfg.validate().is_err(), "{bad}");
        }
        let cfg = JudgeConfig {
            docker_host: "unix:///x; reboot".to_string(),
            ..JudgeConfig::default()
        };
        assert!(cfg.validate().is_err());
        let cfg = JudgeConfig {
            container_memory: Some("2g --privileged".to_string()),
            ..JudgeConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    // ---- parsers ----

    #[test]
    fn parser_names_map_like_upstream() {
        assert_eq!(
            LogParser::from_name("parse_log_django"),
            Some(LogParser::Django)
        );
        assert_eq!(
            LogParser::from_name("parse_log_sphinx"),
            Some(LogParser::PytestV2)
        );
        assert_eq!(
            LogParser::from_name("parse_log_astropy"),
            Some(LogParser::PytestV2)
        );
        assert_eq!(
            LogParser::from_name("parse_log_requests"),
            Some(LogParser::PytestOptions)
        );
        assert_eq!(
            LogParser::from_name("parse_log_pylint"),
            Some(LogParser::PytestOptions)
        );
        assert_eq!(
            LogParser::from_name("parse_log_pytest"),
            Some(LogParser::Pytest)
        );
        assert_eq!(
            LogParser::from_name("parse_log_sympy"),
            Some(LogParser::Sympy)
        );
        assert_eq!(LogParser::from_name("parse_log_nope"), None);
    }

    #[test]
    fn pytest_parser() {
        let log = "PASSED testing/test_a.py::test_one\n\
                   FAILED testing/test_a.py::test_two - AssertionError: x\n\
                   SKIPPED [1] testing/test_a.py:10: needs network\n\
                   XFAIL testing/test_a.py::test_three\n\
                   ERROR testing/test_b.py::test_four\n\
                   PASSED\n\
                   collected 5 items\n";
        let m = LogParser::Pytest.parse(log);
        assert_eq!(m["testing/test_a.py::test_one"], "PASSED");
        assert_eq!(m["testing/test_a.py::test_two"], "FAILED");
        assert_eq!(m["testing/test_a.py::test_three"], "XFAIL");
        assert_eq!(m["testing/test_b.py::test_four"], "ERROR");
        assert!(!m.contains_key("[1]"));
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn pytest_options_parser_shortens_path_options() {
        let log = "PASSED tests/test_x.py::test_p[/usr/lib/python3/foo.py]\n\
                   PASSED tests/test_x.py::test_q[a-b]\n\
                   FAILED tests/test_x.py::test_r[//net/share] - boom\n\
                   PASSED tests/test_x.py::test_s[/a/*/c]\n";
        let m = LogParser::PytestOptions.parse(log);
        assert_eq!(m["tests/test_x.py::test_p[/foo.py]"], "PASSED");
        assert_eq!(m["tests/test_x.py::test_q[a-b]"], "PASSED");
        assert_eq!(m["tests/test_x.py::test_r[//net/share]"], "FAILED");
        assert_eq!(m["tests/test_x.py::test_s[/a/*/c]"], "PASSED");
    }

    #[test]
    fn pytest_v2_parser_strips_colour_and_keeps_spaced_ids() {
        let log = "\u{1b}[32mPASSED\u{1b}[0m tests/test_ext.py::test_one\n\
                   FAILED tests/test_ext.py::test_two - assert 1 == 2\n\
                   tests/test_util.py::test_old_style PASSED\n\
                   PASSED tests/test_ext.py::test_three[a b]\n";
        let m = LogParser::PytestV2.parse(log);
        assert_eq!(m["tests/test_ext.py::test_one"], "PASSED");
        assert_eq!(m["tests/test_ext.py::test_two"], "FAILED");
        assert_eq!(m["tests/test_util.py::test_old_style"], "PASSED");
        assert_eq!(m["tests/test_ext.py::test_three[a b]"], "PASSED");
    }

    #[test]
    fn django_parser() {
        let log = "test_a (app.tests.T) ... ok\n\
                   test_b (app.tests.T) ... FAIL\n\
                   test_c (app.tests.T) ... ERROR\n\
                   test_d (app.tests.T) ... skipped 'no db'\n\
                   test_e (app.tests.T) ... some output\n\
                   ok\n\
                   test_f (app.tests.T) ... System check identified no issues (0 silenced)\nok\n\
                   ======\n\
                   FAIL: test_b (app.tests.T)\n\
                   ERROR:\n";
        let m = LogParser::Django.parse(log);
        assert_eq!(m["test_a (app.tests.T)"], "PASSED");
        assert_eq!(m["test_b (app.tests.T)"], "FAILED");
        assert_eq!(m["test_b"], "FAILED"); // the `FAIL:` header line, as upstream
        assert_eq!(m["test_c (app.tests.T)"], "ERROR");
        assert_eq!(m["test_d (app.tests.T)"], "SKIPPED");
        assert_eq!(m["test_e (app.tests.T)"], "PASSED");
        assert_eq!(m["test_f (app.tests.T)"], "PASSED");
    }

    #[test]
    fn sympy_parser() {
        let log = "sympy/core/tests/test_basic.py[3]\n\
                   test_one ok\n\
                   test_two F\n\
                   test_three E\n\
                   ________________ sympy/core/tests/test_basic.py:test_two ________________\n";
        let m = LogParser::Sympy.parse(log);
        assert_eq!(m["test_one"], "PASSED");
        assert_eq!(m["test_two"], "FAILED");
        assert_eq!(m["test_three"], "ERROR");
        assert_eq!(m["sympy/core/tests/test_basic.py:test_two"], "FAILED");
    }

    #[test]
    fn seaborn_parser() {
        let log = "tests/test_a.py::test_x PASSED [ 10%]\nFAILED tests/test_a.py::test_y\nPASSED tests/test_a.py::test_z\n";
        let m = LogParser::Seaborn.parse(log);
        assert_eq!(m["tests/test_a.py::test_x"], "PASSED");
        assert_eq!(m["tests/test_a.py::test_y"], "FAILED");
        assert_eq!(m["tests/test_a.py::test_z"], "PASSED");
    }

    // ---- log evaluation + resolved decision ----

    fn wrap(body: &str) -> String {
        format!("{APPLY_PATCH_PASS}\n+ : '{START_TEST_OUTPUT}'\n{body}\n+ : '{END_TEST_OUTPUT}'\n")
    }

    #[test]
    fn evaluate_log_rejects_bad_logs() {
        assert!(evaluate_log(&format!("{APPLY_PATCH_FAIL}\n"), LogParser::Pytest).is_err());
        assert!(evaluate_log("PASSED a::b\n", LogParser::Pytest)
            .unwrap_err()
            .contains("markers"));
        assert!(evaluate_log(&wrap("nothing here"), LogParser::Pytest)
            .unwrap_err()
            .contains("parsed"));
        assert!(evaluate_log(&wrap(TESTS_TIMEOUT), LogParser::Pytest).is_err());
        let m = evaluate_log(&wrap("PASSED a::b"), LogParser::Pytest).unwrap();
        assert_eq!(m["a::b"], "PASSED");
        // Exit code says failure but log claims all passed: not trustworthy.
        let lying = format!("{}{TEST_EXIT_CODE}: 1\n", wrap("PASSED a::b"));
        assert!(evaluate_log(&lying, LogParser::Pytest).is_err());
    }

    #[test]
    fn evaluate_log_falls_back_to_whole_log() {
        let log = format!("PASSED a::b\n+ : '{START_TEST_OUTPUT}'\n+ : '{END_TEST_OUTPUT}'\n");
        assert_eq!(
            evaluate_log(&log, LogParser::Pytest).unwrap()["a::b"],
            "PASSED"
        );
    }

    fn sm(pairs: &[(&str, &str)]) -> StatusMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn resolved_needs_all_f2p_and_all_p2p() {
        let f2p = ["f1".to_string(), "f2".to_string()];
        let p2p = ["p1".to_string(), "p2".to_string()];
        let all = sm(&[
            ("f1", "PASSED"),
            ("f2", "XFAIL"),
            ("p1", "PASSED"),
            ("p2", "SKIPPED"),
        ]);
        assert!(grade(&all, &f2p, &p2p).resolved);
        let f_missing = sm(&[("f1", "PASSED"), ("p1", "PASSED"), ("p2", "PASSED")]);
        let r = grade(&f_missing, &f2p, &p2p);
        assert!(!r.resolved);
        assert_eq!(r.f2p_failure, vec!["f2"]);
        let f_skipped = sm(&[
            ("f1", "PASSED"),
            ("f2", "SKIPPED"),
            ("p1", "PASSED"),
            ("p2", "PASSED"),
        ]);
        assert!(
            !grade(&f_skipped, &f2p, &p2p).resolved,
            "a skipped F2P test is not a fix"
        );
        let regression = sm(&[
            ("f1", "PASSED"),
            ("f2", "PASSED"),
            ("p1", "FAILED"),
            ("p2", "PASSED"),
        ]);
        assert_eq!(grade(&regression, &f2p, &p2p).p2p_failure, vec!["p1"]);
        let unknown_token = sm(&[
            ("f1", "PASSED"),
            ("f2", "WEIRD"),
            ("p1", "PASSED"),
            ("p2", "PASSED"),
        ]);
        assert!(!grade(&unknown_token, &f2p, &p2p).resolved);
        assert!(
            !grade(&StatusMap::new(), &[], &[]).resolved,
            "no F2P tests is never resolved"
        );
    }

    #[test]
    fn truncated_parametrized_ids_prefix_match_when_outcomes_agree() {
        let f2p = ["t.py::test_fail[log(photon".to_string()];
        let agree = sm(&[
            ("t.py::test_fail[log(photon)]", "PASSED"),
            ("t.py::test_fail[log(photon/s)]", "PASSED"),
        ]);
        assert!(grade(&agree, &f2p, &[]).resolved);
        let disagree = sm(&[
            ("t.py::test_fail[log(photon)]", "PASSED"),
            ("t.py::test_fail[log(photon/s)]", "FAILED"),
        ]);
        assert!(!grade(&disagree, &f2p, &[]).resolved);
        // A balanced id never prefix-matches.
        let exact_only = sm(&[("t.py::test_ok[a]x", "PASSED")]);
        assert!(!grade(&exact_only, &["t.py::test_ok[a]".to_string()], &[]).resolved);
    }

    // ---- script ----

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_wrapped(b""), "");
        assert_eq!(base64_wrapped(b"f"), "Zg==");
        assert_eq!(base64_wrapped(b"fo"), "Zm8=");
        assert_eq!(base64_wrapped(b"foo"), "Zm9v");
        assert_eq!(base64_wrapped(b"foobar"), "Zm9vYmFy");
        let long = base64_wrapped(&[0u8; 120]);
        assert!(long.lines().all(|l| l.len() <= 76));
        assert_eq!(long.replace('\n', "").len(), 160);
    }

    #[test]
    fn judge_script_embeds_patch_and_eval_safely() {
        let s = judge_script(b"PERMAGENT_B64_EOF\n'$(rm -rf /)'", "echo hi");
        assert!(s.starts_with("set -u\nexec 2>&1\n"));
        assert_eq!(
            s.matches("\nPERMAGENT_B64_EOF\n").count(),
            2,
            "payload cannot end the heredoc"
        );
        assert!(!s.contains("rm -rf"));
        assert!(
            s.contains("git apply --verbose /tmp/permagent.patch || patch --batch --fuzz=5 -p1")
        );
        assert!(s.trim_end().ends_with("bash /tmp/permagent_eval.sh"));
    }

    // ---- judge decisions with a fake host ----

    struct FakeHost {
        free_kb: &'static str,
        present: bool,
        pull_exit: i32,
        run: RemoteOutput,
        calls: std::cell::RefCell<Vec<String>>,
        /// The cap each call was given, in call order.
        timeouts: std::cell::RefCell<Vec<Duration>>,
    }

    impl RemoteExec for FakeHost {
        fn run(&self, command: &str, _stdin: Option<&str>, t: Duration) -> Result<RemoteOutput> {
            self.calls.borrow_mut().push(command.to_string());
            self.timeouts.borrow_mut().push(t);
            let ok = |s: &str| RemoteOutput {
                exit: Some(0),
                stdout: s.to_string(),
                ..Default::default()
            };
            Ok(if command.contains(BEGIN_TAG) {
                ok(&format!(
                    "present={}\nfree_kb={}\nledger=ok\n",
                    u8::from(self.present),
                    self.free_kb.trim()
                ))
            } else if command.contains("docker pull") {
                RemoteOutput {
                    exit: Some(self.pull_exit),
                    stderr: "denied\n".to_string(),
                    ..Default::default()
                }
            } else if command.contains("docker run") {
                self.run.clone()
            } else {
                ok("ledger=ok\n")
            })
        }
    }

    fn host(free_kb: &'static str, present: bool, run_stdout: &str) -> FakeHost {
        FakeHost {
            free_kb,
            present,
            pull_exit: 0,
            run: RemoteOutput {
                exit: Some(0),
                stdout: run_stdout.to_string(),
                ..Default::default()
            },
            calls: Default::default(),
            timeouts: Default::default(),
        }
    }

    fn passing_log() -> String {
        wrap(
            "test_field_deep_copy_error_messages (forms_tests.tests.test_forms.FormsTestCase) ... ok\n\
             test_attribute_class (forms_tests.tests.test_forms.RendererTests) ... ok\n\
             test_skipped_one (forms_tests.tests.test_forms.FormsTestCase) ... skipped 'x'",
        )
    }

    const PLENTY: &str = "41943040\n"; // 40 GiB

    #[test]
    fn judge_resolves_and_cleans_up_what_it_pulled() {
        let h = host(PLENTY, false, &passing_log());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff --git a/x b/x\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            Some(1 << 30),
        );
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert_eq!((v.f2p_passed, v.p2p_passed), (1, 2));
        assert!(v.pulled);
        let calls = h.calls.borrow();
        assert!(calls[0].contains(BEGIN_TAG));
        let cleanup = calls.last().unwrap();
        assert!(cleanup.contains(RELEASE_TAG));
        assert!(cleanup.contains("docker rm -f permagent-swe-"));
        assert!(cleanup.contains("docker rmi"));
        assert!(cleanup.contains("fstrim"));
        assert!(calls.iter().any(|c| c.contains("--memory 2g")));
        assert!(calls
            .iter()
            .all(|c| !c.contains("docker") || c.contains("DOCKER_HOST=unix://$HOME/.colima")));
    }

    /// The container never pulls by itself: a vanished image must be an
    /// error, not an unrecorded pull that skipped the disk floor.
    #[test]
    fn judge_container_never_pulls_and_carries_its_reap_deadline() {
        let h = host(PLENTY, true, &passing_log());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        let calls = h.calls.borrow();
        let run = calls.iter().find(|c| c.contains("docker run")).unwrap();
        assert!(run.contains("docker run --rm -i --pull=never --name permagent-swe-"));
        assert!(run.contains("--label permagent.swe.deadline=$(($(date +%s) + 2100))"));
        assert!(run.contains("--label permagent.swe.judge="));
    }

    /// The runner can only SIGTERM/SIGKILL `swe-judge`, which cannot catch
    /// either, so the oracle ceiling must outlast every cap the judge gives
    /// its remote steps, cleanup included. The old default (run cap + 30 min)
    /// did not even cover the pull cap plus the run cap.
    #[test]
    fn the_default_oracle_timeout_outlasts_every_step_the_judge_can_take() {
        let cfg = JudgeConfig::default();
        let h = host(PLENTY, false, &passing_log());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &cfg,
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        let caps: Vec<u64> = h.timeouts.borrow().iter().map(Duration::as_secs).collect();
        assert_eq!(caps.len(), 4, "begin, pull, run, release: {caps:?}");
        let remote: u64 = caps.iter().sum();
        assert!(remote + JUDGE_OVERHEAD_SECS <= cfg.worst_case_secs());
        assert!(cfg.default_oracle_timeout_secs() > cfg.worst_case_secs());
        let old_default = cfg.run_timeout_secs + 1800;
        assert!(old_default < remote);
        assert!(cfg.check_oracle_timeout(old_default).is_err());
        assert!(cfg
            .check_oracle_timeout(cfg.default_oracle_timeout_secs())
            .is_ok());
    }

    #[test]
    fn judge_respects_the_disk_floor() {
        // 16 GiB free, a 1 GiB-compressed image needs ~4 GiB, floor 15.
        let h = host("16777216\n", false, &passing_log());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            Some(1 << 30),
        );
        assert_eq!(v.outcome, JudgeOutcome::InfraError);
        assert!(v.reason.unwrap().contains("disk floor"));
        let calls = h.calls.borrow();
        assert!(calls.iter().all(|c| !c.contains("docker pull")));
        assert!(
            calls.last().unwrap().contains(RELEASE_TAG),
            "the claim is released even though nothing ran"
        );
    }

    #[test]
    fn judge_infra_and_agent_failures_are_distinguished() {
        let mut h = host(PLENTY, false, "");
        h.pull_exit = 1;
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::InfraError);
        assert_eq!(v.outcome.exit_code(), 2);

        let h = host(PLENTY, true, &format!("{APPLY_PATCH_FAIL}\n"));
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Unresolved);
        assert_eq!(v.patch_applied, Some(false));
        assert_eq!(v.outcome.exit_code(), 1);

        let failing = passing_log().replace(
            "messages (forms_tests.tests.test_forms.FormsTestCase) ... ok",
            "messages (forms_tests.tests.test_forms.FormsTestCase) ... FAIL",
        );
        let h = host(PLENTY, true, &failing);
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Unresolved);
        assert_eq!(v.f2p_failures.len(), 1);

        let mut h = host(PLENTY, true, "");
        h.run = RemoteOutput {
            exit: Some(125),
            stderr: "docker: Error response from daemon\n".to_string(),
            ..Default::default()
        };
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::InfraError);

        let mut h = host(PLENTY, true, "");
        h.run = RemoteOutput {
            exit: None,
            timed_out: true,
            ..Default::default()
        };
        let v = judge_instance(
            &inst(),
            &entry(),
            b"diff\n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Unresolved);
        assert_eq!(v.reason.as_deref(), Some("tests timed out"));
    }

    /// The runner reads a SWE-bench oracle's exit 2 as errored; every other
    /// category, and every other exit, keeps its plain reading.
    #[test]
    fn only_a_swe_judge_exit_2_is_an_infrastructure_exit() {
        assert!(is_judge_infra_exit(SWE_CATEGORY, Some(2)));
        assert!(!is_judge_infra_exit(SWE_CATEGORY, Some(1)));
        assert!(!is_judge_infra_exit(SWE_CATEGORY, Some(0)));
        assert!(!is_judge_infra_exit(SWE_CATEGORY, None));
        assert!(!is_judge_infra_exit("classic", Some(2)));
    }

    #[test]
    fn empty_patch_short_circuits_unless_validating() {
        let h = host(PLENTY, true, &passing_log());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"  \n",
            &JudgeConfig::default(),
            JudgeRunOptions::default(),
            &h,
            None,
        );
        assert_eq!(v.outcome, JudgeOutcome::Unresolved);
        assert!(h.calls.borrow().is_empty());
        let v = judge_instance(
            &inst(),
            &entry(),
            b"",
            &JudgeConfig::default(),
            JudgeRunOptions {
                run_empty_patch: true,
            },
            &h,
            None,
        );
        assert_eq!(
            v.outcome,
            JudgeOutcome::Resolved,
            "the fake log passes, so the run happened"
        );
    }

    // ---- the host ledger, run for real against a stand-in docker ----

    /// A stand-in `docker` CLI: images and containers are files under
    /// `$FAKE_DOCKER_STATE`, every call is appended to its `calls` file, and
    /// `rmi` refuses an image a container still uses, as the real one does.
    #[cfg(unix)]
    const FAKE_DOCKER: &str = r#"#!/bin/sh
S="$FAKE_DOCKER_STATE"
echo "$*" >> "$S/calls"
[ -f "$S/down" ] && exit 1
key() { printf '%s' "$1" | tr -c 'A-Za-z0-9' '_'; }
case "$1" in
info) exit 0 ;;
image) [ -f "$S/images/$(key "$3")" ] ;;
pull) : > "$S/images/$(key "$3")" ;;
rmi)
  f="$S/images/$(key "$2")"
  [ -f "$f" ] || exit 1
  for c in "$S"/containers/*; do
    [ -f "$c" ] || continue
    if [ "$(sed -n 1p "$c")" = "$2" ]; then echo "conflict: image is in use" >&2; exit 1; fi
  done
  rm -f "$f" ;;
rm) rm -f "$S/containers/$3" ;;
ps)
  for c in "$S"/containers/*; do
    [ -f "$c" ] || continue
    echo "$(basename "$c") $(sed -n 2p "$c")"
  done ;;
run)
  name=
  img=
  dl=
  prev=
  for a in "$@"; do
    if [ "$prev" = --name ]; then name=$a; fi
    case "$prev=$a" in --label=permagent.swe.deadline=*) dl=${a#*=} ;; esac
    if [ "$a" = bash ]; then img=$prev; fi
    prev=$a
  done
  if [ ! -f "$S/images/$(key "$img")" ]; then echo "No such image: $img" >&2; exit 125; fi
  printf '%s\n%s\n' "$img" "$dl" > "$S/containers/$name"
  cat > /dev/null
  cat "$S/run_stdout" 2>/dev/null
  rm -f "$S/containers/$name" ;;
esac
"#;

    #[cfg(unix)]
    const FAKE_COLIMA: &str = "#!/bin/sh\necho \"colima $*\" >> \"$FAKE_DOCKER_STATE/calls\"\n";

    /// A Docker host simulated in a temp dir: the ledger scripts run for real
    /// under `/bin/sh` with `$HOME` pointed at it and the stand-in `docker`
    /// first on the prelude's PATH.
    #[cfg(unix)]
    struct SimHost {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        state: PathBuf,
        bin: PathBuf,
    }

    #[cfg(unix)]
    impl SimHost {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            let state = tmp.path().join("docker");
            let bin = tmp.path().join("bin");
            for d in [
                home.clone(),
                state.join("images"),
                state.join("containers"),
                bin.clone(),
            ] {
                std::fs::create_dir_all(d).unwrap();
            }
            for (name, text) in [("docker", FAKE_DOCKER), ("colima", FAKE_COLIMA)] {
                let path = bin.join(name);
                std::fs::write(&path, text).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            Self {
                _tmp: tmp,
                home,
                state,
                bin,
            }
        }

        fn cfg(&self) -> JudgeConfig {
            JudgeConfig {
                remote_path_prefix: self.bin.to_string_lossy().into_owned(),
                min_free_gib: 0.0,
                ledger: LedgerTiming {
                    lock_wait_secs: 1,
                    lock_stale_mins: 5,
                },
                ..JudgeConfig::default()
            }
        }

        fn ledger_dir(&self, cfg: &JudgeConfig) -> PathBuf {
            self.home
                .join(".permagent-swe")
                .join(file_key(&cfg.docker_host))
        }

        /// List `image` in the ledger, as a judge that pulled it would.
        fn list_in_ledger(&self, cfg: &JudgeConfig, image: &str) {
            let images = self.ledger_dir(cfg).join("images");
            std::fs::create_dir_all(&images).unwrap();
            std::fs::write(images.join(file_key(image)), format!("{image}\n")).unwrap();
        }

        fn listed_in_ledger(&self, cfg: &JudgeConfig, image: &str) -> bool {
            self.ledger_dir(cfg)
                .join("images")
                .join(file_key(image))
                .exists()
        }

        fn claim(&self, cfg: &JudgeConfig, judge: &str, deadline: u64, image: &str) {
            let claims = self.ledger_dir(cfg).join("claims");
            std::fs::create_dir_all(&claims).unwrap();
            std::fs::write(claims.join(judge), format!("{deadline} {image}\n")).unwrap();
        }

        fn add_image(&self, image: &str) {
            std::fs::write(self.state.join("images").join(file_key(image)), "").unwrap();
        }

        fn has_image(&self, image: &str) -> bool {
            self.state.join("images").join(file_key(image)).exists()
        }

        fn set_run_stdout(&self, text: &str) {
            std::fs::write(self.state.join("run_stdout"), text).unwrap();
        }

        fn docker_calls(&self) -> String {
            std::fs::read_to_string(self.state.join("calls")).unwrap_or_default()
        }

        fn count(dir: &Path) -> usize {
            std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
        }
    }

    #[cfg(unix)]
    impl RemoteExec for SimHost {
        fn run(&self, command: &str, stdin: Option<&str>, _t: Duration) -> Result<RemoteOutput> {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .env("HOME", &self.home)
                .env("FAKE_DOCKER_STATE", &self.state)
                .stdin(if stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
                let _ = pipe.write_all(text.as_bytes());
            }
            let out = child.wait_with_output()?;
            Ok(RemoteOutput {
                exit: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                timed_out: false,
            })
        }
    }

    #[cfg(unix)]
    fn sim_judge(sim: &SimHost, cfg: &JudgeConfig) -> JudgeVerdict {
        judge_instance(
            &inst(),
            &entry(),
            b"diff --git a/x b/x\n",
            cfg,
            JudgeRunOptions::default(),
            sim,
            Some(1 << 20),
        )
    }

    #[cfg(unix)]
    #[test]
    fn judge_removes_the_image_it_pulled_and_leaves_no_ledger_state() {
        let sim = SimHost::new();
        sim.set_run_stdout(&passing_log());
        let cfg = sim.cfg();
        let image = entry().image_ref();
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert!(v.pulled);
        assert!(!sim.has_image(&image), "{}", sim.docker_calls());
        let ledger = sim.ledger_dir(&cfg);
        assert_eq!(SimHost::count(&ledger.join("images")), 0);
        assert_eq!(SimHost::count(&ledger.join("claims")), 0);
        assert!(!ledger.join("lock").exists(), "the lock is released");
        let calls = sim.docker_calls();
        assert!(calls.contains("run --rm -i --pull=never"), "{calls}");
        assert!(calls.contains("colima ssh -- sudo fstrim"), "{calls}");
    }

    /// An image the host already had is listed in no ledger, so no judge
    /// ever removes it. (This test used to assert that no command sent to a
    /// fake host even mentioned `docker rmi`; the removal decision now runs
    /// on the host against the ledger, so it checks the image itself.)
    #[cfg(unix)]
    #[test]
    fn judge_never_removes_an_image_it_did_not_pull() {
        let sim = SimHost::new();
        sim.set_run_stdout(&passing_log());
        let cfg = sim.cfg();
        let image = entry().image_ref();
        sim.add_image(&image);
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert!(!v.pulled);
        assert!(sim.has_image(&image));
        let calls = sim.docker_calls();
        assert!(
            calls
                .lines()
                .all(|l| !l.starts_with("pull ") && !l.starts_with("rmi ")),
            "{calls}"
        );
    }

    /// Two judges of one image overlap (two tiers over the same tasks): A
    /// pulls, B finds it present. Whoever releases last removes it, whoever
    /// pulled it; before, A's `rmi` failed under B's container and B never
    /// removed an image it had not pulled, so it stayed for good.
    #[cfg(unix)]
    #[test]
    fn overlapping_judges_the_last_to_release_removes_the_image() {
        let sim = SimHost::new();
        let cfg = sim.cfg();
        let image = entry().image_ref();
        let step = Duration::from_secs(60);
        let a = HostLedger::new(&cfg, "django__django-11880", &image);
        let b = HostLedger::new(&cfg, "django__django-11880", &image);
        assert!(!a.begin(&cfg, &sim).unwrap().present);
        assert_eq!(
            sim.run(&a.pull_command(&cfg), None, step).unwrap().exit,
            Some(0)
        );
        assert!(sim.listed_in_ledger(&cfg, &image), "listed before the pull");
        let begin_b = b.begin(&cfg, &sim).unwrap();
        assert!(begin_b.present);
        assert!(begin_b.reaped.is_empty(), "A's claim protects its image");
        sim.run(&a.release_command(&cfg), None, step).unwrap();
        assert!(sim.has_image(&image), "B still claims it");
        sim.run(&b.release_command(&cfg), None, step).unwrap();
        assert!(!sim.has_image(&image), "the last judge out removes it");
        assert!(!sim.listed_in_ledger(&cfg, &image));
    }

    /// A judge killed mid-run (Ctrl-C of `run`, a runner timeout) leaves its
    /// container and its pulled image behind. The next judge reaps both once
    /// the dead judge's claim has expired, and leaves a live judge's alone.
    #[cfg(unix)]
    #[test]
    fn a_killed_judges_leftovers_are_reaped_by_the_next_judge() {
        let sim = SimHost::new();
        sim.set_run_stdout(&passing_log());
        let cfg = sim.cfg();
        let dead = format!("ghcr.io/example/dead@sha256:{}", "d".repeat(64));
        sim.add_image(&dead);
        sim.list_in_ledger(&cfg, &dead);
        sim.claim(&cfg, "dead-judge", 100, &dead);
        std::fs::write(
            sim.state.join("containers").join("permagent-swe-dead"),
            format!("{dead}\n100\n"),
        )
        .unwrap();
        let live = format!("ghcr.io/example/live@sha256:{}", "e".repeat(64));
        sim.add_image(&live);
        sim.list_in_ledger(&cfg, &live);
        sim.claim(&cfg, "live-judge", 99_999_999_999, &live);
        sim.add_image(&entry().image_ref());

        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert!(!sim.has_image(&dead), "{}", sim.docker_calls());
        assert!(!sim.listed_in_ledger(&cfg, &dead));
        assert!(!sim.state.join("containers/permagent-swe-dead").exists());
        let claims = sim.ledger_dir(&cfg).join("claims");
        assert!(!claims.join("dead-judge").exists());
        assert!(v
            .reaped
            .contains(&"container permagent-swe-dead".to_string()));
        assert!(v.reaped.contains(&format!("image {dead}")));
        assert!(sim.has_image(&live), "a live claim is never reaped");
        assert!(sim.listed_in_ledger(&cfg, &live));
        assert!(claims.join("live-judge").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_held_lock_makes_the_judge_give_up_and_a_stale_one_is_broken() {
        let sim = SimHost::new();
        sim.set_run_stdout(&passing_log());
        let cfg = sim.cfg();
        sim.add_image(&entry().image_ref());
        let lock = sim.ledger_dir(&cfg).join("lock");
        std::fs::create_dir_all(&lock).unwrap();
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::InfraError);
        assert!(
            v.reason.as_deref().unwrap_or("").contains("locked"),
            "{:?}",
            v.reason
        );
        assert!(lock.exists(), "another judge's lock is left alone");
        // The same lock, left by a judge that died holding it.
        let st = Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(&lock)
            .status()
            .unwrap();
        assert!(st.success());
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert!(!lock.exists());
    }

    /// With Docker down every `image inspect` fails; the reaper must not read
    /// that as "the image is gone" and forget what the ledger lists.
    #[cfg(unix)]
    #[test]
    fn an_unreachable_docker_is_an_infra_error_that_keeps_the_ledger() {
        let sim = SimHost::new();
        let cfg = sim.cfg();
        let listed = format!("ghcr.io/example/listed@sha256:{}", "f".repeat(64));
        sim.list_in_ledger(&cfg, &listed);
        std::fs::write(sim.state.join("down"), "").unwrap();
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::InfraError);
        assert!(
            v.reason
                .as_deref()
                .unwrap_or("")
                .contains("docker unreachable"),
            "{:?}",
            v.reason
        );
        assert!(sim.listed_in_ledger(&cfg, &listed));
        assert!(!sim.ledger_dir(&cfg).join("lock").exists());
    }

    /// `--keep-image`: the pulled image is never listed, so neither this
    /// judge nor any later one removes it.
    #[cfg(unix)]
    #[test]
    fn keep_image_never_lists_what_it_pulls() {
        let sim = SimHost::new();
        sim.set_run_stdout(&passing_log());
        let cfg = JudgeConfig {
            keep_image: true,
            ..sim.cfg()
        };
        let image = entry().image_ref();
        let v = sim_judge(&sim, &cfg);
        assert_eq!(v.outcome, JudgeOutcome::Resolved, "{:?}", v.reason);
        assert!(v.pulled);
        assert!(sim.has_image(&image));
        assert!(!sim.listed_in_ledger(&cfg, &image));
        assert!(sim_judge(&sim, &cfg).reaped.is_empty());
        assert!(sim.has_image(&image));
    }

    // ---- workspace patch (real git, temp dir) ----

    #[test]
    fn workspace_patch_includes_untracked_and_skips_harness_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        std::fs::write(ws.join("a.py"), "x = 1\n").unwrap();
        let seed = crate::runner::seed_git_repo(ws, "t").unwrap();
        std::fs::write(ws.join("a.py"), "x = 2\n").unwrap();
        std::fs::write(ws.join("new.py"), "y = 1\n").unwrap();
        std::fs::create_dir_all(ws.join(".permagent-goal-worktrees/g1")).unwrap();
        std::fs::write(ws.join(".permagent-goal-worktrees/g1/junk.py"), "z\n").unwrap();
        std::fs::create_dir_all(ws.join(".permagent")).unwrap();
        std::fs::write(ws.join(".permagent/state.json"), "{}\n").unwrap();
        // A commit made by the harness (auto-land) must still be in the diff.
        std::fs::write(ws.join("b.py"), "b = 1\n").unwrap();
        let wss = ws.to_string_lossy().into_owned();
        let st = Command::new("git")
            .args(["-C", wss.as_str(), "add", "b.py"])
            .status()
            .unwrap();
        assert!(st.success());
        let st = Command::new("git")
            .args([
                "-C",
                wss.as_str(),
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "landed",
            ])
            .status()
            .unwrap();
        assert!(st.success());
        let wp = workspace_patch(ws).unwrap();
        assert_eq!(wp.seed_commit, seed, "diffed against the runner's seed");
        let patch = String::from_utf8(wp.bytes).unwrap();
        assert!(patch.contains("diff --git a/a.py b/a.py"));
        assert!(patch.contains("+x = 2"));
        assert!(patch.contains("diff --git a/new.py b/new.py"));
        assert!(patch.contains("diff --git a/b.py b/b.py"));
        assert!(!patch.contains(".permagent"));
    }

    #[test]
    fn workspace_patch_is_empty_for_an_untouched_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
        crate::runner::seed_git_repo(dir.path(), "t").unwrap();
        assert!(workspace_patch(dir.path())
            .unwrap()
            .bytes
            .iter()
            .all(u8::is_ascii_whitespace));
    }

    /// A text file in another encoding is diffed as raw bytes. Decoding that
    /// lossily turned 0xE9 into EF BF BD, so the patch no longer matched
    /// `/testbed` and a correct fix was graded "did not apply".
    #[test]
    fn workspace_patch_keeps_non_utf8_bytes_so_it_still_applies() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let before: &[u8] = b"# -*- coding: latin-1 -*-\n# caf\xe9\nx = 1\n";
        let after: &[u8] = b"# -*- coding: latin-1 -*-\n# caf\xe9\nx = 2\n";
        std::fs::write(ws.join("latin1.py"), before).unwrap();
        crate::runner::seed_git_repo(&ws, "t").unwrap();
        std::fs::write(ws.join("latin1.py"), after).unwrap();
        let patch = workspace_patch(&ws).unwrap().bytes;
        assert!(
            patch.windows(2).any(|w| w == b"\xe9\n"),
            "the raw byte survives"
        );
        assert!(!patch.windows(3).any(|w| w == "\u{fffd}".as_bytes()));
        // It applies to a pristine copy byte for byte, as `/testbed` sees it.
        let testbed = dir.path().join("testbed");
        std::fs::create_dir_all(&testbed).unwrap();
        std::fs::write(testbed.join("latin1.py"), before).unwrap();
        let patch_file = dir.path().join("agent.patch");
        std::fs::write(&patch_file, &patch).unwrap();
        let st = Command::new("git")
            .arg("-C")
            .arg(&testbed)
            .args(["apply", "--verbose"])
            .arg(&patch_file)
            .status()
            .unwrap();
        assert!(st.success(), "git apply rejected the patch");
        assert_eq!(std::fs::read(testbed.join("latin1.py")).unwrap(), after);
        // ...and the judge ships those same bytes into the container.
        assert!(judge_script(&patch, "true").contains(&base64_wrapped(&patch)));
    }

    #[test]
    fn workspace_run_dir_is_the_runners_scratch_dir_name() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir
            .path()
            .join("cheap-django__django-11880-17-0")
            .join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        assert_eq!(
            workspace_run_dir(&ws).as_deref(),
            Some("cheap-django__django-11880-17-0")
        );
    }

    /// A judge that cannot even diff the workspace (the harness was killed
    /// mid auto-land and left `.git/index.lock`) still has an InfraError
    /// verdict keyed to its run, which the summary reads as a judge error.
    #[test]
    fn a_workspace_the_judge_cannot_diff_is_a_keyed_infra_error() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir
            .path()
            .join("cheap-sympy__sympy-13877-17-0")
            .join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.py"), "x = 1\n").unwrap();
        let seed = crate::runner::seed_git_repo(&ws, "t").unwrap();
        std::fs::write(ws.join("a.py"), "x = 2\n").unwrap();
        std::fs::write(ws.join(".git").join("index.lock"), "").unwrap();
        let err = workspace_patch(&ws).unwrap_err();
        let v =
            JudgeVerdict::infra_error_for_workspace("sympy__sympy-13877", format!("{err:#}"), &ws);
        assert_eq!(v.outcome, JudgeOutcome::InfraError);
        assert_eq!(v.outcome.exit_code(), 2);
        assert!(v.reason.as_deref().unwrap().contains("index.lock"));
        assert_eq!(v.run_dir.as_deref(), Some("cheap-sympy__sympy-13877-17-0"));
        assert_eq!(v.seed_commit.as_deref(), Some(seed.as_str()));

        // Written and read back like any other verdict...
        let verdicts = dir.path().join("verdicts");
        write_verdict(&verdicts, &v).unwrap();
        let loaded = load_verdicts(&verdicts);
        assert_eq!(loaded, vec![v]);
        // ...it marks this run's row, which the runner graded from exit 2.
        let report = serde_json::json!({"tiers": [{"tier": "cheap", "model": "m", "tasks": [
            {"task_id": "sympy__sympy-13877", "category": "swe-bench", "solved": false,
             "oracle": "errored", "oracle_exit": 2, "oracle_timed_out": false,
             "cost_usd": 0.1, "ledger_rows": 3, "duration_secs": 10.0, "oracle_secs": 1.0,
             "seed_commit": seed}]}]});
        assert_eq!(summarize_report(&report, None, &loaded)[0].infra_errors, 1);
    }

    // ---- summary ----

    #[test]
    fn summary_computes_dollars_per_solved_calls_and_wall() {
        let report = serde_json::json!({"tiers": [{
        "tier": "cheap", "model": "m",
        "tasks": [
            {"task_id": "django__django-11880", "category": "swe-bench", "solved": true, "oracle": "pass",
             "cost_usd": 0.02, "ledger_rows": 12, "duration_secs": 100.0, "oracle_secs": 30.0},
            {"task_id": "sympy__sympy-13877", "category": "swe-bench", "solved": false, "oracle": "fail",
             "cost_usd": 0.04, "ledger_rows": 30, "duration_secs": 300.0, "oracle_secs": 10.0,
             "seed_commit": "s2"},
            {"task_id": "fizzbuzz", "category": "classic", "solved": true, "oracle": "pass",
             "cost_usd": 1.0, "ledger_rows": 1, "duration_secs": 1.0, "oracle_secs": 1.0},
            {"task_id": "x", "category": "swe-bench", "solved": false, "oracle": "not_run",
             "cost_usd": null, "ledger_rows": 0, "duration_secs": 0.0, "oracle_secs": 0.0}
        ]}]});
        // The verdict this very run's judge wrote (same tier, same seed).
        let verdicts = [JudgeVerdict {
            outcome: JudgeOutcome::InfraError,
            run_dir: Some("cheap-sympy__sympy-13877-123".to_string()),
            seed_commit: Some("s2".to_string()),
            ..JudgeVerdict::new(
                &{
                    let mut i = inst();
                    i.instance_id = "sympy__sympy-13877".to_string();
                    i
                },
                &entry(),
                b"",
            )
        }];
        let s = summarize_report(&report, None, &verdicts);
        assert_eq!(s.len(), 1);
        let t = &s[0];
        assert_eq!(t.rows.len(), 2, "non-SWE and not-run tasks are excluded");
        assert!((t.dollars_per_solved.unwrap() - 0.06).abs() < 1e-9);
        assert!((t.total_cost_usd.unwrap() - 0.06).abs() < 1e-9);
        assert_eq!(t.median_model_calls, Some(21.0));
        assert_eq!(t.median_wall_secs, Some(220.0));
        assert_eq!(t.infra_errors, 1);
        let md = render_summary_markdown(&s);
        assert!(md.contains("| cheap | m | 1/2 | 50.0% | $0.0600 |"));
        assert!(md.contains("judge error"));
    }

    #[test]
    fn summary_dollars_per_solved_unknown_when_nothing_solved_or_cost_unknown() {
        let report = serde_json::json!({"tiers": [{"tier": "t", "model": "m", "tasks": [
            {"task_id": "a", "category": "swe-bench", "solved": false, "oracle": "fail", "cost_usd": 0.5,
             "ledger_rows": 3, "duration_secs": 1.0, "oracle_secs": 1.0}]}]});
        assert_eq!(
            summarize_report(&report, None, &[])[0].dollars_per_solved,
            None
        );
        let report = serde_json::json!({"tiers": [{"tier": "t", "model": "m", "tasks": [
            {"task_id": "a", "category": "swe-bench", "solved": true, "oracle": "pass", "cost_usd": null,
             "ledger_rows": 3, "duration_secs": 1.0, "oracle_secs": 1.0}]}]});
        assert_eq!(
            summarize_report(&report, None, &[])[0].dollars_per_solved,
            None
        );
    }

    /// One graded row of a `run --format json` report.
    fn graded_row(task_id: &str, solved: bool, exit: i64, seed_commit: &str) -> serde_json::Value {
        let oracle = if solved { "pass" } else { "fail" };
        serde_json::json!({"task_id": task_id, "category": "swe-bench", "solved": solved,
            "oracle": oracle, "oracle_exit": exit,
            "oracle_timed_out": false, "cost_usd": 0.1, "ledger_rows": 3,
            "duration_secs": 10.0, "oracle_secs": 1.0, "seed_commit": seed_commit})
    }

    /// Every run over one tasks dir writes into the same verdict dir. An
    /// InfraError an earlier run of the same tier and task left there must
    /// not label a later run of it; only that run's own verdict (same seed
    /// commit) does. Before, a `<tier>-<task>-` prefix match marked round 2's
    /// real failure, and even its solve, as judge errors.
    #[test]
    fn summary_keys_judge_errors_to_the_run_not_the_task() {
        let report = serde_json::json!({"tiers": [{"tier": "cheap", "model": "m", "tasks": [
            graded_row("sympy__sympy-13877", false, 1, "round2-sympy"),
            graded_row("django__django-11880", true, 0, "round2-django")]}]});
        let round1 = |id: &str| JudgeVerdict {
            run_dir: Some(format!("cheap-{id}-1700000000000000000-0")),
            seed_commit: Some(format!("round1-{id}")),
            ..JudgeVerdict::infra_error(id, "disk floor: 14.9 GiB free")
        };
        // A verdict from before verdicts carried a seed commit matches nothing.
        let legacy = JudgeVerdict {
            run_dir: Some("cheap-sympy__sympy-13877-1600000000000000000-0".to_string()),
            ..JudgeVerdict::infra_error("sympy__sympy-13877", "ssh down")
        };
        let stale = vec![
            round1("sympy__sympy-13877"),
            round1("django__django-11880"),
            legacy,
        ];
        let s = summarize_report(&report, None, &stale);
        assert_eq!(s[0].infra_errors, 0);
        assert!(s[0].rows.iter().all(|r| !r.infra_error));
        assert!(!render_summary_markdown(&s).contains("judge error"));

        // The same run's own verdict still marks it; another tier's does not.
        let mut verdicts = stale;
        verdicts.push(JudgeVerdict {
            run_dir: Some("other-sympy__sympy-13877-1800000000000000000-0".to_string()),
            seed_commit: Some("round2-sympy".to_string()),
            ..JudgeVerdict::infra_error("sympy__sympy-13877", "ssh down")
        });
        assert_eq!(
            summarize_report(&report, None, &verdicts)[0].infra_errors,
            0
        );
        verdicts.push(JudgeVerdict {
            run_dir: Some("cheap-sympy__sympy-13877-1800000000000000000-0".to_string()),
            seed_commit: Some("round2-sympy".to_string()),
            ..JudgeVerdict::infra_error("sympy__sympy-13877", "ssh down")
        });
        let s = summarize_report(&report, None, &verdicts);
        assert_eq!(s[0].infra_errors, 1);
        assert!(s[0].rows[0].infra_error, "{:?}", s[0].rows[0]);
        assert!(
            !s[0].rows[1].infra_error,
            "a solved row is never a judge error"
        );
    }

    /// A row's own oracle exit says when its judge could not grade (exit 2,
    /// or the runner's oracle ceiling), with no verdict dir at all.
    #[test]
    fn summary_reads_a_rows_own_judge_exit() {
        let mut timed_out = graded_row("psf__requests-1142", false, 0, "s3");
        timed_out["oracle_exit"] = serde_json::Value::Null;
        timed_out["oracle_timed_out"] = serde_json::json!(true);
        let report = serde_json::json!({"tiers": [{"tier": "cheap", "model": "m", "tasks": [
            graded_row("sympy__sympy-13877", false, 2, "s1"),
            graded_row("django__django-11880", false, 1, "s2"),
            timed_out]}]});
        let s = summarize_report(&report, None, &[]);
        let marked: Vec<bool> = s[0].rows.iter().map(|r| r.infra_error).collect();
        assert_eq!(marked, vec![true, false, true]);
        assert_eq!(s[0].infra_errors, 2);
    }
}
