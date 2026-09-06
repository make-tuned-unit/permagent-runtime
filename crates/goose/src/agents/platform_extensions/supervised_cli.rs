//! S1 (#427, epic #399): launch a SUPERVISED Claude Code session — stream-json
//! in/out, permission gates ENABLED — in a visible Build-tab terminal.
//!
//! The S0 reconciliation (#426, `docs/architecture/CONTROL_LOOP_SPEC.md`) ruled
//! that #424's `ExternalCliEngine` is gate-bypassed and headless
//! (`--dangerously-skip-permissions`, `stdin: null`, buffered `cmd.output()`),
//! so the supervised path is a SIBLING engine, not an extension. This module is
//! that sibling: it composes the gate-enabled stream-json invocation
//! (`claude -p --output-format stream-json --input-format stream-json --verbose
//! --permission-prompt-tool stdio` — the `--dangerously-skip-permissions` roster
//! arg is deliberately dropped) and launches it into the EXISTING visible
//! Build-tab PTY path: it emits the same `project_launch` event the
//! command-center already consumes (`useAppNavigate` → `BuildView` →
//! `TerminalManager.createProjectTab` → Tauri `spawn_pty_session` +
//! `write_to_pty(command)`), per the Fork-1 ruling ("PTY in the Build-tab
//! terminal; reuse the existing terminal.rs / Build-tab infra").
//!
//! Two entry points share the same session machinery (ruling — the
//! supervised session is NOT always a goal):
//! - **Dispatched-goal** ([`SupervisedCliEngine`], a [`GoalEngine`] impl):
//!   reuses #424's worktree / evidence / credential-guard scaffolding.
//! - **Free-standing watched session** ([`launch_watched_session`]): "just run
//!   CC" — no worktree, no goal card, no review; runs at the project root.
//!
//! What S1 deliberately does NOT do (later slices, do not fold in):
//! - No output parsing / gate detection — S2 (#428) tees `pty_data` into the
//!   NDJSON parser. This module only exposes the completion seam
//!   ([`complete_supervised_session`]) S2 will call on `type:"result"`.
//! - No Decision-Inbox bridge (S3, #429) and no relay of answers into stdin
//!   (S5, #431). Until those land, a fired gate simply sits VISIBLE in the tab
//!   waiting; a hand-typed `control_response` NDJSON line is the escape hatch.
//!
//! ## Why the composed command looks the way it does
//!
//! The daemon cannot spawn into the Tauri-owned PTY directly — the frontend
//! types the command into the tab's shell. Three consequences:
//! - `-p` is explicit: `--input-format`/`--output-format` only work with
//!   `--print` (CLI help), and inside a PTY both stdio ends are a TTY, so
//!   without `-p` the CLI would open its interactive TUI and reject the flags.
//! - The initial goal prompt travels via a daemon-written FILE, never inline:
//!   the command line is interpreted by the user's shell, and an LLM-composed
//!   prompt must not be shell-interpreted. `cat '<file>' - | claude …` feeds
//!   the prompt first, then keeps the PTY wired through to claude's stdin so
//!   the S5 relay (and hand-typing today) can reach the session.
//! - Only daemon-controlled strings (paths, the binary name, git-config pairs)
//!   appear on the command line, each single-quoted; control characters are
//!   rejected outright (fail closed) so a hostile path cannot smuggle a second
//!   command past the quoting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::sync::oneshot;

use super::goal_engine::{
    collect_evidence, create_goal_worktree, install_work_base_hooks, push_clean_work,
    scan_committed_changes, worker_git_env, DispatchedWork, GoalEngine, GoalKill, GoalOutcome,
    GoalTask, COMMIT_ONLY_BRIEF,
};

/// Prefix of every supervised-session id (`sup-<uuid>`). Distinct from #424's
/// `cli-<uuid>` run ids so registries, worktree sweeps, and logs can tell the
/// two engine families apart at a glance.
pub const SUPERVISED_SESSION_PREFIX: &str = "sup-";

