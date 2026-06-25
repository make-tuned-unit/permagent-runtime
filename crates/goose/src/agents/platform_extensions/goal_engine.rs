//! Goal dispatch engines (#59).
//!
//! `dispatch_goal` selects a worker, then hands the goal to a [`GoalEngine`]
//! chosen by the worker's engine kind. The engine owns *how* a goal is run; the
//! orchestrator owns the card lifecycle (selection, baseline commit, state
//! transitions, completion tracking) around it.
//!
//! Slice 0 extracts the original in-process subagent body verbatim into
//! [`InternalSubagentEngine`]. Slice 1 adds [`ExternalCliEngine`], which spawns
//! an external agentic CLI (Claude Code, Codex) in an isolated git worktree.
//!
//! Every engine satisfies one completion contract: it returns a [`JoinHandle`]
//! that resolves to a [`GoalOutcome`], which the dispatch tracker routes to
//! `handle_goal_completion` (success / retriable failure) or — for `TimedOut` —
//! `handle_goal_timeout` (park as an unblock decision, never a silent retry).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::agents::subagent_handler::{run_subagent_task, SubagentRunParams};
use crate::agents::subagent_task_config::TaskConfig;
use crate::agents::{AgentRunnerConfig, GoosePlatform};
use crate::config::permission::PermissionManager;
use crate::config::{ExtensionConfig, GooseMode};
use crate::providers;
use crate::providers::base::Provider;
use crate::recipe::Recipe;
use crate::session::session_manager::SessionType;
use crate::session::SessionManager;
use crate::subprocess::configure_subprocess;

/// Default wall-clock bound for a single external-CLI dispatch. On expiry the
/// goal is PARKED (unblock decision), never retried.
pub const DEFAULT_EXTERNAL_CLI_TIMEOUT_SECS: u64 = 30 * 60;

/// Literal token in an external worker's arg template, replaced with the goal
/// prompt at dispatch time. Everything else passes through verbatim.
pub const PROMPT_TOKEN: &str = "{prompt}";

/// Terminal outcome of a dispatched goal, produced by every engine.
#[derive(Debug)]
pub enum GoalOutcome {
    /// Worker finished cleanly; the work product is in the working dir.
    /// Carries deterministic verification evidence (commit SHAs, diffstat,
    /// push target, worker summary) when the engine can produce it — the
    /// external-CLI worktree path always can; the in-process subagent yields
    /// `None` (no worktree / push model).
    Success(Option<GoalEvidence>),
    /// Retriable failure within budget — routes through the existing
    /// budget/retry logic in `handle_goal_completion`.
    Failed(String),
    /// The worker exceeded its time bound. Routes to an unconditional PARK
    /// (`handle_goal_timeout`) — never a silent retry.
    TimedOut { secs: u64 },
}

/// Deterministic proof-of-work captured at goal completion, persisted to the
/// goal card and surfaced in the Decision Inbox Evidence panel + the
/// Discuss-with-Henry context. Every field is derived from git in the worker's
/// own worktree (or the worker's stdout) — zero LLM, zero guessing. This is the
/// evidence a reviewer needs to trust a dispatched goal without manually
/// running git against the (stale) local main.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GoalEvidence {
    /// Absolute path to the isolated worktree the worker committed in.
    pub worktree_path: String,
    /// Dispatch-time HEAD the work was branched off (the diff baseline).
    pub baseline_commit: String,
    /// Short SHA of the worktree HEAD after the worker exited, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    /// Commits the worker produced (`baseline..HEAD`), newest first, as
    /// `"<short-sha> <subject>"`. Empty when the worker committed nothing.
    #[serde(default)]
    pub commits: Vec<String>,
    /// `git diff --stat baseline..HEAD`, truncated. Human-readable diffstat.
    #[serde(default)]
    pub diffstat: String,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
    /// Remote ref the work was pushed to (e.g. `"origin/main"`), or `None`
    /// when the commits live only in the worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_target: Option<String>,
    /// Tail of the worker's own stdout — its final summary of what it did
    /// (Layer 2a self-report). Empty if the worker printed nothing.
    #[serde(default)]
    pub worker_summary: String,
}

/// Per-goal data handed to an engine. Engine-specific capabilities (provider,
/// session manager, CLI binary) live on the engine struct, not here.
pub struct GoalTask {
    pub card_title: String,
    /// Fully-formatted goal instructions (goal / description / project / root).
    pub instructions: String,
    pub working_dir: PathBuf,
    /// Repo HEAD at dispatch time (recorded by the orchestrator). The external
    /// engine branches a worktree off this; `None` when the working dir is not
    /// a git repo (external dispatch then fails fast).
    pub baseline_commit: Option<String>,
    pub timeout: Duration,
}

/// What an engine returns once the goal is spawned: a stable run identifier
/// (recorded in card metadata as the worker session id), the join handle the
/// tracker awaits, and a [`GoalKill`] handle the cancel path uses to stop the
/// worker on demand (#490).
pub struct DispatchedWork {
    pub run_id: String,
    pub join: JoinHandle<GoalOutcome>,
    pub kill: GoalKill,
}

