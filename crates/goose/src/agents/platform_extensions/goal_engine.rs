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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
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
///
/// 2 h (#467): the old 30-min cap killed legitimately long goals (bug audits,
/// large refactors) mid-work. Per-worker override: `timeout_secs` on the
/// worker's agent.yaml entry. Per-goal bounds are a future enhancement,
/// deliberately not this knob.
pub const DEFAULT_EXTERNAL_CLI_TIMEOUT_SECS: u64 = 2 * 60 * 60;

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
pub(crate) const COMMIT_ONLY_BRIEF: &str =
    "\n\nIMPORTANT: Commit your work in this worktree, but do NOT push. \
Pushing from this worktree is disabled — Permagent scans your commits for \
credential-shaped content and performs the push itself after the scan passes.";

/// Compose the ephemeral `GIT_CONFIG_*` env pairs injected into the worker's
/// process (#523 hooks pattern): the push block is unconditional; the
/// work-base hooks path is added when available. Inherited only by this
/// worker's git subprocesses — the user's repo config is never touched.
pub(crate) fn worker_git_env(hooks_dir: Option<&PathBuf>) -> Vec<(String, String)> {
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
    /// Best-effort progress events consumed by the orchestrator's receipt
    /// tracker. External workers emit one event for every stdout read; engines
    /// without a byte stream leave this unused.
    pub output_tx: Option<mpsc::UnboundedSender<WorkerOutputEvent>>,
}

/// Timestamped evidence that an external worker produced stdout. The timestamp
/// is captured at the read boundary so a queued event still records when the
/// worker first became observable, rather than when the database write ran.
#[derive(Debug, Clone)]
pub struct WorkerOutputEvent {
    pub observed_at: String,
}

/// What an engine returns once the goal is spawned: a stable run identifier
/// (recorded in card metadata as the worker session id), the join handle the
/// tracker awaits, a [`GoalKill`] handle the cancel path uses to stop the
/// worker on demand (#490), and — for steerable workers — a [`SteerHandle`]
/// the orchestrator can use to inject a mid-run correction.
pub struct DispatchedWork {
    pub run_id: String,
    pub join: JoinHandle<GoalOutcome>,
    pub kill: GoalKill,
    /// `Some` only for claude external-CLI workers (the one engine with a
    /// bidirectional stdin protocol today). `None` = not steerable; the steer
    /// tool reports that honestly instead of pretending.
    pub steer: Option<std::sync::Arc<SteerHandle>>,
}

/// Mid-run steering for a claude external-CLI worker (hardening pass,
/// 2026-08-10). The CLI's `--input-format stream-json` mode is bidirectional:
/// user messages written to stdin become new turns. The lifecycle rule that
/// makes this SAFE for completion detection: the CLI exits only when stdin
/// closes, so the reader loop closes stdin after each `result` event UNLESS a
/// steer arrived during the turn — one pending steer buys exactly one more
/// turn, and an unsteered worker behaves byte-identically to the old one-shot
/// dispatch (first result → close → exit).
///
/// Proven against a live CLI before this was written: with
/// `--input-format stream-json` the `-p` argument is NOT auto-run — the first
/// stdin message is the first turn — and the process exits 0 on stdin close.
pub struct SteerHandle {
    writer: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    steer_pending: std::sync::atomic::AtomicBool,
}