/// Default binary for a supervised session. Only Claude Code speaks the
/// stream-json control protocol this loop is built on.
pub const SUPERVISED_CLI_DEFAULT_BIN: &str = "claude";

/// The FIXED flag roster of a supervised session. Composed here — never
/// operator-configurable — because every flag is load-bearing for the loop:
/// `-p` forces print mode inside the TTY, the two stream-json flags carry the
/// structured protocol, `--verbose` is required by stream-json output, and
/// `--permission-prompt-tool stdio` makes gates arrive as `control_request`
/// NDJSON lines instead of being auto-approved. `--dangerously-skip-permissions`
/// is deliberately ABSENT — skipping permissions defeats gate detection (the S0
/// finding that forced this sibling engine).
fn supervised_stream_json_args() -> Vec<&'static str> {
    vec![
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
    ]
}

/// Single-quote `s` for the PTY shell line. Fails closed on control characters
/// (a `\n` would submit a second command; `\r`, `\0`, escapes are equally
/// hostile) — every quoted string here is daemon-controlled, so a control char
/// is a bug or an attack, never legitimate.
fn shell_single_quote(s: &str) -> Result<String, String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(format!(
            "refusing to shell-quote a string containing control characters: {:?}",
            s
        ));
    }
    Ok(format!("'{}'", s.replace('\'', "'\\''")))
}

/// The initial stream-json user message carrying the goal prompt — the same
/// wire shape `claude_code.rs::build_stream_json_input` sends the headless
/// provider process. One NDJSON line, newline-terminated.
pub fn initial_prompt_line(prompt: &str, session_id: &str) -> String {
    let msg = serde_json::json!({
        "type": "user",
        "session_id": session_id,
        "message": { "role": "user", "content": [ { "type": "text", "text": prompt } ] }
    });
    let mut line = serde_json::to_string(&msg).expect("serializing a JSON string cannot fail");
    line.push('\n');
    line
}

/// Directory holding the daemon-written initial-prompt files, keyed by session
/// id. Lives under the OS temp dir: the prompt is transient launch input, not
/// work product (the work product is the worktree's commits).
fn prompt_file_dir() -> PathBuf {
    std::env::temp_dir().join("permagent-supervised")
}

/// Write the initial prompt NDJSON to a daemon-controlled file and return its
/// path. The file path — not the prompt content — is what appears (quoted) on
/// the shell command line.
async fn write_prompt_file(session_id: &str, prompt: &str) -> Result<PathBuf, String> {
    let dir = prompt_file_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("failed to create supervised prompt dir: {}", e))?;
    let path = dir.join(format!("{session_id}.prompt.ndjson"));
    tokio::fs::write(&path, initial_prompt_line(prompt, session_id))
        .await
        .map_err(|e| format!("failed to write supervised prompt file: {}", e))?;
    Ok(path)
}

/// Compose the full shell line the Build-tab terminal runs.
///
/// Shape: `[cat '<prompt_file>' - | ][GIT_CONFIG_* env ]'<bin>' <fixed flags>`.
/// The `cat '<file>' -` prefix feeds the initial prompt, then forwards the PTY
/// (stdin `-`) so the session's stdin stays reachable for relay/hand-typing.
/// The git-config env pairs (worktree push block + work-base hooks, #522/#523)
/// prefix the CLI member of the pipeline — a plain `FOO=1 a | b` would apply
/// them to `cat`, not the worker.
pub(crate) fn compose_supervised_command(
    bin: &str,
    prompt_file: Option<&Path>,
    git_env: &[(String, String)],
) -> Result<String, String> {
    let mut cmd = String::new();
    if let Some(f) = prompt_file {
        cmd.push_str("cat ");
        cmd.push_str(&shell_single_quote(&f.to_string_lossy())?);
        cmd.push_str(" - | ");
    }
    if !git_env.is_empty() {
        cmd.push_str(&format!("GIT_CONFIG_COUNT={} ", git_env.len()));
        for (i, (key, value)) in git_env.iter().enumerate() {
            cmd.push_str(&format!(
                "GIT_CONFIG_KEY_{i}={} GIT_CONFIG_VALUE_{i}={} ",
                shell_single_quote(key)?,
                shell_single_quote(value)?
            ));
        }
    }
    cmd.push_str(&shell_single_quote(bin)?);
    for arg in supervised_stream_json_args() {
        cmd.push(' ');
        cmd.push_str(arg);
    }
    Ok(cmd)
}