/// Handle to stop a dispatched goal's worker (#490). The orchestrator holds one
/// per in-flight goal in a process-global registry keyed by card id; the cancel
/// path takes it and calls [`kill`](GoalKill::kill) before marking the goal
/// Cancelled.
pub enum GoalKill {
    /// External CLI: the worker runs in its own process group (pgid == the
    /// leader pid). SIGKILLing the whole group reaps the CLI *and* any tool
    /// subprocesses it spawned — killing just the leader would orphan them.
    ProcessGroup(u32),
    /// In-process subagent: cooperative cancellation token.
    Cancel(CancellationToken),
    /// No handle available (the child never reported a pid). Cancel still marks
    /// the goal terminal; a still-running worker no-ops at completion (its card
    /// has already left in_progress) and dies on the dispatch timeout.
    None,
}

impl GoalKill {
    /// Best-effort stop: SIGKILL the process group (external) or fire the
    /// cancellation token (in-process). Safe to call once per dispatch.
    pub fn kill(&self) {
        match self {
            GoalKill::ProcessGroup(pid) => kill_process_group(*pid),
            GoalKill::Cancel(token) => token.cancel(),
            GoalKill::None => {}
        }
    }
}

/// SIGKILL an entire process group by pgid (== the group leader's pid). The
/// negative argument targets the group. Unix only; a no-op elsewhere
/// (`kill_on_drop` still reaps the direct child).
#[cfg(unix)]
pub(crate) fn kill_process_group(pid: u32) {
    // SIGKILL, not SIGTERM: the worker is being abandoned, not asked to wind
    // down. Unsafe FFI; the worst case of a stale/reused pid is a no-op because
    // a fresh group with that leader pid is astronomically unlikely mid-cancel.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_process_group(_pid: u32) {}

#[async_trait]
pub trait GoalEngine: Send + Sync {
    /// Spawn the goal. Returns immediately with a handle the orchestrator's
    /// completion tracker awaits. `Err` here means the goal never started
    /// (card stays Ready, attempt not consumed).
    async fn spawn(&self, task: GoalTask) -> Result<DispatchedWork, String>;
}

// ── Internal subagent engine (Slice 0 — extracted verbatim) ───────────────

/// Runs the goal as an in-process subagent on the parent session's provider.
/// Behaviour is identical to the pre-refactor `dispatch_goal` body.
pub struct InternalSubagentEngine {
    pub session_manager: Arc<SessionManager>,
    pub provider: Arc<dyn Provider>,
    pub extensions: Vec<ExtensionConfig>,
    /// `(system_prompt_block, display_name)` prepended to the subagent prompt.
    pub persona_override: Option<(String, String)>,
}

#[async_trait]
impl GoalEngine for InternalSubagentEngine {
    async fn spawn(&self, task: GoalTask) -> Result<DispatchedWork, String> {
        let subagent_session = self
            .session_manager
            .create_session(
                task.working_dir.clone(),
                format!("Goal: {}", task.card_title),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .map_err(|e| format!("Failed to create subagent session: {}", e))?;

        let session_id = subagent_session.id.clone();

        let model_config = self.provider.get_model_config();
        let task_provider = providers::create(
            self.provider.get_name(),
            model_config,
            self.extensions.clone(),
        )
        .await
        .map_err(|e| format!("Failed to create provider for goal dispatch: {}", e))?;

        let task_config = TaskConfig::new(
            task_provider,
            &session_id,
            &task.working_dir,
            self.extensions.clone(),
        );

        let agent_config = AgentRunnerConfig::new(
            self.session_manager.clone(),
            PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true,
            GoosePlatform::GooseCli,
        );

        let recipe = Recipe::builder()
            .version("1.0.0")
            .title(format!("Goal: {}", task.card_title))
            .description("Orchestrator-dispatched goal")
            .prompt(&task.instructions)
            .build()
            .map_err(|e| format!("Failed to build recipe: {}", e))?;

        let persona_override = self.persona_override.clone();
        let cancel_token = CancellationToken::new();
        // The cancel path (#490) fires this token to stop an in-flight subagent.
        let kill = GoalKill::Cancel(cancel_token.clone());
        let run_session_id = session_id.clone();
        let join = tokio::spawn(async move {
            match run_subagent_task(SubagentRunParams {
                config: agent_config,
                recipe,
                task_config,
                return_last_only: true,
                session_id: run_session_id,
                cancellation_token: Some(cancel_token),
                on_message: None,
                notification_tx: None,
                persona_override,
            })
            .await
            {
                Ok(_) => GoalOutcome::Success(None),
                Err(e) => GoalOutcome::Failed(e.to_string()),
            }
        });

        Ok(DispatchedWork {
            run_id: session_id,
            join,
            kill,
        })
    }
}

// ── External CLI engine (Slice 1) ─────────────────────────────────────────

/// Spawns an external agentic CLI (Claude Code, Codex, …) in an isolated git
/// worktree off the goal's baseline commit. The CLI runs its own agentic loop;
/// the commits it leaves in the worktree are the reviewed work product.
pub struct ExternalCliEngine {
    pub bin: String,
    /// Argument template. The literal token [`PROMPT_TOKEN`] is replaced with
    /// the goal prompt; every other arg passes through verbatim.
    pub args: Vec<String>,
    /// `(system_prompt_block, display_name)` prepended to the goal prompt.
    pub persona_override: Option<(String, String)>,
}

impl ExternalCliEngine {
    fn build_prompt(&self, instructions: &str) -> String {
        match &self.persona_override {
            Some((block, _)) if !block.is_empty() => format!("{}\n\n{}", block, instructions),
            _ => instructions.to_string(),
        }
    }
}

#[async_trait]
impl GoalEngine for ExternalCliEngine {
    async fn spawn(&self, task: GoalTask) -> Result<DispatchedWork, String> {
        let baseline = task.baseline_commit.clone().ok_or_else(|| {
            "External-CLI dispatch requires a git baseline commit, but the project root is not a \
             git repository"
                .to_string()
        })?;

        let run_id = format!("cli-{}", uuid::Uuid::new_v4());
        let worktree = create_goal_worktree(&task.working_dir, &baseline, &run_id).await?;

        let prompt = self.build_prompt(&task.instructions);
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| {
                if a == PROMPT_TOKEN {
                    prompt.clone()
                } else {
                    a.clone()
                }
            })
            .collect();
        let bin = self.bin.clone();
        let timeout = task.timeout;

        // Spawn the worker NOW (in its own process group) so we can capture its
        // pid for a cancel/timeout group-kill before handing the wait off to the
        // tracker task. A spawn failure here means the goal never started.
        let mut cmd = build_cli_command(&bin, &args, &worktree);
        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed to run `{}`: {}", bin, e))?;
        let kill = match child.id() {
            Some(pid) => GoalKill::ProcessGroup(pid),
            None => GoalKill::None,
        };
        let pid = child.id();

        let join = tokio::spawn(async move {
            await_external_child(child, pid, bin, worktree, baseline, timeout).await
        });

        Ok(DispatchedWork { run_id, join, kill })
    }
}

/// Create a detached git worktree at
/// `<repo>/../.permagent-goal-worktrees/<run_id>` checked out at `baseline`.
/// Returns the worktree path. The worktree is intentionally *not* removed on
/// completion — its commits are the work product the Decision Inbox review
/// points to.
async fn create_goal_worktree(
    repo: &Path,
    baseline: &str,
    run_id: &str,
) -> Result<PathBuf, String> {
    let base_dir = repo
        .parent()
        .unwrap_or(repo)
        .join(".permagent-goal-worktrees");
    let dest = base_dir.join(run_id);

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(&dest)
        .arg(baseline)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_subprocess(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run `git worktree add`: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "`git worktree add` failed: {}",
            tail(
                &redact_secrets(&String::from_utf8_lossy(&output.stderr)),
                2000
            )
        ));
    }
    Ok(dest)
}