impl SteerHandle {
    fn new(stdin: tokio::process::ChildStdin) -> Self {
        Self {
            writer: tokio::sync::Mutex::new(Some(stdin)),
            steer_pending: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Encode one user message as the CLI's NDJSON wire line.
    fn user_message_line(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] }
        })
        .to_string()
            + "\n"
    }

    /// Inject a correction into the running worker. Sets the pending flag
    /// BEFORE writing so a result event racing this call cannot close stdin
    /// between the write and the flag.
    pub async fn steer(&self, text: &str) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        self.steer_pending
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut guard = self.writer.lock().await;
        let Some(writer) = guard.as_mut() else {
            self.steer_pending
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Err(
                "the worker has already finished its final turn — steering arrived too late"
                    .to_string(),
            );
        };
        writer
            .write_all(Self::user_message_line(text).as_bytes())
            .await
            .map_err(|e| format!("could not write to the worker's stdin: {e}"))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("could not flush the worker's stdin: {e}"))?;
        Ok(())
    }

    /// Called by the reader on each `result` event: a pending steer buys one
    /// more turn; otherwise close stdin so the CLI exits and the existing
    /// completion path runs unchanged.
    async fn close_unless_steered(&self) {
        if self
            .steer_pending
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        self.writer.lock().await.take();
    }

    /// Send the opening turn (the goal prompt). Same write path as steer but
    /// without the pending flag — the first result after an unsteered opening
    /// turn must close stdin.
    async fn open_with_prompt(&self, prompt: &str) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut guard = self.writer.lock().await;
        let Some(writer) = guard.as_mut() else {
            return Err("worker stdin closed before the opening prompt".to_string());
        };
        writer
            .write_all(Self::user_message_line(prompt).as_bytes())
            .await
            .map_err(|e| format!("could not send the goal prompt: {e}"))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("could not flush the goal prompt: {e}"))?;
        Ok(())
    }
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
    /// The workflow role the selected worker plays (for the live cache guard).
    /// `None` when the worker yields no role signal.
    pub role: Option<crate::cost_router::WorkflowRole>,
    /// The role's CONFIGURED provider+model (#730 wiring). `Some` ⇒ route the goal
    /// to it; `None` ⇒ clone the parent session's model — the single-model
    /// fallback, never a baked-in vendor default.
    pub model_override: Option<crate::cost_router::RoleModel>,
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

        // Worktree isolation (2026-08-11 bench-contamination bug): the internal
        // subagent used to run in the PRIMARY project root, so verification
        // diffed the operator's own uncommitted work into the worker's verdict
        // (which voided the first three benchmark cells), and concurrent
        // internal goals poisoned each other. Isolate exactly like the
        // external engine — worktree off the baseline on goal/<session_id>,
        // work-base hooks, credential scan, evidence — so landing derives the
        // same branch name it does for CLI workers. A non-git project (no
        // baseline) or a worktree failure degrades LOUDLY to the shared dir
        // rather than failing dispatch: unisolated is how it always ran.
        let mut isolation: Option<(PathBuf, String)> = None;
        if let Some(baseline) = task.baseline_commit.clone() {
            match create_goal_worktree(&task.working_dir, &baseline, &session_id).await {
                Ok(wt) => {
                    let _ = install_work_base_hooks(&wt).await;
                    isolation = Some((wt, baseline));
                }
                Err(e) => tracing::warn!(
                    target: "permagentd::goals",
                    session_id = %session_id,
                    "internal goal worktree creation failed — running UNISOLATED \
                     in the shared project dir (verification may attribute \
                     unrelated tree changes to this worker): {}",
                    e
                ),
            }
        } else {
            tracing::warn!(
                target: "permagentd::goals",
                session_id = %session_id,
                "internal goal has no baseline commit (non-git project?) — \
                 running unisolated"
            );
        }
        let work_dir = isolation
            .as_ref()
            .map(|(wt, _)| wt.clone())
            .unwrap_or_else(|| task.working_dir.clone());

        // Route this goal to its workflow role's CONFIGURED model (#730 wiring)
        // when the orchestrator resolved one; otherwise clone the parent session's
        // provider+model — the single-model fallback, never a baked-in default.
        let (provider_name, model_config) = match &self.model_override {
            Some(rm) => {
                let mc = crate::model::ModelConfig::new(&rm.model)
                    .map(|c| c.with_canonical_limits(&rm.provider))
                    .map_err(|e| {
                        format!(
                            "Failed to build model config for role model {}/{}: {}",
                            rm.provider, rm.model, e
                        )
                    })?;
                (rm.provider.clone(), mc)
            }
            None => (
                self.provider.get_name().to_string(),
                self.provider.get_model_config(),
            ),
        };
        let task_provider =
            providers::create(&provider_name, model_config, self.extensions.clone())
                .await
                .map_err(|e| format!("Failed to create provider for goal dispatch: {}", e))?;

        // Live cache guard: a cache-heavy role routed by the role map to a
        // non-caching provider forfeits the warm-prefix saving. Fires only when the
        // role map selected this provider (not the parent-model fallback).
        if let Some(role) = self.role {
            let supports_cache = task_provider.supports_cache_control().await;
            if crate::cost_router::cache_guard_should_warn(
                role,
                self.model_override.is_some(),
                supports_cache,
            ) {
                tracing::warn!(
                    target: "permagentd::brain",
                    "cost-router cache guard: cache-heavy role '{}' routed to provider '{}' \
                     which has no prompt caching — prefer a caching provider for the {} role",
                    role.as_str(),
                    provider_name,
                    role.as_str()
                );
            }
        }

        let task_config = TaskConfig::new(
            task_provider,
            &session_id,
            &work_dir,
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
                // Isolated runs get the same post-exit treatment as external
                // workers: credential scan first (#508 — a secret must never
                // survive to review), then deterministic evidence from the
                // worktree so verification diffs THE WORKER'S commits, not the
                // shared project tree.
                Ok(_) => match &isolation {
                    Some((wt, baseline)) => {
                        if let Some(reason) = scan_committed_changes(wt, baseline).await {
                            GoalOutcome::Blocked { reason }
                        } else {
                            GoalOutcome::Success(Some(
                                collect_evidence(
                                    wt,
                                    baseline,
                                    "in-process subagent run".to_string(),
                                )
                                .await,
                            ))
                        }
                    }
                    None => GoalOutcome::Success(None),
                },
                Err(e) => GoalOutcome::Failed(e.to_string()),
            }
        });

        Ok(DispatchedWork {
            run_id: session_id,
            join,
            kill,
            // In-process subagents have no stdin protocol; steering them is a
            // different seam (session message), not built yet.
            steer: None,
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
        let mut args: Vec<String> = self
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

        // Worker policy hooks (hardening pass, 2026-08-10): prose in the brief
        // does not bind a worker — one burned 3.85M input tokens waiting on a
        // cold-worktree cargo build it was explicitly told to skip. Claude
        // workers get a PreToolUse hook (via --settings) that BLOCKS the known
        // failure classes deterministically, with a message that teaches the
        // policy instead of a bare denial. Best-effort: a write failure logs
        // and the worker runs unhooked — a missing guard must not kill the
        // dispatch it guards.
        if self.bin.contains("claude") {
            match write_worker_policy_settings(&worktree).await {
                Ok(settings_path) => {
                    args.push("--settings".to_string());
                    args.push(settings_path);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::brain",
                        error = %e,
                        "worker policy hook not installed; worker runs unguarded",
                    );
                }
            }
        }
        let bin = self.bin.clone();
        let timeout = task.timeout;
        let output_tx = task.output_tx;

        // Steering (hardening pass): claude's stream-json stdin protocol makes
        // the worker correctable mid-run. In that mode the `-p` argument is
        // NOT auto-run (proven live) — the goal prompt is sent as the opening
        // stdin message instead, and the reader closes stdin after a result
        // event unless a steer bought another turn.
        let steerable = bin.contains("claude");
        if steerable && !args.iter().any(|a| a == "--input-format") {
            args.push("--input-format".to_string());
            args.push("stream-json".to_string());
        }

        // Spawn the worker NOW (in its own process group) so we can capture its
        // pid for a cancel/timeout group-kill before handing the wait off to the
        // tracker task. A spawn failure here means the goal never started.
        let mut cmd = build_cli_command(&bin, &args, &worktree, steerable);
        // Ephemeral git config for the worker (#523 hooks + #522 push block) —
        // inherited only by this worker's git subprocesses, so the user's repo
        // config is never touched.
        let env_pairs = worker_git_env(work_base_hooks.as_ref());
        cmd.env("GIT_CONFIG_COUNT", env_pairs.len().to_string());
        for (i, (key, value)) in env_pairs.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), key)
                .env(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to run `{}`: {}", bin, e))?;
        let kill = match child.id() {
            Some(pid) => GoalKill::ProcessGroup(pid),
            None => GoalKill::None,
        };
        let pid = child.id();

        let steer = if steerable {
            match child.stdin.take() {
                Some(stdin) => {
                    let handle = std::sync::Arc::new(SteerHandle::new(stdin));
                    handle.open_with_prompt(&prompt).await?;
                    Some(handle)
                }
                None => None,
            }
        } else {
            None
        };

        let reader_steer = steer.clone();
        let join = tokio::spawn(async move {
            await_external_child(
                child,
                pid,
                bin,
                worktree,
                baseline,
                timeout,
                output_tx,
                reader_steer,
            )
            .await
        });

        Ok(DispatchedWork {
            run_id,
            join,
            kill,
            steer,
        })
    }
}

