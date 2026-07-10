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

/// #522 — the worker cannot push. An ephemeral `remote.origin.pushurl`
/// override points its git at this sentinel (an unsupported protocol), so any
/// `git push` from the worker fails deterministically with this string in the
/// error. Permagent performs the real push itself AFTER the credential scan
/// (`scan_committed_changes`) passes — the leak-before-scan window of the
/// post-exit-only guard (#508) is closed: a secret can never reach origin.
pub const PUSH_BLOCK_SENTINEL: &str = "permagent-credential-guard://push-disabled";

/// Standing constraint appended to every external-CLI worker prompt (#522).
/// The pushurl override above is the enforcement; this line keeps the worker
/// from burning its run retrying a push that can never succeed.
const COMMIT_ONLY_BRIEF: &str =
    "\n\nIMPORTANT: Commit your work in this worktree, but do NOT push. \
Pushing from this worktree is disabled — Permagent scans your commits for \
credential-shaped content and performs the push itself after the scan passes.";

/// Compose the ephemeral `GIT_CONFIG_*` env pairs injected into the worker's
/// process (#523 hooks pattern): the push block is unconditional; the
/// work-base hooks path is added when available. Inherited only by this
/// worker's git subprocesses — the user's repo config is never touched.
fn worker_git_env(hooks_dir: Option<&PathBuf>) -> Vec<(String, String)> {
    let mut pairs = vec![(
        "remote.origin.pushurl".to_string(),
        PUSH_BLOCK_SENTINEL.to_string(),
    )];
    if let Some(dir) = hooks_dir {
        pairs.push((
            "core.hooksPath".to_string(),
            dir.to_string_lossy().into_owned(),
        ));
    }
    pairs
}

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
    /// The worker's committed changes contain credential-shaped content (#508).
    /// A terminal, non-retriable block: parks the goal for human attention with
    /// the offending file + pattern, so a re-dispatch can't just re-leak. Carries
    /// the human-readable block reason.
    Blocked { reason: String },
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
    /// Fix (#531): the worker's TRUE head — the worktree HEAD at its LAST commit,
    /// recorded emit-side by the in-worktree hook (last-wins, re-captured on
    /// rebase), falling back to a completion-time `git rev-parse HEAD` only when
    /// the hook did not fire. The hook value is robust to the worker integrating a
    /// concurrent push, which can otherwise leave the detached worktree HEAD ref
    /// reading as an ANCESTOR of the true tip — diffing `work_base..stale-head`
    /// then inverts the range and false-fails correct multi-commit work. Paired
    /// with `work_base_commit`, `work_base_commit..head_commit` is exactly the
    /// worker's own commits (1 or N).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    /// Fix (#523): the worker's TRUE base — the parent of its FIRST commit,
    /// captured by an in-worktree git hook (re-captured on rebase). This is the
    /// commit the worker actually forked onto AFTER any pull/fast-forward, so
    /// `work_base_commit..head_commit` is exactly the worker's own work — correct
    /// for N commits and robust to fast-forward/rebase after dispatch. The
    /// dispatch `baseline_commit` is a stale project-root snapshot and must NOT
    /// be used to anchor verification. `None` when the hook did not fire (the
    /// verifier then records Uncertain rather than diffing a possibly-stale base).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_base_commit: Option<String>,
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
    /// Fix A (#505): `true` when the diff/diffstat git command itself FAILED
    /// (non-zero exit / could not run) while collecting evidence — e.g. the
    /// baseline was unreachable. Distinguishes a git ERROR from a genuine empty
    /// diff so `files_changed == 0` is never silently trusted as "no work".
    #[serde(default)]
    pub diff_errored: bool,
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
        let base = match &self.persona_override {
            Some((block, _)) if !block.is_empty() => format!("{}\n\n{}", block, instructions),
            _ => instructions.to_string(),
        };
        format!("{}{}", base, COMMIT_ONLY_BRIEF)
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

        // Fix (#523): install the work-base capture hooks BEFORE the worker runs,
        // so the parent of its first commit (its true fork point) is recorded the
        // moment it commits — the only point at which the base is knowable (after
        // the worker pushes, `origin/main == HEAD`, so it cannot be recovered at
        // completion). Best-effort: a failure here leaves `work_base` unrecorded,
        // and the verifier records Uncertain rather than guessing a stale base.
        let work_base_hooks = install_work_base_hooks(&worktree).await;

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
        // Ephemeral git config for the worker (#523 hooks + #522 push block) —
        // inherited only by this worker's git subprocesses, so the user's repo
        // config is never touched.
        let env_pairs = worker_git_env(work_base_hooks.as_ref());
        cmd.env("GIT_CONFIG_COUNT", env_pairs.len().to_string());
        for (i, (key, value)) in env_pairs.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), key)
                .env(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
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

/// Fix (#523): install per-worktree git hooks that record the worker's TRUE base
/// commit — the parent of its FIRST commit, re-captured if the worker rebases
/// (e.g. to integrate a concurrent push before re-pushing). This is the only
/// anchor that is correct for N commits AND robust to a fast-forward/rebase after
/// dispatch; it cannot be computed at completion, because the worker's
/// `git push origin HEAD:main` makes `origin/main == HEAD` before we look.
///
/// The hooks live inside the worktree's private git dir, so `git worktree remove`
/// (#511) reaps them automatically, and are reached via an ephemeral
/// `core.hooksPath` env on the worker process only — the user's repo config is
/// never modified. `post-commit` runs even under `--no-verify`. Returns
/// `(hooks_dir, work_base_file)`, or `None` if the hooks could not be installed
/// (the verifier then records Uncertain rather than diffing a possibly-stale
/// base — we never silently fall back to a weaker anchor).
async fn install_work_base_hooks(worktree: &Path) -> Option<PathBuf> {
    let git_dir = git_text(worktree, &["rev-parse", "--absolute-git-dir"]).await;
    if git_dir.is_empty() {
        tracing::error!(
            target: "permagentd::brain",
            worktree = %worktree.display(),
            "could not resolve git dir to install work-base hooks (#523) — \
             verification will be Uncertain for this goal"
        );
        return None;
    }
    let hooks_dir = work_base_hooks_dir(&git_dir);
    let work_base_file = hooks_dir.join("work_base");
    if let Err(e) = tokio::fs::create_dir_all(&hooks_dir).await {
        tracing::error!(
            target: "permagentd::brain",
            dir = %hooks_dir.display(),
            "failed to create work-base hooks dir (#523): {} — verification will be Uncertain",
            e
        );
        return None;
    }

    // Path is fully controlled by us; single-quote it (escaping any literal quote)
    // so a worktree path with spaces is safe inside the POSIX hook scripts.
    let wbf = work_base_file.to_string_lossy().replace('\'', "'\\''");
    let work_head_file = hooks_dir.join("work_head");
    let whf = work_head_file.to_string_lossy().replace('\'', "'\\''");
    // At commit time, record BOTH the worker's true base (parent of its FIRST
    // commit, write-once — #523) AND its true head (current HEAD, last-wins —
    // #531). The head is captured here, not at completion, because the worker's
    // concurrent-push integration can leave the detached worktree HEAD reading as
    // an ANCESTOR of the true tip — a completion-time `git rev-parse HEAD` then
    // inverts the verifier's diff range and false-fails correct work.
    let post_commit = format!(
        "#!/bin/sh\n\
         # Permagent (#523/#531): record the worker's true base (parent of FIRST commit,\n\
         # write-once) and true head (current HEAD, last-wins) at commit time.\n\
         h=\"$(git rev-parse HEAD 2>/dev/null)\" && printf '%s\\n' \"$h\" > '{whf}'\n\
         f='{wbf}'\n\
         [ -f \"$f\" ] && exit 0\n\
         p=\"$(git rev-parse HEAD^ 2>/dev/null)\" && printf '%s\\n' \"$p\" > \"$f\"\n\
         exit 0\n"
    );
    // On rebase (worker integrated a concurrent push), the commits are reparented;
    // re-capture the parent of the earliest rewritten commit = the NEW base, and
    // the rebased tip = the NEW head.
    let post_rewrite = format!(
        "#!/bin/sh\n\
         # Permagent (#523/#531): after a rebase (concurrent-push integration) re-capture\n\
         # BOTH the base (parent of the earliest rewritten commit) and the head (the\n\
         # rebased tip = newest rewritten commit) from the rewrite MAP on stdin. The map\n\
         # ('old new' per commit, oldest first) is authoritative; reading `git rev-parse\n\
         # HEAD` here races the rebase's ref update and intermittently reads empty (#531).\n\
         [ \"$1\" = rebase ] || exit 0\n\
         map=\"$(cat)\"\n\
         [ -n \"$map\" ] || exit 0\n\
         first=\"$(printf '%s\\n' \"$map\" | head -n1 | awk '{{print $2}}')\"\n\
         last=\"$(printf '%s\\n' \"$map\" | tail -n1 | awk '{{print $2}}')\"\n\
         [ -n \"$first\" ] && p=\"$(git rev-parse \"${{first}}^\" 2>/dev/null)\" && printf '%s\\n' \"$p\" > '{wbf}'\n\
         [ -n \"$last\" ] && printf '%s\\n' \"$last\" > '{whf}'\n\
         exit 0\n"
    );

    for (name, body) in [("post-commit", post_commit), ("post-rewrite", post_rewrite)] {
        let path = hooks_dir.join(name);
        if let Err(e) = tokio::fs::write(&path, body).await {
            tracing::error!(
                target: "permagentd::brain",
                hook = %path.display(),
                "failed to write work-base hook (#523): {} — verification will be Uncertain",
                e
            );
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
    Some(hooks_dir)
}

/// The hooks directory the work-base capture lives in, inside a worktree's
/// private git dir (#523). Single source of truth for install and read-back.
fn work_base_hooks_dir(git_dir: &str) -> PathBuf {
    PathBuf::from(git_dir).join("permagent-base-hooks")
}

/// Read the worker's true base SHA recorded by the work-base hooks (#523), if
/// present. Resolves the worktree's private git dir, so it works from the live
/// dispatch path and the restart-recovery path alike. `None` when the hook did
/// not fire (the verifier then records Uncertain rather than guessing a base).
async fn read_work_base(worktree: &Path) -> Option<String> {
    let git_dir = git_text(worktree, &["rev-parse", "--absolute-git-dir"]).await;
    if git_dir.is_empty() {
        return None;
    }
    let f = work_base_hooks_dir(&git_dir).join("work_base");
    match tokio::fs::read_to_string(&f).await {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Read the worker's true head SHA recorded by the work-base hooks (#531) — the
/// worktree HEAD at the worker's LAST commit (last-wins, re-captured on rebase).
/// Mirrors [`read_work_base`]. `None` when the hook did not fire. This is the
/// durable head the verifier anchors `work_base..head` to; the completion-time
/// `git rev-parse HEAD` is unreliable once the worker integrates a concurrent
/// push (the detached worktree HEAD can read as an ANCESTOR of the true tip).
async fn read_work_head(worktree: &Path) -> Option<String> {
    let git_dir = git_text(worktree, &["rev-parse", "--absolute-git-dir"]).await;
    if git_dir.is_empty() {
        return None;
    }
    let f = work_base_hooks_dir(&git_dir).join("work_head");
    match tokio::fs::read_to_string(&f).await {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
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
                // #508: deterministic credential guard. Scan the worker's
                // committed changes BEFORE declaring success; a credential-shaped
                // file blocks the goal (terminal, non-retriable) instead of
                // advancing it to Review. Fail-closed: an unverifiable file blocks.
                if let Some(reason) = scan_committed_changes(&working_dir, &baseline).await {
                    return GoalOutcome::Blocked { reason };
                }
                // #522: the scan passed and the worker itself cannot push (its
                // pushurl is the block sentinel) — Permagent owns the push.
                // Best-effort: a push failure (e.g. non-fast-forward from a
                // concurrent push) leaves the work reviewable-but-unpushed in
                // the worktree rather than failing the goal.
                push_clean_work(&working_dir, &baseline).await;
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

/// #522 — the Permagent-owned push: after `scan_committed_changes` passes,
/// push the worker's commits to `origin` `HEAD:main` (the target workers used
/// when they owned the push). Skipped when the worker made no commits. A
/// failure — unreachable remote, non-fast-forward from a concurrent push — is
/// logged loudly and left for review-in-worktree; it never fails the goal and
/// never bypasses the scan.
async fn push_clean_work(worktree: &Path, baseline: &str) {
    let range = format!("{}..HEAD", baseline);
    let committed = git_text(worktree, &["rev-list", "--count", &range])
        .await
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    if committed == 0 {
        return; // nothing to publish (analysis/docs goal with no commits)
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree)
        .args(["push", "origin", "HEAD:main"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            tracing::info!(
                worktree = %worktree.display(),
                commits = committed,
                "credential scan clean — pushed worker commits to origin/main (#522)"
            );
        }
        Ok(out) => {
            tracing::warn!(
                worktree = %worktree.display(),
                commits = committed,
                error = %tail(&redact_secrets(&String::from_utf8_lossy(&out.stderr)), 500),
                "post-scan push failed — work stays reviewable in the worktree, unpushed (#522)"
            );
        }
        Err(e) => {
            tracing::warn!(
                worktree = %worktree.display(),
                error = %e,
                "post-scan push could not run — work stays reviewable in the worktree, unpushed (#522)"
            );
        }
    }
}

/// Per-file content read cap for the credential scan. Secrets live near the top
/// of a file; reading the whole of a large generated artifact is wasteful.
const SECRET_SCAN_READ_CAP: usize = 256 * 1024;

/// #508 — deterministic credential guard over the worker's *committed* changes
/// (`baseline..HEAD`), run after a clean exit and before the goal is allowed to
/// advance. Returns a human-readable block reason on the first credential-shaped
/// file/content, or `None` when the changeset is clean.
///
/// **Fail-closed:** a changed (added/modified/renamed) file that exists but
/// cannot be read is treated as a match — the guard must never *allow* on
/// uncertainty. Deletions are excluded (`--diff-filter=ACMR`) so a legitimately
/// removed file can't trip the fail-closed path. Binary / oversized files are
/// still caught by the filename rule; their content scan is skipped.
pub(crate) async fn scan_committed_changes(worktree: &Path, baseline: &str) -> Option<String> {
    let range = format!("{}..HEAD", baseline);
    let names = git_text(
        worktree,
        &["diff", "--name-only", "--diff-filter=ACMR", &range],
    )
    .await;

    for path in names.lines().map(str::trim).filter(|l| !l.is_empty()) {
        // Filename rule first — catches even binary / unreadable files.
        if let Some(finding) = crate::steward::secret_scan::scan_path(path) {
            return Some(block_message(path, &finding));
        }
        // Content rule. The worktree is checked out at HEAD, so the committed
        // file is on disk.
        let full = worktree.join(path);
        match read_capped_text(&full) {
            Ok(Some(content)) => {
                if let Some(finding) = crate::steward::secret_scan::scan_content(&content) {
                    return Some(block_message(path, &finding));
                }
            }
            // Binary / oversized — filename rule already ran; nothing to add.
            Ok(None) => {}
            // Fail-closed: a tracked, non-deleted file we cannot verify blocks.
            Err(e) => {
                return Some(format!(
                    "Commit blocked by the credential guard: could not read `{path}` to verify it \
                     contains no secrets ({e}). The guard fails closed — fix the file or remove it \
                     from the commit."
                ));
            }
        }
    }
    None
}

/// Render the human-readable block message for a credential finding.
fn block_message(path: &str, finding: &crate::steward::secret_scan::SecretFinding) -> String {
    format!(
        "Commit blocked by the credential guard ({}): `{}` {}. Credentials must never be \
         committed — move the secret to a secrets manager / environment variable and add the file \
         to `.gitignore`, then re-run the goal.",
        finding.rule, path, finding.detail
    )
}

/// Read a file as UTF-8 text, capped at [`SECRET_SCAN_READ_CAP`]. Returns
/// `Ok(None)` for binary content (a NUL byte in the read window) or a file that
/// does not exist as a regular readable file; `Err` only on a real I/O error of
/// an existing path (the fail-closed signal).
fn read_capped_text(path: &Path) -> std::io::Result<Option<String>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; SECRET_SCAN_READ_CAP];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    if buf.contains(&0) {
        return Ok(None); // binary
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
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

    // Fix (#531): prefer the DURABLE head recorded by the in-worktree hook at the
    // worker's last commit (last-wins, re-captured on rebase) over a
    // completion-time `git rev-parse HEAD`. After the worker integrates a
    // concurrent push, the detached worktree HEAD can read as an ANCESTOR of the
    // true tip — a multi-commit goal then false-fails on the inverted range
    // work_base..stale-head. The hook captures the tip in the worktree's own
    // commit context, the same way work_base is captured, so work_base..head is
    // exactly the worker's commits for N commits. Fall back to `rev-parse HEAD`
    // only when the hook did not fire (work_base is then also absent and the
    // verifier records Uncertain regardless).
    let head_commit = match read_work_head(worktree).await {
        Some(h) => Some(h),
        None => git_line(worktree, &["rev-parse", "--short", "HEAD"]).await,
    };
    // Fix (#523): the worker's true base, recorded by the in-worktree git hook at
    // its first commit (re-captured on rebase). It lives in the worktree's private
    // git dir, so we read it while the worktree still exists (before the #511
    // reaper) and persist the durable SHA on the card. Self-resolved from the
    // git dir so every caller (dispatch + restart recovery) gets it for free.
    let work_base_commit = read_work_base(worktree).await;
    let commits = git_text(worktree, &["log", "--format=%h %s", &range])
        .await
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>();
    // Fix A (#505): the diff/diffstat are the LOAD-BEARING evidence fields. Use
    // the checked `git_try` so a git FAILURE (e.g. unreachable baseline) is loud
    // and recorded as `diff_errored`, never silently degraded to an empty diff
    // that reads as "0 files changed → no work".
    let diffstat_raw = git_try(worktree, &["diff", "--stat", &range]).await;
    let shortstat_raw = git_try(worktree, &["diff", "--shortstat", &range]).await;
    let diff_errored = diffstat_raw.is_none() || shortstat_raw.is_none();
    let diffstat = tail(&diffstat_raw.unwrap_or_default(), 4000);
    let (files_changed, insertions, deletions) =
        parse_shortstat(&shortstat_raw.unwrap_or_default());
    let push_target = git_text(worktree, &["branch", "-r", "--contains", "HEAD"])
        .await
        .lines()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty());

    GoalEvidence {
        worktree_path: worktree.to_string_lossy().to_string(),
        baseline_commit: baseline.to_string(),
        head_commit,
        work_base_commit,
        commits,
        diffstat,
        files_changed,
        insertions,
        deletions,
        push_target,
        worker_summary,
        diff_errored,
    }
}

/// Run `git <args>` in `dir`. `Some(trimmed stdout)` on success; `None` on ANY
/// failure (could-not-run or non-zero exit), logged LOUDLY (Fix A, #505) so a
/// silent baseline-unreachable failure cannot masquerade as "no work".
async fn git_try(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => {
            tracing::warn!(
                target: "permagentd::brain",
                dir = %dir.display(),
                args = ?args,
                "evidence git command exited {:?}: {}",
                o.status.code(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::brain",
                dir = %dir.display(),
                args = ?args,
                "evidence git command could not run: {}",
                e
            );
            None
        }
    }
}

/// Run `git <args>` in `dir`, returning trimmed stdout (empty on any failure).
/// Non-load-bearing convenience over [`git_try`] for fields where an empty
/// value is an acceptable degradation (head sha, commit list, push target).
async fn git_text(dir: &Path, args: &[&str]) -> String {
    git_try(dir, args).await.unwrap_or_default()
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
        // #522: every worker prompt carries the commit-only brief.
        assert_eq!(resolved[0], "-p");
        assert_eq!(
            resolved[1],
            format!("Implement the thing{}", COMMIT_ONLY_BRIEF)
        );
        assert_eq!(resolved[2], "--dangerously-skip-permissions");
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
        // #522: the commit-only brief is appended after the persona + goal.
        assert_eq!(
            prompt,
            format!("You are Claude Code.\n\nDo the work{}", COMMIT_ONLY_BRIEF)
        );
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
        assert!(!ev.diff_errored, "a successful diff is not errored");
    }

    /// Fix A (#505): when the diff git command itself FAILS (here: the dir is not
    /// a git repo, so `git diff baseline..HEAD` errors), the evidence is marked
    /// `diff_errored` — distinguishing a git ERROR from a genuine empty diff, so
    /// `files_changed == 0` is never silently trusted as "no work".
    #[tokio::test]
    async fn collect_evidence_flags_git_failure_as_errored() {
        let tmp = tempfile::TempDir::new().unwrap();
        // A plain directory — not a git repo. Every `git -C` call here fails.
        let ev = collect_evidence(tmp.path(), "deadbeef", "summary".to_string()).await;
        assert!(
            ev.diff_errored,
            "a failed diff git command must surface as ERRORED, not a clean zero-diff"
        );
        assert_eq!(
            ev.files_changed, 0,
            "no count is recoverable from a failure"
        );
    }

    /// Fix (#523), the mechanism test: the in-worktree hooks capture the worker's
    /// TRUE base — the parent of its FIRST commit — and re-capture it when the
    /// worker rebases (the concurrent-push case). Proves correctness across
    /// fast-forward-before-commit AND rebase-after-commit, the exact cases a
    /// dispatch-time or `head^` anchor gets wrong.
    #[tokio::test]
    async fn work_base_hook_captures_true_base_across_ff_and_rebase() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("proj");
        std::fs::create_dir(&repo).unwrap();

        // Plain git (no hook): the foreign commits exist BEFORE the worktree's
        // hooks are installed — in reality they arrive via fetch/fast-forward, not
        // a local `git commit`, so `post-commit` never fires for them.
        let plain = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        let rev = |r: &str| {
            String::from_utf8(plain(&["rev-parse", r]).stdout)
                .unwrap()
                .trim()
                .to_string()
        };

        plain(&["init", "-q", "-b", "work"]);
        plain(&["config", "user.email", "t@t.t"]);
        plain(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "base\n").unwrap();
        plain(&["add", "-A"]);
        plain(&["commit", "-q", "-m", "B0"]);
        // A foreign commit the worker fast-forwards onto before doing any work.
        std::fs::write(repo.join("other.txt"), "other goal\n").unwrap();
        plain(&["add", "-A"]);
        plain(&["commit", "-q", "-m", "O1-foreign"]);
        let o1 = rev("HEAD");

        // Now the worktree exists and the hooks are installed; the worker's git is
        // routed at them exactly as the dispatch env does.
        let hooks_dir = install_work_base_hooks(&repo).await.expect("hooks install");
        let work_base_file = hooks_dir.join("work_base");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "core.hooksPath")
                .env("GIT_CONFIG_VALUE_0", &hooks_dir)
                .args(args)
                .output()
                .unwrap()
        };

        // The worker's FIRST commit — post-commit records its parent (= O1).
        std::fs::write(repo.join("work.txt"), "the work\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "W1"]);
        let captured = std::fs::read_to_string(&work_base_file).unwrap();
        assert_eq!(
            captured.trim(),
            o1,
            "base must be the parent of the worker's first commit (post-FF), not the dispatch baseline"
        );

        // A concurrent push lands as a foreign O2 on top of O1 (again via plain
        // git — it is not the worker's commit), and the worker rebases its work
        // onto it before re-pushing. post-rewrite must re-capture O2.
        plain(&["branch", "side", &o1]);
        plain(&["checkout", "-q", "side"]);
        std::fs::write(repo.join("other2.txt"), "second other goal\n").unwrap();
        plain(&["add", "-A"]);
        plain(&["commit", "-q", "-m", "O2-foreign"]);
        let o2 = rev("HEAD");
        plain(&["checkout", "-q", "work"]);
        git(&["rebase", "-q", "side"]);
        let captured2 = std::fs::read_to_string(&work_base_file).unwrap();
        assert_eq!(
            captured2.trim(),
            o2,
            "after a rebase (concurrent-push integration) the base must re-capture to the new parent O2"
        );
    }

    /// Fix (#531), the head-side mechanism test: the in-worktree hook records the
    /// worker's TRUE head — the tip at its LAST commit (last-wins), re-captured on
    /// rebase. This is the head analogue of the #523 base fix. The bug it guards:
    /// a completion-time `git rev-parse HEAD` in the detached worktree can read as
    /// an ANCESTOR of the true tip after a concurrent-push integration, inverting
    /// the verifier's `work_base..head` range and false-failing N-commit work.
    #[tokio::test]
    async fn work_head_hook_captures_tip_across_multi_commit_and_rebase() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("proj");
        std::fs::create_dir(&repo).unwrap();

        let plain = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        let rev = |r: &str| {
            String::from_utf8(plain(&["rev-parse", r]).stdout)
                .unwrap()
                .trim()
                .to_string()
        };

        plain(&["init", "-q", "-b", "work"]);
        plain(&["config", "user.email", "t@t.t"]);
        plain(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "base\n").unwrap();
        plain(&["add", "-A"]);
        plain(&["commit", "-q", "-m", "B0"]);
        let b0 = rev("HEAD");

        let hooks_dir = install_work_base_hooks(&repo).await.expect("hooks install");
        let work_head_file = hooks_dir.join("work_head");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "core.hooksPath")
                .env("GIT_CONFIG_VALUE_0", &hooks_dir)
                .args(args)
                .output()
                .unwrap()
        };

        // First worker commit W1 — head records W1.
        std::fs::write(repo.join("w1.txt"), "one\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "W1"]);
        let w1 = rev("HEAD");
        assert_eq!(
            std::fs::read_to_string(&work_head_file).unwrap().trim(),
            w1,
            "head must record the first commit"
        );

        // Second worker commit W2 — head LAST-WINS to the tip (the exact #531 bug:
        // a stale read would leave the head at the ancestor W1, not the tip W2).
        std::fs::write(repo.join("w2.txt"), "two\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "W2"]);
        let w2 = rev("HEAD");
        assert_ne!(w1, w2, "W2 is a distinct, newer commit than W1");
        assert_eq!(
            std::fs::read_to_string(&work_head_file).unwrap().trim(),
            w2,
            "head must re-capture (last-wins) to the worker's TIP, not stay at the ancestor W1"
        );

        // A concurrent push lands a foreign O on the side; the worker rebases its
        // two commits onto it. post-rewrite must re-capture head to the rebased tip.
        plain(&["branch", "side", &b0]);
        plain(&["checkout", "-q", "side"]);
        std::fs::write(repo.join("other.txt"), "concurrent\n").unwrap();
        plain(&["add", "-A"]);
        plain(&["commit", "-q", "-m", "O-foreign"]);
        plain(&["checkout", "-q", "work"]);
        git(&["rebase", "-q", "side"]);
        let rebased_tip = rev("HEAD");
        assert_eq!(
            std::fs::read_to_string(&work_head_file).unwrap().trim(),
            rebased_tip,
            "after a rebase the head must re-capture to the new rebased tip"
        );
    }

    /// #508: the credential guard scans the worker's committed changes. A clean
    /// changeset passes; a committed `.env` (or secret content) is blocked with a
    /// reason naming the file. A removed file must not trip the fail-closed path.
    #[tokio::test]
    async fn scan_committed_changes_blocks_secrets_and_passes_clean() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("proj");
        std::fs::create_dir(&repo).unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t.t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);
        let baseline = String::from_utf8(g(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();

        // Clean change → no block.
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "clean work"]);
        assert!(scan_committed_changes(&repo, &baseline).await.is_none());

        // Commit a dotenv file → blocked, message names the file.
        std::fs::write(repo.join(".env"), "API_KEY=Sup3rS3cretValue123\n").unwrap();
        g(&["add", "-f", ".env"]); // -f: repos often gitignore .env
        g(&["commit", "-q", "-m", "oops secret"]);
        let reason = scan_committed_changes(&repo, &baseline)
            .await
            .expect("committed .env must be blocked");
        assert!(reason.contains(".env"), "reason names the file: {reason}");
        assert!(reason.contains("credential guard"), "reason: {reason}");

        // Removing a tracked file must NOT trip the fail-closed read path.
        g(&["rm", "-q", ".env"]);
        g(&["commit", "-q", "-m", "remove secret"]);
        // The deletion commit alone (new baseline) is clean.
        let after_del = String::from_utf8(g(&["rev-parse", "HEAD~1"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        assert!(scan_committed_changes(&repo, &after_del).await.is_none());
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

    // ── #522 push-ownership inversion ───────────────────────────────────────

    /// The env composer always blocks the worker's push and adds hooks when
    /// available.
    #[test]
    fn worker_git_env_always_blocks_push() {
        let no_hooks = worker_git_env(None);
        assert_eq!(
            no_hooks,
            vec![(
                "remote.origin.pushurl".to_string(),
                PUSH_BLOCK_SENTINEL.to_string()
            )]
        );

        let dir = PathBuf::from("/tmp/hooks");
        let with_hooks = worker_git_env(Some(&dir));
        assert_eq!(with_hooks.len(), 2);
        assert_eq!(with_hooks[0].1, PUSH_BLOCK_SENTINEL);
        assert_eq!(
            with_hooks[1],
            ("core.hooksPath".to_string(), "/tmp/hooks".to_string())
        );
    }

    /// A worker-side `git push` under the injected env fails deterministically
    /// — the sentinel is not a real protocol, so the push can never reach the
    /// remote regardless of what the worker's model decides to do.
    #[tokio::test]
    async fn sentinel_env_blocks_worker_push() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-pushblock")
            .await
            .unwrap();
        commit_in_worktree(&wt, "work.txt");

        let pairs = worker_git_env(None);
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(&wt)
            .args(["push", "origin", "HEAD:main"])
            .env("GIT_CONFIG_COUNT", pairs.len().to_string());
        for (i, (k, v)) in pairs.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), k)
                .env(format!("GIT_CONFIG_VALUE_{i}"), v);
        }
        let out = cmd.output().unwrap();
        assert!(!out.status.success(), "worker push must be refused");

        // And nothing reached origin.
        let remote_tip =
            String::from_utf8(git(&repo, &["ls-remote", "origin", "refs/heads/main"]).stdout)
                .unwrap();
        assert!(
            remote_tip.starts_with(&baseline),
            "origin/main must still be at the baseline"
        );
    }

    /// After a clean scan, Permagent's own push (no blocking env) publishes
    /// the worker's commits; with no commits it is a no-op.
    #[tokio::test]
    async fn push_clean_work_publishes_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-permagent-push")
            .await
            .unwrap();

        // No commits: no-op, origin untouched.
        push_clean_work(&wt, &baseline).await;
        let tip = String::from_utf8(git(&repo, &["ls-remote", "origin", "refs/heads/main"]).stdout)
            .unwrap();
        assert!(tip.starts_with(&baseline));

        // With a commit: origin/main advances to the worktree HEAD.
        commit_in_worktree(&wt, "work.txt");
        let head = String::from_utf8(git(&wt, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        push_clean_work(&wt, &baseline).await;
        let tip = String::from_utf8(git(&repo, &["ls-remote", "origin", "refs/heads/main"]).stdout)
            .unwrap();
        assert!(
            tip.starts_with(&head),
            "origin/main must be at the worker's HEAD after the Permagent-owned push"
        );
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