// ── Completion seam (fulfilled by S2's parser; awaited by the goal tracker) ──

/// Terminal outcome of a supervised session, reported through
/// [`complete_supervised_session`].
#[derive(Debug)]
pub enum SupervisedOutcome {
    /// The session's stream-json `type:"result"` arrived (S2 detects this).
    /// `summary` is the result text, persisted as the worker self-report.
    Completed { summary: String },
    /// The session failed (error result / abnormal exit).
    Failed { reason: String },
}

/// Pending completion hooks keyed by supervised session id. S1 owns only this
/// narrow seam — the full `{session → project, goal, PTY, request_id}` bridge
/// registry is S2's slice (spec: "S2's registry must bridge …").
static COMPLETION_HOOKS: Lazy<Mutex<HashMap<String, oneshot::Sender<SupervisedOutcome>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Register a completion hook for `session_id`, returning the receiver the
/// dispatch tracker awaits. `pub(crate)` so S2's registry tests can prove the
/// parser fulfils the seam end-to-end.
pub(crate) fn register_completion_hook(session_id: &str) -> oneshot::Receiver<SupervisedOutcome> {
    let (tx, rx) = oneshot::channel();
    if let Ok(mut map) = COMPLETION_HOOKS.lock() {
        map.insert(session_id.to_string(), tx);
    }
    rx
}

/// Drop a session's completion hook (timeout cleanup — avoid a leaked entry).
fn drop_completion_hook(session_id: &str) {
    if let Ok(mut map) = COMPLETION_HOOKS.lock() {
        map.remove(session_id);
    }
}

/// Report a supervised session's terminal outcome. Returns `true` when a
/// dispatched goal was awaiting it (the hook existed), `false` for an unknown /
/// already-completed / free-standing session (free-standing sessions register
/// no hook — nothing tracks their completion in S1).
///
/// PRODUCTION CALLER: S2's stream-json parser
/// (`terminal_supervision::ingest_output`), on the session's `type:"result"`
/// line — or on PTY close without one. A dispatched supervised goal that
/// nobody completes still runs to its wall-clock timeout and PARKS (never a
/// silent retry), exactly like a hung external worker.
pub fn complete_supervised_session(session_id: &str, outcome: SupervisedOutcome) -> bool {
    let tx = match COMPLETION_HOOKS.lock() {
        Ok(mut map) => map.remove(session_id),
        Err(_) => None,
    };
    match tx {
        Some(tx) => tx.send(outcome).is_ok(),
        None => false,
    }
}

// ── Free-standing watched session ("just run CC") ──────────────────────────

/// What a launch produced: the loop session id (the future correlation key)
/// and the exact shell command handed to the visible tab.
#[derive(Debug)]
pub struct SupervisedLaunch {
    pub session_id: String,
    pub shell_command: String,
}