/// Self-knowledge descriptor for the goal landing path — how dispatched work
/// becomes durable, reviewable code. Co-located with the engine per the
/// descriptor convention.
pub const GOAL_LANDING_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "goal_landing",
        display_name: "Goal work landing path",
        category: crate::agents::self_knowledge::FeatureCategory::Guard,
        what_it_does: "Every dispatched goal runs in an isolated git worktree on its own goal/<run-id> branch; when the worker finishes, its commits are credential-scanned and pushed to that goal branch on origin — never to main. Failed and timed-out attempts push their partial work too, so no attempt's commits are ever lost. Landing on main happens only through the user's review and approval, never as a side effect of a worker finishing",
        why_it_matters:
            "When asked where a goal's work went: it is on its goal branch and cited in the review decision — not on main until the user approves. Never tell a user their goal's changes are live before the review gate has passed",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// The branch a goal run's work lives on, derived from its run id. Named (not
/// detached) so the commits are reachable from a ref: a detached worktree was
/// one `git worktree prune` away from GC — three finished goals' commits were
/// orphaned exactly that way on 2026-08-05 and had to be hand-rescued.
pub(crate) fn goal_branch_name(run_id: &str) -> String {
    format!("goal/{}", run_id)
}

/// Create a git worktree at `<repo>/../.permagent-goal-worktrees/<run_id>`
/// checked out at `baseline` on a fresh `goal/<run_id>` branch. Returns the
/// worktree path. The worktree is intentionally *not* removed on completion —
/// its commits are the work product the Decision Inbox review points to, and
/// the branch keeps them alive even if it is.
pub(crate) async fn create_goal_worktree(
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
        .arg("-b")
        .arg(goal_branch_name(run_id))
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
pub(crate) async fn install_work_base_hooks(worktree: &Path) -> Option<PathBuf> {
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

// The #504 reap primitives (`ReapOutcome`, `has_unpushed_work`,
// `remove_worktree_dir`, `git_checked`) were lifted VERBATIM into
// `crate::steward::hygiene` so the goal reaper and the Steward's git-health
// lane share one implementation. Semantics are unchanged; `ReapOutcome` is
// re-exported here so existing paths keep resolving.
pub use crate::steward::hygiene::ReapOutcome;
use crate::steward::hygiene::{has_unpushed_work, remove_worktree_dir};

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
/// The PreToolUse policy script installed into every claude worker's worktree.
///
/// Deterministic enforcement of the two rules prose has already failed to
/// hold (each line cites the incident that earned it):
/// - heavy cargo invocations: a cold goal worktree has no target/ tree, so
///   `cargo build/check/test/clippy` runs for tens of minutes, can outlive the
///   session (the 3.85M-token wait, 2026-08-10) and can fill the disk (three
///   ENOSPC incidents that same weekend). The central gate compiles; workers
///   verify by reading.
/// - `git push`: the #522 rule — Permagent owns the push after the scan. The
///   pushurl sentinel already blocks the push itself; blocking the attempt
///   here means the worker LEARNS instead of burning a turn on a git error.
///
/// Exit 2 blocks the call and the stderr message reaches the model. Grep on
/// the raw hook JSON keeps the script dependency-free (no jq): PreToolUse is
/// matcher-scoped to Bash, so the only `"command"` value present is Bash's.
const WORKER_POLICY_SCRIPT: &str = r#"#!/bin/sh
# Permagent worker policy (generated per-dispatch; see goal_engine.rs).
payload=$(cat)
case "$payload" in
*"cargo build"*|*"cargo check"*|*"cargo test"*|*"cargo clippy"*|*"cargo run"*)
    echo "Blocked by worker policy: heavy cargo invocations are not run in goal worktrees (cold target tree: slow, disk-hungry, and can outlive your session). Verify by reading the code; the central gate compiles after review." >&2
    exit 2
    ;;
*"git push"*)
    echo "Blocked by worker policy: workers never push. Commit locally; Permagent scans and pushes clean work itself." >&2
    exit 2
    ;;
