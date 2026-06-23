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
    Success,
    /// Retriable failure within budget — routes through the existing
    /// budget/retry logic in `handle_goal_completion`.
    Failed(String),
    /// The worker exceeded its time bound. Routes to an unconditional PARK
    /// (`handle_goal_timeout`) — never a silent retry.
    TimedOut { secs: u64 },
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
/// (recorded in card metadata as the worker session id) plus the join handle
/// the tracker awaits.
pub struct DispatchedWork {
    pub run_id: String,
    pub join: JoinHandle<GoalOutcome>,
}

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
                Ok(_) => GoalOutcome::Success,
                Err(e) => GoalOutcome::Failed(e.to_string()),
            }
        });

        Ok(DispatchedWork {
            run_id: session_id,
            join,
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

        let join =
            tokio::spawn(async move { run_external_cli(&bin, &args, &worktree, timeout).await });

        Ok(DispatchedWork { run_id, join })
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
            tail(&String::from_utf8_lossy(&output.stderr), 2000)
        ));
    }
    Ok(dest)
}

/// Spawn the external CLI in `working_dir`, bounded by `timeout`. Exit 0 →
/// `Success`; nonzero → `Failed(stderr tail)`; timeout → `TimedOut` (the
/// process is killed via `kill_on_drop`).
async fn run_external_cli(
    bin: &str,
    args: &[String],
    working_dir: &Path,
    timeout: Duration,
) -> GoalOutcome {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_subprocess(&mut cmd);

    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                GoalOutcome::Success
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
                    tail(&String::from_utf8_lossy(&output.stderr), 2000)
                ))
            }
        }
        Ok(Err(e)) => GoalOutcome::Failed(format!("Failed to run `{}`: {}", bin, e)),
        Err(_) => GoalOutcome::TimedOut {
            secs: timeout.as_secs(),
        },
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            Duration::from_secs(1),
        )
        .await;
        assert!(
            matches!(outcome, GoalOutcome::TimedOut { secs: 1 }),
            "expected TimedOut, got {:?}",
            outcome
        );
    }

    #[tokio::test]
    async fn external_cli_nonzero_exit_is_failure() {
        let outcome = run_external_cli(
            "sh",
            &["-c".to_string(), "exit 3".to_string()],
            Path::new("."),
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
        let outcome = run_external_cli("true", &[], Path::new("."), Duration::from_secs(10)).await;
        assert!(
            matches!(outcome, GoalOutcome::Success),
            "expected Success, got {:?}",
            outcome
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
}