/// Directory (under the repo's parent) holding every goal worktree, keyed by
/// run id. Single source of truth for both creation and reaping (#504).
pub(crate) const GOAL_WORKTREES_DIR: &str = ".permagent-goal-worktrees";

/// `<repo_parent>/.permagent-goal-worktrees`, mirroring [`create_goal_worktree`].
fn goal_worktrees_dir(repo: &Path) -> PathBuf {
    repo.parent().unwrap_or(repo).join(GOAL_WORKTREES_DIR)
}

/// Outcome of a worktree reap attempt (#504). Returned for logging/observability
/// and asserted in tests; never an error — reaping is best-effort by contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapOutcome {
    /// `git worktree remove` succeeded (git-tracked worktree).
    RemovedTracked,
    /// `git worktree remove` failed; the dir was deleted via `rm -rf` +
    /// `git worktree prune` (an orphaned dir git had lost the ref to).
    RemovedOrphaned,
    /// Nothing on disk to remove.
    Absent,
    /// Kept on purpose: unpushed commits present (or push state unprovable) and
    /// removal was not force-allowed. Protects unreviewed work — see #504.
    SkippedUnpushed,
    /// Removal was attempted but the dir still exists afterwards.
    Failed,
}

/// Run `git <args>` in `dir`; `Some(trimmed stdout)` on a clean exit (possibly
/// empty), `None` if git failed to launch or exited non-zero. Unlike
/// [`git_text`], this distinguishes "succeeded with empty output" from "failed",
/// which the push-safety guard depends on.
async fn git_checked(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => None,
    }
}

/// Does this worktree hold commits not present on any remote (unpushed work)?
///
/// - `Some(true)`  — HEAD has commits reachable from no remote-tracking ref.
/// - `Some(false)` — HEAD is fully contained on a remote (or has no commits
///   beyond what remotes already have).
/// - `None`        — git state is unreadable (e.g. an orphaned dir whose
///   worktree admin ref is gone). The caller decides: the on-transition reaper
///   protects (can't prove safety); the sweep treats it as safe because an
///   unreadable admin ref means those commits are already unreachable in the
///   repo regardless.
async fn has_unpushed_work(worktree: &Path) -> Option<bool> {
    // Readability probe: if HEAD won't resolve, the worktree's admin ref is gone.
    git_checked(worktree, &["rev-parse", "--verify", "HEAD"]).await?;
    // Commits reachable from HEAD but from no remote-tracking ref.
    let unpushed = git_checked(worktree, &["rev-list", "HEAD", "--not", "--remotes"]).await?;
    Some(!unpushed.is_empty())
}