esac
exit 0
"#;

/// Write the worker policy hook + settings into `<worktree>/.permagent/`,
/// returning the settings path for `--settings`. The directory is inside the
/// worktree on purpose: it is reaped with the worktree and never touches the
/// user's own ~/.claude configuration.
async fn write_worker_policy_settings(worktree: &Path) -> Result<String, String> {
    let dir = worktree.join(".permagent");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create {}: {e}", dir.display()))?;

    let script_path = dir.join("worker-policy.sh");
    tokio::fs::write(&script_path, WORKER_POLICY_SCRIPT)
        .await
        .map_err(|e| format!("write {}: {e}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| format!("chmod {}: {e}", script_path.display()))?;
    }

    let settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Bash",
                "hooks": [{ "type": "command", "command": script_path.to_string_lossy() }]
            }]
        }
    });
    let settings_path = dir.join("worker-settings.json");
    tokio::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    )
    .await
    .map_err(|e| format!("write {}: {e}", settings_path.display()))?;

    Ok(settings_path.to_string_lossy().into_owned())
}

fn build_cli_command(bin: &str, args: &[String], working_dir: &Path, steerable: bool) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(working_dir)
        // Steerable (claude) workers keep stdin open for mid-run user
        // messages; everything else gets the old closed stdin so a
        // stdin-reading CLI (codex without a prompt arg) can never hang.
        .stdin(if steerable {
            Stdio::piped()
        } else {
            Stdio::null()
        })
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
#[allow(clippy::too_many_arguments)]
async fn await_external_child(
    child: tokio::process::Child,
    pid: Option<u32>,
    bin: String,
    working_dir: PathBuf,
    baseline: String,
    timeout: Duration,
    output_tx: Option<mpsc::UnboundedSender<WorkerOutputEvent>>,
    steer: Option<std::sync::Arc<SteerHandle>>,
) -> GoalOutcome {
    match tokio::time::timeout(timeout, collect_child_output(child, output_tx, steer)).await {
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
                let worker_summary = worker_closing_summary(
                    &redact_secrets(&String::from_utf8_lossy(&output.stdout)),
                    4000,
                );
                let evidence = collect_evidence(&working_dir, &baseline, worker_summary).await;
                GoalOutcome::Success(Some(evidence))
            } else {
                // W4: failure must not destroy the work. Scan + push whatever
                // was committed to the goal branch so a partial attempt is
                // durable and the retry (or a human) can build on it — a worker
                // that failed at minute 119 with twelve commits used to leave
                // nothing but an error string and a GC-able worktree.
                if scan_committed_changes(&working_dir, &baseline)
                    .await
                    .is_none()
                {
                    push_clean_work(&working_dir, &baseline).await;
                }
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
            // Same W4 preservation as the failure arm: a timed-out worker's
            // commits survive on the goal branch.
            if scan_committed_changes(&working_dir, &baseline)
                .await
                .is_none()
            {
                push_clean_work(&working_dir, &baseline).await;
            }
            GoalOutcome::TimedOut {
                secs: timeout.as_secs(),
            }
        }
    }
}