/// Launch a free-standing watched Claude Code session in a visible Build-tab
/// terminal at `root_path`. No worktree, no goal card, no review — the session
/// is supervised by being VISIBLE (and, from S2 on, parsed).
///
/// `prompt` is the optional initial instruction (Henry composes it — the single
/// LLM-in-loop point of S1); when absent the session idles waiting for
/// stream-json input on its stdin (hand-typed today, relayed from S5 on).
pub async fn launch_watched_session(
    root_path: &str,
    label: &str,
    project_slug: &str,
    prompt: Option<&str>,
    reason: &str,
) -> Result<SupervisedLaunch, String> {
    let session_id = format!("{SUPERVISED_SESSION_PREFIX}{}", uuid::Uuid::new_v4());
    let prompt_file = match prompt.filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(write_prompt_file(&session_id, p).await?),
        None => None,
    };
    let shell_command =
        compose_supervised_command(SUPERVISED_CLI_DEFAULT_BIN, prompt_file.as_deref(), &[])?;

    // S2 (#428): register with the supervision registry BEFORE the launch
    // event goes out, so the PTY attach that follows can never hit an unknown
    // session.
    super::terminal_supervision::register_session(
        &session_id,
        super::terminal_supervision::SupervisedSessionKind::Watched,
        project_slug,
        root_path,
    );

    crate::events::emit(crate::events::project_launch(
        root_path,
        label,
        Some(&shell_command),
        project_slug,
        reason,
        Some(&session_id),
    ));

    Ok(SupervisedLaunch {
        session_id,
        shell_command,
    })
}

// ── Dispatched-goal engine (the #424 sibling) ──────────────────────────────

/// [`GoalEngine`] that launches the goal as a SUPERVISED stream-json Claude
/// Code session in a visible Build-tab PTY — the S1 sibling of
/// [`ExternalCliEngine`](super::goal_engine::ExternalCliEngine).
///
/// Reuses #424's goal scaffolding (worktree off the baseline, work-base hooks,
/// push block, credential scan, evidence collection) and replaces the
/// invocation: gates ENABLED, stdin writable (via the PTY), output visible in
/// the tab. The process itself is spawned by the frontend's terminal path, so:
///
/// - `kill` is [`GoalKill::None`] — the daemon holds no process handle. Cancel
///   marks the goal terminal; the session stays visible in its tab for the
///   user to close. (S5's relay guard work is where a daemon-side stop lands.)
/// - completion is the [`complete_supervised_session`] seam. Until S2's parser
///   calls it, a dispatched supervised goal runs to its timeout and PARKS.
pub struct SupervisedCliEngine {
    pub bin: String,
    /// Slug of the goal's project — used for the tab label and the launch
    /// event payload.
    pub project_slug: String,
    /// `(system_prompt_block, display_name)` prepended to the goal prompt,
    /// mirroring the other engines.
    pub persona_override: Option<(String, String)>,
}

impl SupervisedCliEngine {
    fn build_prompt(&self, instructions: &str) -> String {
        let base = match &self.persona_override {
            Some((block, _)) if !block.is_empty() => format!("{}\n\n{}", block, instructions),
            _ => instructions.to_string(),
        };
        // Same commit-only discipline as the external engine: the worktree's
        // push is blocked (#522) and Permagent pushes after the credential scan.
        format!("{}{}", base, COMMIT_ONLY_BRIEF)
    }
}

#[async_trait]
impl GoalEngine for SupervisedCliEngine {
    async fn spawn(&self, task: GoalTask) -> Result<DispatchedWork, String> {
        let baseline = task.baseline_commit.clone().ok_or_else(|| {
            "Supervised-CLI dispatch requires a git baseline commit, but the project root is not \
             a git repository"
                .to_string()
        })?;

        let session_id = format!("{SUPERVISED_SESSION_PREFIX}{}", uuid::Uuid::new_v4());
        let worktree = create_goal_worktree(&task.working_dir, &baseline, &session_id).await?;

        // #523 work-base hooks + #522 push block, exactly as the external
        // engine installs them — evidence and the Permagent-owned push depend
        // on both. Delivered as GIT_CONFIG_* env on the CLI member of the
        // composed pipeline (the daemon spawns no process here).
        let hooks_dir = install_work_base_hooks(&worktree).await;
        let git_env = worker_git_env(hooks_dir.as_ref());

        let prompt = self.build_prompt(&task.instructions);
        let prompt_file = write_prompt_file(&session_id, &prompt).await?;
        let shell_command = compose_supervised_command(&self.bin, Some(&prompt_file), &git_env)?;

        let rx = register_completion_hook(&session_id);

        // S2 (#428): register with the supervision registry BEFORE the launch
        // event goes out (attach must never race an unknown session). The
        // session id doubles as the goal's run_id/worker_session_id.
        super::terminal_supervision::register_session(
            &session_id,
            super::terminal_supervision::SupervisedSessionKind::DispatchedGoal,
            &self.project_slug,
            &worktree.to_string_lossy(),
        );

        let label = format!("{} · {} (supervised)", self.project_slug, self.bin);
        let reason = format!(
            "Supervised {} session for goal \"{}\" — gates will surface here",
            self.bin, task.card_title
        );
        crate::events::emit(crate::events::project_launch(
            &worktree.to_string_lossy(),
            &label,
            Some(&shell_command),
            &self.project_slug,
            &reason,
            Some(&session_id),
        ));

        let timeout = task.timeout;
        let join_session_id = session_id.clone();
        let join = tokio::spawn(async move {
            await_supervised_completion(join_session_id, rx, worktree, baseline, timeout).await
        });

        Ok(DispatchedWork {
            run_id: session_id,
            process_id: None,
            join,
            kill: GoalKill::None,
            // Supervised sessions are steered by the human IN the terminal.
            steer: None,
        })
    }
}