/// Two-phase removal handling both #504 leak forms. Phase 1 lets git deregister
/// and delete a tracked worktree (`--force` so a dirty working tree of build
/// artifacts doesn't block it). On any failure, phase 2 deletes the dir directly
/// and prunes the stale admin entry — the orphaned-dir case git no longer tracks.
/// A phase-1 failure must never leave the dir behind.
async fn remove_worktree_dir(repo: &Path, dest: &Path) -> ReapOutcome {
    let dest_str = dest.to_string_lossy().to_string();
    if git_checked(repo, &["worktree", "remove", "--force", &dest_str])
        .await
        .is_some()
        && !dest.exists()
    {
        tracing::info!(
            target: "permagentd::brain",
            "reaper: removed tracked worktree {}",
            dest.display()
        );
        return ReapOutcome::RemovedTracked;
    }

    // Orphaned dir (git remove failed / dir survived): rm -rf, then prune the
    // stale `.git/worktrees/<name>` admin entry git may still list.
    let _ = tokio::fs::remove_dir_all(dest).await;
    if dest.exists() {
        tracing::warn!(
            target: "permagentd::brain",
            "reaper: failed to remove worktree dir {}",
            dest.display()
        );
        return ReapOutcome::Failed;
    }
    let _ = git_checked(repo, &["worktree", "prune"]).await;
    tracing::info!(
        target: "permagentd::brain",
        "reaper: removed orphaned worktree dir {} (git ref lost) + pruned",
        dest.display()
    );
    ReapOutcome::RemovedOrphaned
}

/// Reap a single goal worktree once its goal is terminal (#504).
///
/// `allow_unpushed` is `true` ONLY for Cancelled/abandoned goals — the user has
/// explicitly discarded the work. For Complete goals it stays `false`: unpushed
/// commits are never destroyed (a lingering dir beats deleting unreviewed work),
/// and an unprovable push state is treated as unsafe and kept.
pub async fn reap_goal_worktree(repo: &Path, run_id: &str, allow_unpushed: bool) -> ReapOutcome {
    let dest = goal_worktrees_dir(repo).join(run_id);
    if !dest.exists() {
        return ReapOutcome::Absent;
    }

    if !allow_unpushed {
        match has_unpushed_work(&dest).await {
            Some(true) => {
                tracing::info!(
                    target: "permagentd::brain",
                    "reaper: keeping worktree {} — goal complete but has unpushed commits (not on origin)",
                    dest.display()
                );
                return ReapOutcome::SkippedUnpushed;
            }
            None => {
                tracing::warn!(
                    target: "permagentd::brain",
                    "reaper: keeping worktree {} — cannot evaluate push state, protecting potential unreviewed work",
                    dest.display()
                );
                return ReapOutcome::SkippedUnpushed;
            }
            Some(false) => {}
        }
    }

    remove_worktree_dir(repo, &dest).await
}

/// Resolve a terminal goal's worktree from its project root + run id and reap it
/// (#504). Worktrees live at `<root_parent>/.permagent-goal-worktrees/<run_id>`,
/// mirroring [`create_goal_worktree`]. Fully self-contained and best-effort:
/// every failure is logged, nothing is returned — the caller (a goal-state
/// transition) must never depend on the reap.
pub async fn reap_terminal_goal_worktree(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project_id: &str,
    run_id: &str,
    allow_unpushed: bool,
) {
    let root = match crate::projects::get_project(pool, project_id).await {
        Ok(Some(p)) => p.root_path,
        _ => None,
    };
    let Some(root) = root else {
        return;
    };
    let outcome = reap_goal_worktree(&PathBuf::from(&root), run_id, allow_unpushed).await;
    tracing::info!(
        target: "permagentd::brain",
        "reaper: goal worktree {} terminal reap → {:?} (allow_unpushed={})",
        run_id, outcome, allow_unpushed
    );
}

/// Boot-time sweep reclaiming orphaned goal worktrees left by crashed or prior
/// daemon lifecycles (#504). Each `cli-*` dir under `repo`'s worktrees dir that
/// is NOT in `active_run_ids` is reaped under the push-safety guard: a readable
/// worktree with unpushed commits is kept; an unreadable one (its git admin ref
/// already gone, so its commits are unreachable regardless) is safe to delete.
/// Returns the number of dirs reclaimed.
pub async fn sweep_orphaned_worktrees(repo: &Path, active_run_ids: &[String]) -> usize {
    let dir = goal_worktrees_dir(repo);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return 0, // no worktrees dir — nothing to sweep
    };

    let mut reclaimed = 0usize;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.starts_with("cli-") => n.to_string(),
            _ => continue,
        };
        if active_run_ids.iter().any(|id| id == &name) {
            continue; // belongs to a live (non-terminal) goal — never reap
        }
        // Safety guard (same as on-transition): keep readable worktrees that
        // hold unpushed commits. `None` (unreadable) is a truly-orphaned dir —
        // its commits are already unreachable, so removal is safe.
        if matches!(has_unpushed_work(&path).await, Some(true)) {
            tracing::info!(
                target: "permagentd::brain",
                "sweep: keeping orphaned-candidate {} — has unpushed commits",
                path.display()
            );
            continue;
        }
        match remove_worktree_dir(repo, &path).await {
            ReapOutcome::RemovedTracked | ReapOutcome::RemovedOrphaned => reclaimed += 1,
            _ => {}
        }
    }

    if reclaimed > 0 {
        tracing::info!(
            target: "permagentd::brain",
            "sweep: reclaimed {} orphaned goal worktree(s) under {}",
            reclaimed,
            dir.display()
        );
    }
    reclaimed
}