/// The streaming equivalent of `Child::wait_with_output`: drain stdout and
/// stderr concurrently while waiting, retaining every byte for the existing
/// completion/evidence path. Only stdout produces progress events.
async fn collect_child_output(
    mut child: tokio::process::Child,
    output_tx: Option<mpsc::UnboundedSender<WorkerOutputEvent>>,
    steer: Option<std::sync::Arc<SteerHandle>>,
) -> std::io::Result<std::process::Output> {
    let mut stdout = child.stdout.take().ok_or_else(|| {
        std::io::Error::other("external worker stdout was not configured as piped")
    })?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        std::io::Error::other("external worker stderr was not configured as piped")
    })?;

    let read_stdout = async move {
        let mut accumulated = Vec::new();
        let mut buffer = [0_u8; 8192];
        // Byte offset of the first not-yet-line-scanned byte: `result`-event
        // detection must see each COMPLETE line exactly once, chunk boundaries
        // notwithstanding.
        let mut scanned_to = 0usize;
        loop {
            let read = stdout.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            accumulated.extend_from_slice(&buffer[..read]);
            if let Some(steer) = &steer {
                while let Some(nl) = accumulated[scanned_to..].iter().position(|b| *b == b'\n') {
                    let line = &accumulated[scanned_to..scanned_to + nl];
                    scanned_to += nl + 1;
                    // Cheap containment test on the raw line: the CLI emits one
                    // JSON object per line, and only result events carry this
                    // key at the top level.
                    if line.windows(16).any(|w| w == b"\"type\":\"result\"") {
                        steer.close_unless_steered().await;
                    }
                }
            }
            if let Some(tx) = &output_tx {
                let _ = tx.send(WorkerOutputEvent {
                    observed_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
        Ok::<_, std::io::Error>(accumulated)
    };
    let read_stderr = async move {
        let mut accumulated = Vec::new();
        stderr.read_to_end(&mut accumulated).await?;
        Ok::<_, std::io::Error>(accumulated)
    };

    let (status, stdout, stderr) = tokio::try_join!(child.wait(), read_stdout, read_stderr)?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
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
    let mut cmd = build_cli_command(bin, args, working_dir, false);
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
                None,
                None,
            )
            .await
        }
        Err(e) => GoalOutcome::Failed(format!("Failed to run `{}`: {}", bin, e)),
    }
}

/// P0-3 — the only branch [`push_clean_work`] may publish for `run_id`. A
/// worker with shell access can move its own worktree's HEAD, so the branch it
/// is left on is verified against the run's goal branch rather than trusted:
/// taking it at face value let a worker name `main` and route its commits
/// around the review gate.
fn resolve_push_branch(run_id: &str, head_branch: &str) -> Result<String, String> {
    let expected = goal_branch_name(run_id);
    let branch = if head_branch.is_empty() {
        expected.clone()
    } else {
        head_branch.to_string()
    };
    if branch != expected || crate::steward::is_protected_branch(&branch) {
        return Err(format!(
            "refusing to push run {run_id}: worktree HEAD is on `{branch}`, not the run's goal branch `{expected}` — the goal branch is the only ref the completion path may publish"
        ));
    }
    Ok(branch)
}