/// Await the session's reported outcome (the [`complete_supervised_session`]
/// seam), bounded by the goal's wall-clock timeout. Mirrors the external
/// engine's completion contract: success runs the credential scan (fail-closed
/// `Blocked`), then the Permagent-owned push, then evidence collection; timeout
/// yields `TimedOut` (the tracker parks the goal — never a silent retry).
async fn await_supervised_completion(
    session_id: String,
    rx: oneshot::Receiver<SupervisedOutcome>,
    worktree: PathBuf,
    baseline: String,
    timeout: Duration,
) -> GoalOutcome {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(SupervisedOutcome::Completed { summary })) => {
            if let Some(reason) = scan_committed_changes(&worktree, &baseline).await {
                return GoalOutcome::Blocked { reason };
            }
            push_clean_work(&worktree, &baseline).await;
            let evidence = collect_evidence(&worktree, &baseline, summary).await;
            GoalOutcome::Success(Some(evidence))
        }
        Ok(Ok(SupervisedOutcome::Failed { reason })) => {
            // W4: preserve partial work on the goal branch (see the external
            // engine's failure arm for the rationale).
            if scan_committed_changes(&worktree, &baseline).await.is_none() {
                push_clean_work(&worktree, &baseline).await;
            }
            GoalOutcome::Failed(reason)
        }
        Ok(Err(_)) => GoalOutcome::Failed(
            "supervised session completion hook dropped without an outcome".to_string(),
        ),
        Err(_) => {
            drop_completion_hook(&session_id);
            if scan_committed_changes(&worktree, &baseline).await.is_none() {
                push_clean_work(&worktree, &baseline).await;
            }
            GoalOutcome::TimedOut {
                secs: timeout.as_secs(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{PermagentEvent, PermagentEventType};
    use std::process::Command as StdCommand;

    /// Drain the (shared, global) event bus looking for THIS session's launch
    /// frame. Filters by our unique session id — parallel tests emit onto the
    /// same bus — and tolerates `Lagged` (skipped frames were not ours to
    /// find… but if the bus lagged past our frame the test fails loudly at the
    /// `expect`, never silently).
    fn find_launch_event(
        rx: &mut tokio::sync::broadcast::Receiver<PermagentEvent>,
        session_id: &str,
    ) -> Option<PermagentEvent> {
        use tokio::sync::broadcast::error::TryRecvError;
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if ev.event_type == PermagentEventType::ProjectLaunch
                        && ev.payload["supervised_session_id"] == session_id
                    {
                        return Some(ev);
                    }
                }
                Err(TryRecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    }

    // ── Command composition ────────────────────────────────────────────────

    #[test]
    fn flag_roster_is_gate_enabled_stream_json() {
        let cmd = compose_supervised_command("claude", None, &[]).unwrap();
        assert!(cmd.contains("-p "), "print mode must be explicit: {cmd}");
        assert!(cmd.contains("--output-format stream-json"), "{cmd}");
        assert!(cmd.contains("--input-format stream-json"), "{cmd}");
        assert!(cmd.contains("--verbose"), "{cmd}");
        assert!(cmd.contains("--permission-prompt-tool stdio"), "{cmd}");
        assert!(
            !cmd.contains("--dangerously-skip-permissions"),
            "supervised sessions must NEVER skip permissions: {cmd}"
        );
        // No prompt file, no env: the command starts at the quoted binary.
        assert!(cmd.starts_with("'claude'"), "{cmd}");
    }

    #[test]
    fn prompt_file_is_fed_then_stdin_forwarded() {
        let cmd = compose_supervised_command(
            "claude",
            Some(Path::new("/tmp/permagent-supervised/sup-x.prompt.ndjson")),
            &[],
        )
        .unwrap();
        assert!(
            cmd.starts_with("cat '/tmp/permagent-supervised/sup-x.prompt.ndjson' - | "),
            "prompt must be fed first, then the PTY forwarded (`-`): {cmd}"
        );
    }

    #[test]
    fn git_env_prefixes_the_cli_member_not_cat() {
        let env = vec![
            (
                "remote.origin.pushurl".to_string(),
                "permagent-credential-guard://push-disabled".to_string(),
            ),
            ("core.hooksPath".to_string(), "/wt/.git/hooks x".to_string()),
        ];
        let cmd =
            compose_supervised_command("claude", Some(Path::new("/tmp/p.ndjson")), &env).unwrap();
        // Slice-free split (workspace clippy::string_slice).
        let cli_member = cmd.split(" | ").nth(1).expect("pipeline present");
        assert!(
            cli_member.starts_with("GIT_CONFIG_COUNT=2 "),
            "env must prefix the CLI member, not cat: {cmd}"
        );
        assert!(cli_member.contains("GIT_CONFIG_KEY_0='remote.origin.pushurl'"));
        assert!(cli_member.contains("GIT_CONFIG_VALUE_1='/wt/.git/hooks x'"));
    }

    #[test]
    fn quoting_handles_spaces_and_quotes_and_rejects_control_chars() {
        assert_eq!(shell_single_quote("a b").unwrap(), "'a b'");
        assert_eq!(shell_single_quote("a'b").unwrap(), "'a'\\''b'");
        assert!(shell_single_quote("evil\ninjection").is_err());
        assert!(shell_single_quote("evil\rinjection").is_err());
        assert!(shell_single_quote("evil\x1b[31m").is_err());
        // The composition surface fails closed too.
        assert!(compose_supervised_command("cl\naude", None, &[]).is_err());
        assert!(compose_supervised_command("claude", Some(Path::new("/tmp/a\nb")), &[]).is_err());
    }

    /// The composed line must round-trip through a REAL shell without the
    /// prompt content ever touching shell interpretation: a path with spaces
    /// and a quote character still resolves to one file.
    #[test]
    fn composed_command_survives_a_real_shell() {
        let dir = tempfile::tempdir().unwrap();
        let tricky = dir.path().join("weird 'name.ndjson");
        std::fs::write(&tricky, "{\"type\":\"user\"}\n").unwrap();
        // `cat 'file' -` with stdin closed: prints the file then EOF.
        let cmd = format!(
            "cat {} -",
            shell_single_quote(&tricky.to_string_lossy()).unwrap()
        );
        let out = StdCommand::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{cmd}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "{\"type\":\"user\"}\n"
        );
    }

    // ── Initial prompt NDJSON ──────────────────────────────────────────────

    #[test]
    fn initial_prompt_line_matches_stream_json_wire_shape() {
        let line = initial_prompt_line("Fix the bug in foo.rs", "sup-abc");
        assert!(line.ends_with('\n'), "must be one newline-terminated line");
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["session_id"], "sup-abc");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"][0]["type"], "text");
        assert_eq!(v["message"]["content"][0]["text"], "Fix the bug in foo.rs");
    }

    // ── Completion seam ────────────────────────────────────────────────────

    #[tokio::test]
    async fn completion_hook_delivers_outcome() {
        let rx = register_completion_hook("sup-hook-1");
        assert!(complete_supervised_session(
            "sup-hook-1",
            SupervisedOutcome::Completed {
                summary: "done".to_string()
            }
        ));
        match rx.await.unwrap() {
            SupervisedOutcome::Completed { summary } => assert_eq!(summary, "done"),
            other => panic!("expected Completed, got {:?}", other),
        }
        // Second completion for the same id: hook is consumed.
        assert!(!complete_supervised_session(
            "sup-hook-1",
            SupervisedOutcome::Failed {
                reason: "x".to_string()
            }
        ));
    }

    #[test]
    fn completing_an_unknown_session_is_a_noop() {
        assert!(!complete_supervised_session(
            "sup-never-registered",
            SupervisedOutcome::Failed {
                reason: "x".to_string()
            }
        ));
    }

    // ── Free-standing launch ───────────────────────────────────────────────

    #[tokio::test]
    async fn watched_session_emits_project_launch_with_session_id() {
        let mut rx = crate::events::subscribe();
        let launch = launch_watched_session(
            "/tmp/some-project",
            "proj · claude (supervised)",
            "proj",
            Some("Investigate the flaky test"),
            "Opening a supervised Claude Code session",
        )
        .await
        .unwrap();

        assert!(launch.session_id.starts_with(SUPERVISED_SESSION_PREFIX));
        assert!(launch
            .shell_command
            .contains("--permission-prompt-tool stdio"));
        assert!(
            launch.shell_command.starts_with("cat "),
            "prompt file fed first"
        );

        // The prompt file exists and carries the NDJSON user message.
        let pf = prompt_file_dir().join(format!("{}.prompt.ndjson", launch.session_id));
        let content = std::fs::read_to_string(&pf).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(
            v["message"]["content"][0]["text"],
            "Investigate the flaky test"
        );

        // The launch event reached the bus, addressed with OUR session id
        // (other tests share the global bus — filter, don't assume ordering).
        let ev = find_launch_event(&mut rx, &launch.session_id)
            .expect("project_launch event for this session on the bus");
        assert_eq!(ev.payload["root_path"], "/tmp/some-project");
        assert_eq!(ev.payload["command"], launch.shell_command.as_str());
        assert_eq!(ev.payload["project_slug"], "proj");
    }

    #[tokio::test]
    async fn watched_session_without_prompt_has_no_prompt_pipeline() {
        let launch = launch_watched_session("/tmp/p", "label", "p", None, "reason")
            .await
            .unwrap();
        assert!(
            launch.shell_command.starts_with("'claude'"),
            "no prompt → no cat pipeline: {}",
            launch.shell_command
        );
    }

    // ── Dispatched-goal engine ─────────────────────────────────────────────

    fn git(dir: &Path, args: &[&str]) {
        let out = StdCommand::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Minimal repo with one commit; returns `(repo_path, head_sha)`.
    fn init_repo(tmp: &Path) -> (PathBuf, String) {
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let out = StdCommand::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (repo, sha)
    }

    fn engine() -> SupervisedCliEngine {
        SupervisedCliEngine {
            bin: SUPERVISED_CLI_DEFAULT_BIN.to_string(),
            project_slug: "proj".to_string(),
            persona_override: Some(("You are CC.".to_string(), "Claude Code".to_string())),
        }
    }

    fn task(working_dir: PathBuf, baseline: Option<String>, timeout: Duration) -> GoalTask {
        GoalTask {
            card_title: "Fix flaky test".to_string(),
            instructions: "Goal: fix it".to_string(),
            working_dir,
            baseline_commit: baseline,
            timeout,
            output_tx: None,
            parent_session_id: None,
            goal_id: None,
            budget_task_id: None,
            billing_class: Some(crate::config::agent_identity::WorkerBillingClass::Subscription),
            provider: Some("anthropic".to_string()),
            model: None,
            cli_identity: Some("claude".to_string()),
            project_id: None,
        }
    }

    #[tokio::test]
    async fn engine_requires_a_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let err = engine()
            .spawn(task(tmp.path().to_path_buf(), None, Duration::from_secs(5)))
            .await
            .err()
            .expect("no baseline must fail fast");
        assert!(err.contains("baseline"), "{err}");
    }

    #[tokio::test]
    async fn engine_launches_worktree_session_and_completes_via_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, sha) = init_repo(tmp.path());
        let mut rx = crate::events::subscribe();

        let work = engine()
            .spawn(task(
                repo.clone(),
                Some(sha.clone()),
                Duration::from_secs(30),
            ))
            .await
            .unwrap();
        assert!(work.run_id.starts_with(SUPERVISED_SESSION_PREFIX));
        assert!(
            matches!(work.kill, GoalKill::None),
            "no daemon-side process handle"
        );

        // Worktree created off the baseline, beside the repo.
        let worktree = repo
            .parent()
            .unwrap()
            .join(".permagent-goal-worktrees")
            .join(&work.run_id);
        assert!(worktree.is_dir(), "worktree at {}", worktree.display());

        // Launch event: rooted at the WORKTREE, command is the supervised
        // pipeline (prompt fed, gates on, push block env present).
        let ev =
            find_launch_event(&mut rx, &work.run_id).expect("supervised launch event on the bus");
        assert_eq!(ev.payload["root_path"], worktree.to_string_lossy().as_ref());
        let cmd = ev.payload["command"].as_str().unwrap();
        assert!(cmd.contains("--permission-prompt-tool stdio"), "{cmd}");
        assert!(!cmd.contains("--dangerously-skip-permissions"), "{cmd}");
        assert!(
            cmd.contains("GIT_CONFIG_KEY_0='remote.origin.pushurl'"),
            "{cmd}"
        );

        // The prompt file carries persona + instructions + commit-only brief.
        let pf = prompt_file_dir().join(format!("{}.prompt.ndjson", work.run_id));
        let content = std::fs::read_to_string(&pf).unwrap();
        assert!(content.contains("You are CC."));
        assert!(content.contains("Goal: fix it"));
        assert!(content.contains("do NOT push"));

        // Complete via the seam (S2's future call site) → Success + evidence.
        assert!(complete_supervised_session(
            &work.run_id,
            SupervisedOutcome::Completed {
                summary: "did the thing".to_string()
            }
        ));
        match work.join.await.unwrap() {
            GoalOutcome::Success(Some(ev)) => {
                assert_eq!(ev.baseline_commit, sha);
                assert_eq!(ev.worker_summary, "did the thing");
                assert_eq!(ev.files_changed, 0, "no commits made in this test");
            }
            other => panic!("expected Success with evidence, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn engine_times_out_and_parks_when_nobody_completes() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, sha) = init_repo(tmp.path());
        let work = engine()
            .spawn(task(repo, Some(sha), Duration::from_millis(50)))
            .await
            .unwrap();
        match work.join.await.unwrap() {
            GoalOutcome::TimedOut { .. } => {}
            other => panic!("expected TimedOut, got {:?}", other),
        }
        // The hook was cleaned up: completing now reports no listener.
        assert!(!complete_supervised_session(
            &work.run_id,
            SupervisedOutcome::Completed {
                summary: String::new()
            }
        ));
    }

    #[tokio::test]
    async fn engine_reports_session_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, sha) = init_repo(tmp.path());
        let work = engine()
            .spawn(task(repo, Some(sha), Duration::from_secs(30)))
            .await
            .unwrap();
        assert!(complete_supervised_session(
            &work.run_id,
            SupervisedOutcome::Failed {
                reason: "claude exited 1".to_string()
            }
        ));
        match work.join.await.unwrap() {
            GoalOutcome::Failed(reason) => assert_eq!(reason, "claude exited 1"),
            other => panic!("expected Failed, got {:?}", other),
        }
    }
}