/// Build the external-CLI command. The worker runs in its own process group
/// (`process_group(0)`, unix) so a cancel or timeout can SIGKILL the whole tree
/// — the CLI plus any tool subprocesses it spawns — not just the group leader.
/// `kill_on_drop` is the backstop that reaps the leader if the wait future is
/// dropped.
fn build_cli_command(bin: &str, args: &[String], working_dir: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    configure_subprocess(&mut cmd);
    cmd
}

/// Await a spawned external-CLI `child`, bounded by `timeout`. Exit 0 →
/// `Success` (with deterministic git evidence collected from `working_dir`
/// against `baseline`); nonzero → `Failed(stderr tail)`; timeout → `TimedOut`
/// (the whole process group is SIGKILLed via `pid`, with `kill_on_drop` reaping
/// the leader as the dropped wait future falls out of scope).
async fn await_external_child(
    child: tokio::process::Child,
    pid: Option<u32>,
    bin: String,
    working_dir: PathBuf,
    baseline: String,
    timeout: Duration,
) -> GoalOutcome {
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                let worker_summary = tail(
                    &redact_secrets(&String::from_utf8_lossy(&output.stdout)),
                    4000,
                );
                let evidence = collect_evidence(&working_dir, &baseline, worker_summary).await;
                GoalOutcome::Success(Some(evidence))
            } else {
                let code = output
                    .status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                GoalOutcome::Failed(format!(
                    "`{}` exited with status {}: {}",
                    bin,
                    code,
                    tail(
                        &redact_secrets(&String::from_utf8_lossy(&output.stderr)),
                        2000
                    )
                ))
            }
        }
        Ok(Err(e)) => GoalOutcome::Failed(format!("Failed to run `{}`: {}", bin, e)),
        Err(_) => {
            // Group-kill so tool subprocesses die too (dropping the wait future
            // only reaps the leader via kill_on_drop).
            if let Some(p) = pid {
                kill_process_group(p);
            }
            GoalOutcome::TimedOut {
                secs: timeout.as_secs(),
            }
        }
    }
}

/// Spawn the external CLI in `working_dir`, bounded by `timeout`, and await it.
/// Convenience wrapper over [`build_cli_command`] + [`await_external_child`]
/// for callers that don't need the kill handle (tests).
#[cfg(test)]
async fn run_external_cli(
    bin: &str,
    args: &[String],
    working_dir: &Path,
    baseline: &str,
    timeout: Duration,
) -> GoalOutcome {
    let mut cmd = build_cli_command(bin, args, working_dir);
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            await_external_child(
                child,
                pid,
                bin.to_string(),
                working_dir.to_path_buf(),
                baseline.to_string(),
                timeout,
            )
            .await
        }
        Err(e) => GoalOutcome::Failed(format!("Failed to run `{}`: {}", bin, e)),
    }
}

/// Collect deterministic verification evidence from the worker's worktree
/// against `baseline`, after a clean exit. Every git call is failure-tolerant:
/// a non-repo dir or a git error degrades the affected field (empty / `None`),
/// never the outcome — a missing diffstat must not turn a success into a
/// failure. Push detection asks whether any remote ref already contains HEAD
/// (the worker's `git push origin HEAD:main` updates the shared repo's
/// `refs/remotes/origin/main`, which the worktree sees).
pub(crate) async fn collect_evidence(
    worktree: &Path,
    baseline: &str,
    worker_summary: String,
) -> GoalEvidence {
    let range = format!("{}..HEAD", baseline);

    let head_commit = git_line(worktree, &["rev-parse", "--short", "HEAD"]).await;
    let commits = git_text(worktree, &["log", "--format=%h %s", &range])
        .await
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>();
    let diffstat = tail(&git_text(worktree, &["diff", "--stat", &range]).await, 4000);
    let shortstat = git_text(worktree, &["diff", "--shortstat", &range]).await;
    let (files_changed, insertions, deletions) = parse_shortstat(&shortstat);
    let push_target = git_text(worktree, &["branch", "-r", "--contains", "HEAD"])
        .await
        .lines()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty());

    GoalEvidence {
        worktree_path: worktree.to_string_lossy().to_string(),
        baseline_commit: baseline.to_string(),
        head_commit,
        commits,
        diffstat,
        files_changed,
        insertions,
        deletions,
        push_target,
        worker_summary,
    }
}