/// #522 — the Permagent-owned push: after `scan_committed_changes` passes,
/// push the worker's commits to the run's `goal/<run_id>` branch on `origin`.
/// NEVER to main: work used to land on `origin/main` before completion checks
/// or human review ran, making the Review gate advisory theatre — approve had
/// nothing left to land and reject had no revert. The goal branch makes the
/// work durable and reviewable; landing on main is the approve step's job.
/// Skipped when the worker made no commits. A push failure — unreachable
/// remote, no remote at all — is logged loudly and left for
/// review-in-worktree; it never fails the goal and never bypasses the scan.
/// A worktree left on any branch other than the run's own (see
/// [`resolve_push_branch`]) is refused outright — no fallback name, no push.
pub(crate) async fn push_clean_work(worktree: &Path, baseline: &str) {
    let range = format!("{}..HEAD", baseline);
    let committed = git_text(worktree, &["rev-list", "--count", &range])
        .await
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    if committed == 0 {
        return; // nothing to publish (analysis/docs goal with no commits)
    }

    // Only the run's goal branch may be published, including for detached HEAD.
    let run_id = worktree
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed-run");
    let head = git_text(worktree, &["symbolic-ref", "--short", "HEAD"])
        .await
        .trim()
        .to_string();
    let branch = match resolve_push_branch(run_id, &head) {
        Ok(branch) => branch,
        Err(refusal) => {
            tracing::error!(
                worktree = %worktree.display(),
                commits = committed,
                "{refusal}"
            );
            return;
        }
    };
    let refspec = format!("HEAD:refs/heads/{}", branch);

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(worktree)
        .args(["push", "origin", &refspec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_subprocess(&mut cmd);
    match cmd.output().await {
        Ok(out) if out.status.success() => {
            tracing::info!(
                worktree = %worktree.display(),
                commits = committed,
                branch = %branch,
                "credential scan clean — pushed worker commits to the goal branch (#522)"
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

/// #508 — deterministic credential guard over the worker's *committed* changes
/// (`baseline..HEAD`), run after a clean exit and before the goal is allowed to
/// advance. Every path in every commit that will be pushed is checked from its
/// committed blob, never the working tree, and any enumeration or blob-read
/// failure blocks the goal. Returns a human-readable block reason on the first
/// credential-shaped file/content, or `None` when the changeset is clean.
///
/// Deletions are excluded so a legitimately removed file cannot trip the
/// fail-closed path, as are submodule gitlinks, which carry no blob. Every blob
/// is scanned in full; binary data is inspected as lossy UTF-8 because an
/// incomplete inspection must never be reported clean.
pub(crate) async fn scan_committed_changes(worktree: &Path, baseline: &str) -> Option<String> {
    let range = format!("{}..HEAD", baseline);
    let Some(changes) = git_try(
        worktree,
        &[
            "-c",
            "core.quotepath=false",
            "log",
            "--format=%x00%H",
            "--name-only",
            "--diff-filter=ACMRT",
            &range,
        ],
    )
    .await
    else {
        return Some(format!(
            "Commit blocked by the credential guard: could not enumerate committed changes in \
             `{range}`. The guard fails closed."
        ));
    };

    // `%x00` marks the sha line. Git puts no blank line between one commit's
    // last path and the next commit's sha, and a path can itself be 40 hex
    // characters — matching on shape would read such a file as a commit and
    // never scan it. A path can never contain NUL.
    let mut sha = None;
    let mut scanned = HashSet::new();
    for line in changes.lines().filter(|line| !line.is_empty()) {
        if let Some(commit) = line.strip_prefix('\0') {
            sha = Some(commit);
            continue;
        }
        let Some(sha) = sha else {
            return Some(format!(
                "Commit blocked by the credential guard: could not parse committed path `{line}` \
                 in `{range}`. The guard fails closed."
            ));
        };
        if line.starts_with('"') {
            return Some(format!(
                "Commit blocked by the credential guard: git returned quoted path `{line}`, which \
                 cannot be resolved reliably. The guard fails closed."
            ));
        }
        let path = line;
        if !scanned.insert((sha, path)) {
            continue;
        }
        if let Some(finding) = crate::steward::secret_scan::scan_path(path) {
            return Some(block_message(path, &finding));
        }
        let object = format!("{sha}:{path}");
        let Some(blob) = git_try(worktree, &["show", &object]).await else {
            // A submodule entry has no blob for `git show` to resolve and no
            // content to leak. Confirm it really is a gitlink — an unreadable
            // ordinary file must still block.
            if git_try(worktree, &["ls-tree", sha, "--", path])
                .await
                .is_some_and(|entry| entry.split_whitespace().nth(1) == Some("commit"))
            {
                continue;
            }
            return Some(format!(
                "Commit blocked by the credential guard: could not read committed blob `{path}` \
                 at `{}` to verify it contains no secrets. The guard fails closed.",
                sha.get(..8).unwrap_or(sha)
            ));
        };
        if let Some(finding) = crate::steward::secret_scan::scan_content(&blob) {
            return Some(block_message(path, &finding));
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
/// The worker's own closing statement, extracted from its stdout.
///
/// `claude --output-format stream-json` emits NDJSON, so a raw tail of stdout
/// is a slice of machine transcript — half a JSON object, tool-call plumbing,
/// cache-token accounting. That string is what reached the Decision Inbox as
/// "the worker's summary", so a human asked to approve finished work was shown
/// protocol noise instead of a sentence. It also fed the LLM verdict, which
/// then judged real work as having "no evidence provided".
///
/// The stream's final `{"type":"result","result":"…"}` line carries the actual
/// closing statement. Engines that print plain prose (`codex exec`) have no
/// such line and fall through to the tail unchanged, so this is safe for any
/// external CLI.
fn worker_closing_summary(stdout: &str, max: usize) -> String {
    for line in stdout.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("result") {
            continue;
        }
        if let Some(text) = value.get("result").and_then(serde_json::Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return tail(text, max);
            }
        }
    }
    tail(stdout, max)
}

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

    /// The worker policy hook must block exactly the incident classes it was
    /// written for — and nothing else. Exercised through a real sh, the same
    /// way the claude CLI runs it.
    #[tokio::test]
    async fn worker_policy_blocks_cargo_and_push_but_not_normal_work() {
        let tmp = tempfile::tempdir().unwrap();
        let settings_path = write_worker_policy_settings(tmp.path()).await.unwrap();

        // The settings file names the script with a Bash matcher.
        let settings: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&settings_path).await.unwrap())
                .unwrap();
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        let script = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string();

        let run = |payload: &'static str| {
            let script = script.clone();
            async move {
                let mut child = tokio::process::Command::new("sh")
                    .arg(&script)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                use tokio::io::AsyncWriteExt;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(payload.as_bytes())
                    .await
                    .unwrap();
                let out = child.wait_with_output().await.unwrap();
                (
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).to_string(),
                )
            }
        };

        let (code, msg) =
            run(r#"{"tool_name":"Bash","tool_input":{"command":"cargo check -p permagent"}}"#)
                .await;
        assert_eq!(code, Some(2), "cargo check must be blocked");
        assert!(
            msg.contains("central gate compiles"),
            "the denial teaches: {msg}"
        );

        let (code, msg) =
            run(r#"{"tool_name":"Bash","tool_input":{"command":"git push origin HEAD"}}"#).await;
        assert_eq!(code, Some(2), "git push must be blocked");
        assert!(msg.contains("never push"), "{msg}");

        // Ordinary work sails through — a guard that blocks the job is worse
        // than no guard.
        for ok in [
            r#"{"tool_name":"Bash","tool_input":{"command":"git add -A && git commit -m x"}}"#,
            r#"{"tool_name":"Bash","tool_input":{"command":"grep -rn cargo_toml src/"}}"#,
            r#"{"tool_name":"Bash","tool_input":{"command":"python3 -m unittest discover"}}"#,
        ] {
            let (code, _) = run(ok).await;
            assert_eq!(code, Some(0), "must not block: {ok}");
        }
    }

    /// A real `claude --output-format stream-json` tail: the closing statement
    /// exists, but it is the last line of an NDJSON transcript. Taking a raw
    /// tail handed the Decision Inbox tool-call plumbing and token accounting
    /// as "the worker's summary".
    #[test]
    fn worker_summary_is_the_result_line_not_the_json_transcript() {
        let stdout = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"git commit -q -m x"}}]},"usage":{"cache_read_input_tokens":40043}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"tool_use_id":"t1","type":"tool_result","content":"a719579 docs: add README.md"}]}}"#,
            "\n",
            r#"{"type":"result","subtype":"success","total_cost_usd":0.35,"result":"Committed a719579 — README.md with all eight sections. Not pushed, per worktree policy."}"#,
            "\n",
        );

        let summary = worker_closing_summary(stdout, 4000);

        assert_eq!(
            summary,
            "Committed a719579 — README.md with all eight sections. Not pushed, per worktree policy."
        );
        assert!(
            !summary.contains("cache_read_input_tokens") && !summary.contains("tool_use"),
            "the transcript must not survive into the summary: {summary}"
        );
    }

    /// `codex exec` prints prose, not NDJSON. With no result line to find, the
    /// tail is still the best available summary — the extraction must not
    /// blank it out.
    #[test]
    fn worker_summary_falls_back_to_the_tail_for_plain_prose_engines() {
        let stdout = "Created PROOF.txt containing OK.\nCommitted locally as fd39d17.\n";
        assert_eq!(worker_closing_summary(stdout, 4000), stdout);
    }

    /// A result line with an empty payload is not a summary — keep looking
    /// rather than reporting nothing.
    #[test]
    fn worker_summary_ignores_an_empty_result_payload() {
        let stdout = concat!(
            r#"{"type":"result","result":"   "}"#,
            "\n",
            "trailing prose the worker printed\n",
        );
        assert!(worker_closing_summary(stdout, 4000).contains("trailing prose"));
    }

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

    /// The receipt timestamp must describe the first stdout bytes, not process
    /// completion. Keep collecting the complete stream after emitting it.
    #[cfg(unix)]
    #[tokio::test]
    async fn streaming_output_reports_first_bytes_before_child_exit() {
        let mut cmd = build_cli_command(
            "/bin/sh",
            &[
                "-c".to_string(),
                "printf first; sleep 2; printf second".to_string(),
            ],
            Path::new("."),
            false,
        );
        let child = cmd.spawn().expect("spawn fake streaming worker");
        let (output_tx, mut output_rx) = mpsc::unbounded_channel();
        let collector = tokio::spawn(collect_child_output(child, Some(output_tx), None));

        let event = tokio::time::timeout(Duration::from_secs(1), output_rx.recv())
            .await
            .expect("first stdout event must arrive promptly")
            .expect("stdout event channel closed unexpectedly");
        let first_output_at = chrono::DateTime::parse_from_rfc3339(&event.observed_at)
            .expect("event timestamp is RFC3339");
        let mut receipt =
            crate::agents::platform_extensions::execution_receipt::ExecutionReceipt::new(
                "worker",
                "session",
                serde_json::Value::Null,
                "lifecycle",
                chrono::Utc::now().to_rfc3339(),
                1,
            );
        receipt.observe_output(event.observed_at.clone(), chrono::Utc::now().to_rfc3339());
        assert_eq!(
            receipt.first_output_at.as_deref(),
            Some(event.observed_at.as_str())
        );
        assert!(
            !collector.is_finished(),
            "first_output_at must be stamped while the child is still running"
        );

        let output = collector.await.unwrap().unwrap();
        let exited_at = chrono::Utc::now();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"firstsecond");
        assert!(
            exited_at
                .signed_duration_since(first_output_at)
                .num_milliseconds()
                >= 1_000,
            "first_output_at was stamped too close to child exit"
        );
    }

    /// #490: a cancel must actually stop the worker. Spawn a long sleep in its
    /// own process group, group-kill it by pid, and confirm it is reaped fast
    /// (not left running for its full duration).
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_process_group_reaps_the_worker() {
        let mut cmd = build_cli_command("sleep", &["30".to_string()], Path::new("."), false);
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

        // Delete-after-commit bypass: the blob is gone at HEAD but remains in
        // the pushed chain, so scanning from the original baseline must block.
        g(&["rm", "-q", ".env"]);
        g(&["commit", "-q", "-m", "remove secret"]);
        let reason = scan_committed_changes(&repo, &baseline)
            .await
            .expect("deleted secret blob in the pushed chain must be blocked");
        assert!(reason.contains(".env"), "reason names the file: {reason}");

        // The deletion commit alone (new baseline) is clean.
        let after_del = String::from_utf8(g(&["rev-parse", "HEAD~1"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        assert!(scan_committed_changes(&repo, &after_del).await.is_none());
    }

    /// #508: committed blob content is scanned even when a later commit leaves
    /// the working tree and net tree diff clean.
    #[tokio::test]
    async fn scan_committed_changes_blocks_secret_removed_from_working_tree() {
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
        std::fs::write(repo.join("config.txt"), "clean\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);
        let baseline = String::from_utf8(g(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();

        std::fs::write(
            repo.join("config.txt"),
            "-----BEGIN RSA PRIVATE KEY-----\nsecret\n-----END RSA PRIVATE KEY-----\n",
        )
        .unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "commit secret"]);
        std::fs::write(repo.join("config.txt"), "clean\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "clean working tree"]);

        let reason = scan_committed_changes(&repo, &baseline)
            .await
            .expect("historical committed secret blob must be blocked");
        assert!(
            reason.contains("config.txt"),
            "reason names the file: {reason}"
        );
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
            output_tx: None,
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

    /// The run's own goal branch is the only accepted name; a detached HEAD
    /// resolves to it, and anything else — main, another run's goal branch —
    /// is refused with the run id named so the log identifies the run.
    #[test]
    fn resolve_push_branch_only_accepts_the_runs_goal_branch() {
        let run_id = "cli-resolve-push";
        let expected = goal_branch_name(run_id);
        assert_eq!(resolve_push_branch(run_id, &expected), Ok(expected.clone()));
        assert_eq!(resolve_push_branch(run_id, ""), Ok(expected));

        let main_error = resolve_push_branch(run_id, "main").unwrap_err();
        assert!(main_error.contains(run_id));
        assert!(main_error.contains("`main`"));
        assert!(resolve_push_branch(run_id, "goal/another-run").is_err());
    }

    /// After a clean scan, Permagent's own push (no blocking env) publishes
    /// the worker's commits to the run's goal branch — NEVER to main; with no
    /// commits it is a no-op.
    #[tokio::test]
    async fn push_clean_work_publishes_commits_to_goal_branch_not_main() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let wt = create_goal_worktree(&repo, &baseline, "cli-permagent-push")
            .await
            .unwrap();

        // No commits: no-op, origin untouched.
        push_clean_work(&wt, &baseline).await;
        let goal_ref = "refs/heads/goal/cli-permagent-push";
        let tip = String::from_utf8(git(&repo, &["ls-remote", "origin", goal_ref]).stdout).unwrap();
        assert!(tip.trim().is_empty(), "no-commit run must push nothing");

        // With a commit: the goal branch appears at the worktree HEAD, and
        // origin/main does NOT move — landing on main is the approve step's
        // job, not the completion path's.
        commit_in_worktree(&wt, "work.txt");
        let head = String::from_utf8(git(&wt, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        push_clean_work(&wt, &baseline).await;
        let tip = String::from_utf8(git(&repo, &["ls-remote", "origin", goal_ref]).stdout).unwrap();
        assert!(
            tip.starts_with(&head),
            "the goal branch must be at the worker's HEAD after the Permagent-owned push"
        );
        let main_tip =
            String::from_utf8(git(&repo, &["ls-remote", "origin", "refs/heads/main"]).stdout)
                .unwrap();
        assert!(
            main_tip.starts_with(&baseline),
            "origin/main must NOT move on the completion path"
        );
    }

    /// P0-3: a worker with shell access moves its worktree onto `main` and
    /// carries its commits there. The Permagent-owned push must refuse rather
    /// than land the work on origin/main, outside the review gate.
    #[tokio::test]
    async fn push_clean_work_refuses_worker_moved_head() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, baseline) = init_repo_with_remote(tmp.path());
        let run_id = "cli-moved-head";
        let wt = create_goal_worktree(&repo, &baseline, run_id)
            .await
            .unwrap();
        commit_in_worktree(&wt, "work.txt");

        // `checkout -b main` is refused (main is checked out in the primary
        // worktree), but moving the ref and then HEAD is not. Main must end up
        // carrying the worker's commits: otherwise the no-commits gate returns
        // before the branch is ever resolved and the test proves nothing.
        let work_head = String::from_utf8(git(&wt, &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        assert!(git(&wt, &["update-ref", "refs/heads/main", &work_head])
            .status
            .success());
        assert!(git(&wt, &["symbolic-ref", "HEAD", "refs/heads/main"])
            .status
            .success());

        push_clean_work(&wt, &baseline).await;

        let main_tip =
            String::from_utf8(git(&repo, &["ls-remote", "origin", "refs/heads/main"]).stdout)
                .unwrap();
        assert!(main_tip.starts_with(&baseline), "origin/main must not move");
        let goal_ref = format!("refs/heads/{}", goal_branch_name(run_id));
        let goal_tip =
            String::from_utf8(git(&repo, &["ls-remote", "origin", &goal_ref]).stdout).unwrap();
        assert!(
            goal_tip.trim().is_empty(),
            "the goal branch must not be pushed"
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