/// Run `git <args>` in `dir`, returning trimmed stdout (empty on any failure).
async fn git_text(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Like [`git_text`] but yields `None` for empty/failed output (single values).
async fn git_line(dir: &Path, args: &[&str]) -> Option<String> {
    let s = git_text(dir, args).await;
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Parse `git diff --shortstat` ("N files changed, X insertions(+), Y deletions(-)")
/// into `(files, insertions, deletions)`. Any absent term is 0.
fn parse_shortstat(s: &str) -> (u32, u32, u32) {
    let num_before = |kw: &str| -> u32 {
        s.split(',')
            .find(|seg| seg.contains(kw))
            .and_then(|seg| {
                seg.split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
            })
            .unwrap_or(0)
    };
    (
        num_before("file"),
        num_before("insertion"),
        num_before("deletion"),
    )
}

/// Roughly the last `max` bytes of `s`, aligned to char boundaries and prefixed
/// with `…` when cut. Slice-free (clippy::string_slice).
fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut kept_bytes = 0;
    let mut kept: Vec<char> = Vec::new();
    for ch in s.chars().rev() {
        kept_bytes += ch.len_utf8();
        if kept_bytes > max {
            break;
        }
        kept.push(ch);
    }
    let tail: String = kept.into_iter().rev().collect();
    format!("…{}", tail)
}

/// Heuristic secret redaction for worker output that gets PERSISTED on the goal
/// card (#455). Worker stdout/stderr is stored verbatim in
/// `dispatch_evidence.worker_summary` and in failure reasons; a publish/seed step
/// that sources `.env.local` (e.g. a DB reseed) can echo a connection string or
/// token into that stream and leak prod credentials into the card. This masks the
/// obvious secret SHAPES — it is best-effort, NOT a proof of absence: an
/// unrecognized bare secret can still slip through, so capture-less is preferable
/// where feasible (the larger publish-step capture is handled that way in #457).
/// Applied uniformly at all three capture points (worker_summary + the two stderr
/// tails). Always redact BEFORE [`tail`] so truncation can't leave a secret
/// fragment that no longer matches a pattern.
fn redact_secrets(s: &str) -> String {
    static RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
        vec![
            // KEY=VALUE assignments whose key names a secret (case-insensitive).
            // Keeps the key, masks the value. `*_URL=` is deliberately included —
            // over-redacting a plain URL beats leaking a credentialed one.
            (
                Regex::new(
                    r"(?i)\b([A-Za-z_][A-Za-z0-9_]*(?:_URL|_KEY|_TOKEN|_SECRET|PASSWORD))\s*=\s*\S+",
                )
                .unwrap(),
                "${1}=[REDACTED]",
            ),
            // Credentialed connection strings (scheme://...): whole URL masked.
            // `mongodb+srv` before `mongodb` so the longer scheme wins.
            (
                Regex::new(r"(?i)\b(?:postgresql|postgres|mysql|mongodb\+srv|mongodb|redis)://\S+")
                    .unwrap(),
                "[REDACTED-CONNECTION-STRING]",
            ),
            // Provider API tokens, JWTs, AWS access-key ids.
            (
                Regex::new(r"\bsk-[A-Za-z0-9_-]{16,}").unwrap(),
                "[REDACTED-TOKEN]",
            ),
            (
                Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap(),
                "[REDACTED-JWT]",
            ),
            (
                Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
                "[REDACTED-AWS-KEY]",
            ),
        ]
    });
    let mut out = s.to_string();
    for (re, repl) in RULES.iter() {
        out = re.replace_all(&out, *repl).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #455: persisted worker output must not leak prod credentials. A stdout/
    /// stderr tail carrying a `postgres://` connection string and a `*_KEY=`
    /// assignment (the shapes a `.env.local`-sourcing seed step emits) comes back
    /// masked, while non-secret context is preserved.
    #[test]
    fn redact_secrets_masks_connection_strings_and_keys() {
        let raw = "Reseeding threads...\n\
                   DATABASE_URL=postgres://admin:hunter2@db.prod.example.com:5432/app\n\
                   Connecting to mongodb+srv://u:p@cluster.mongodb.net/threads\n\
                   export SUPABASE_KEY=sk-LIVEabcdef0123456789XYZ\n\
                   token eyJhbGciOi.eyJzdWIiOi.s3cr3tSig\n\
                   Seeded 412 threads. Done.";
        let out = redact_secrets(raw);

        // Secret material is gone.
        assert!(!out.contains("hunter2"), "db password leaked: {out}");
        assert!(!out.contains("postgres://"), "postgres URL leaked: {out}");
        assert!(
            !out.contains("mongodb+srv://"),
            "mongo connection string leaked: {out}"
        );
        assert!(
            !out.contains("sk-LIVEabcdef0123456789XYZ"),
            "api token leaked: {out}"
        );
        assert!(
            !out.contains("eyJhbGciOi.eyJzdWIiOi.s3cr3tSig"),
            "jwt leaked: {out}"
        );
        // Masks present + non-secret context preserved.
        assert!(
            out.contains("[REDACTED"),
            "expected redaction markers: {out}"
        );
        assert!(
            out.contains("SUPABASE_KEY=[REDACTED]"),
            "key not masked: {out}"
        );
        assert!(
            out.contains("Reseeding threads") && out.contains("Seeded 412 threads. Done."),
            "non-secret context not preserved: {out}"
        );
    }

    #[test]
    fn prompt_token_substitution_replaces_only_the_token() {
        let engine = ExternalCliEngine {
            bin: "claude".to_string(),
            args: vec![
                "-p".to_string(),
                PROMPT_TOKEN.to_string(),
                "--dangerously-skip-permissions".to_string(),
            ],
            persona_override: None,
        };
        let prompt = engine.build_prompt("Implement the thing");
        let resolved: Vec<String> = engine
            .args
            .iter()
            .map(|a| {
                if a == PROMPT_TOKEN {
                    prompt.clone()
                } else {
                    a.clone()
                }
            })
            .collect();
        assert_eq!(
            resolved,
            vec![
                "-p".to_string(),
                "Implement the thing".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ]
        );
    }

    #[test]
    fn build_prompt_prepends_persona_block() {
        let engine = ExternalCliEngine {
            bin: "claude".to_string(),
            args: vec![],
            persona_override: Some((
                "You are Claude Code.".to_string(),
                "Claude Code".to_string(),
            )),
        };
        let prompt = engine.build_prompt("Do the work");
        assert_eq!(prompt, "You are Claude Code.\n\nDo the work");
    }

    #[tokio::test]
    async fn external_cli_times_out_and_reports_timeout() {
        // `sleep 5` bounded by a 1s timeout must yield TimedOut, killing the proc.
        let outcome = run_external_cli(
            "sleep",
            &["5".to_string()],
            Path::new("."),
            "HEAD",
            Duration::from_secs(1),
        )
        .await;
        assert!(
            matches!(outcome, GoalOutcome::TimedOut { secs: 1 }),
            "expected TimedOut, got {:?}",
            outcome
        );
    }

    /// #490: a cancel must actually stop the worker. Spawn a long sleep in its
    /// own process group, group-kill it by pid, and confirm it is reaped fast
    /// (not left running for its full duration).
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_process_group_reaps_the_worker() {
        let mut cmd = build_cli_command("sleep", &["30".to_string()], Path::new("."));
        let mut child = cmd.spawn().expect("spawn sleep");
        let pid = child.id().expect("child has a pid");

        kill_process_group(pid);

        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("killed worker must exit well before its 30s sleep")
            .expect("wait succeeds");
        assert!(!status.success(), "SIGKILLed process is not a clean exit");
    }

    #[tokio::test]
    async fn external_cli_nonzero_exit_is_failure() {
        let outcome = run_external_cli(
            "sh",
            &["-c".to_string(), "exit 3".to_string()],
            Path::new("."),
            "HEAD",
            Duration::from_secs(10),
        )
        .await;
        match outcome {
            GoalOutcome::Failed(msg) => assert!(msg.contains("status 3"), "msg: {}", msg),
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn external_cli_zero_exit_is_success() {
        let outcome =
            run_external_cli("true", &[], Path::new("."), "HEAD", Duration::from_secs(10)).await;
        assert!(
            matches!(outcome, GoalOutcome::Success(_)),
            "expected Success, got {:?}",
            outcome
        );
    }

    #[test]
    fn parse_shortstat_extracts_counts() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 1159 insertions(+), 4 deletions(-)"),
            (3, 1159, 4)
        );
        assert_eq!(
            parse_shortstat(" 1 file changed, 2 insertions(+)"),
            (1, 2, 0)
        );
        assert_eq!(parse_shortstat(""), (0, 0, 0));
    }

    /// End-to-end evidence capture: a worker that commits in its worktree must
    /// surface its commit SHA, diffstat counts, and a not-pushed marker.
    #[tokio::test]
    async fn collect_evidence_captures_commit_and_diffstat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        let baseline = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        // Worker does its work.
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "did the work"]);

        let ev = collect_evidence(&repo, &baseline, "summary".to_string()).await;
        assert_eq!(ev.baseline_commit, baseline);
        assert_eq!(ev.commits.len(), 1, "one commit above baseline");
        assert!(ev.commits[0].contains("did the work"));
        assert!(ev.head_commit.is_some());
        assert_eq!(ev.files_changed, 2);
        assert_eq!(ev.insertions, 2);
        assert!(ev.push_target.is_none(), "no remote → not pushed");
        assert_eq!(ev.worker_summary, "summary");
    }

    /// Defect 2 (worktree isolation): with no git baseline the external engine
    /// must fail fast with a clear reason — never silently fall back to running
    /// the CLI in the user's project root.
    #[tokio::test]
    async fn external_cli_without_baseline_errors_clearly() {
        let engine = ExternalCliEngine {
            bin: "claude".to_string(),
            args: vec![PROMPT_TOKEN.to_string()],
            persona_override: None,
        };
        let task = GoalTask {
            card_title: "t".to_string(),
            instructions: "do it".to_string(),
            working_dir: std::env::temp_dir(),
            baseline_commit: None,
            timeout: Duration::from_secs(10),
        };
        match engine.spawn(task).await {
            Err(err) => assert!(
                err.contains("git"),
                "error must name the missing git baseline, got: {}",
                err
            ),
            Ok(_) => panic!("dispatch without a baseline must fail, not run in the project root"),
        }
    }

    /// Defect 2: the engine runs the CLI in an isolated worktree off the
    /// baseline, not in the project root. Asserts `create_goal_worktree` checks
    /// out the baseline into `.permagent-goal-worktrees/<run_id>` distinct from
    /// the repo, and that the worktree is what gets used as `working_dir`.
    #[tokio::test]
    async fn create_goal_worktree_uses_isolated_path_off_baseline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("proj");
        std::fs::create_dir(&repo).unwrap();

        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        let head = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();

        let run_id = "cli-test-run";
        let worktree = create_goal_worktree(&repo, &head, run_id).await.unwrap();

        assert_ne!(worktree, repo, "worktree must not be the project root");
        assert!(worktree.ends_with(format!(".permagent-goal-worktrees/{}", run_id)));
        assert!(
            worktree.join("README.md").exists(),
            "worktree must be checked out at the baseline commit"
        );
    }

    // ── #504 worktree reaper ────────────────────────────────────────────────

    fn git(dir: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
    }

    /// Init a repo at `<tmp>/proj` with an `origin` bare remote and one baseline
    /// commit pushed to `origin/main`, so a worktree checked out at the baseline
    /// counts as "pushed". Returns the baseline short-resolvable SHA.
    fn init_repo_with_remote(tmp: &Path) -> (PathBuf, String) {
        let origin = tmp.join("origin.git");
        std::process::Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&origin)
            .output()
            .unwrap();
        let repo = tmp.join("proj");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t.t"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["branch", "-M", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        git(&repo, &["fetch", "-q", "origin"]);
        let head = String::from_utf8(git(&repo, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        (repo, head)
    }

    /// Add an unpushed commit inside a worktree (it shares the repo's user
    /// config, so no per-worktree git config is needed).
    fn commit_in_worktree(wt: &Path, file: &str) {
        std::fs::write(wt.join(file), "work").unwrap();
        git(wt, &["add", "."]);
        git(wt, &["commit", "-q", "-m", "work"]);
    }

    /// Tracked-remove: a Complete goal whose worktree is fully on origin is
    /// reaped via `git worktree remove`.
    #[tokio::test]
    async fn reap_removes_pushed_tracked_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-pushed")
            .await
            .unwrap();
        assert!(wt.is_dir());

        let outcome = reap_goal_worktree(&repo, "cli-pushed", false).await;
        assert_eq!(outcome, ReapOutcome::RemovedTracked);
        assert!(!wt.exists(), "tracked pushed worktree must be removed");
    }

    /// Safety guard: a Complete goal with UNPUSHED commits is NOT reaped — the
    /// unreviewed work survives.
    #[tokio::test]
    async fn reap_keeps_unpushed_complete_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-unpushed")
            .await
            .unwrap();
        commit_in_worktree(&wt, "new.txt");

        let outcome = reap_goal_worktree(&repo, "cli-unpushed", false).await;
        assert_eq!(outcome, ReapOutcome::SkippedUnpushed);
        assert!(
            wt.exists(),
            "a Complete worktree with unpushed commits must be protected"
        );
    }

    /// Cancelled bypasses the push guard: the same unpushed worktree IS reaped
    /// (the user explicitly discarded the work).
    #[tokio::test]
    async fn reap_cancelled_removes_unpushed_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-cancel")
            .await
            .unwrap();
        commit_in_worktree(&wt, "new.txt");

        let outcome = reap_goal_worktree(&repo, "cli-cancel", true).await;
        assert_eq!(outcome, ReapOutcome::RemovedTracked);
        assert!(!wt.exists(), "cancelled goal worktree must be reaped");
    }

    /// Orphaned-dir fallback: a `cli-*` dir git no longer tracks (no worktree
    /// ref) is reclaimed by the sweep via `rm -rf` after `git worktree remove`
    /// fails — the second #504 leak form.
    #[tokio::test]
    async fn sweep_reclaims_orphaned_dir_via_rm_rf() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, _baseline) = init_repo_with_remote(tmp.path());
        let orphan = tmp.path().join(GOAL_WORKTREES_DIR).join("cli-orphan");
        std::fs::create_dir_all(orphan.join("target")).unwrap();
        std::fs::write(orphan.join("target").join("artifact.o"), "junk").unwrap();

        let reclaimed = sweep_orphaned_worktrees(&repo, &[]).await;
        assert_eq!(reclaimed, 1);
        assert!(!orphan.exists(), "orphaned dir must be rm -rf'd");
    }

    /// Sweep safety: skips worktrees of active goals AND keeps unpushed ones,
    /// while still reaping a pushed leak from a finished goal.
    #[tokio::test]
    async fn sweep_skips_active_keeps_unpushed_reaps_pushed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt_active = create_goal_worktree(&repo, &baseline, "cli-active")
            .await
            .unwrap();
        let wt_unpushed = create_goal_worktree(&repo, &baseline, "cli-unpushed")
            .await
            .unwrap();
        commit_in_worktree(&wt_unpushed, "new.txt");
        let wt_done = create_goal_worktree(&repo, &baseline, "cli-done")
            .await
            .unwrap();

        let active = vec!["cli-active".to_string()];
        let reclaimed = sweep_orphaned_worktrees(&repo, &active).await;

        assert_eq!(reclaimed, 1, "only the finished pushed worktree is reaped");
        assert!(wt_active.exists(), "active goal worktree must be skipped");
        assert!(wt_unpushed.exists(), "unpushed worktree must be protected");
        assert!(!wt_done.exists(), "finished pushed worktree must be reaped");
    }
}
