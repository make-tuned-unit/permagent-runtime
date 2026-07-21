//! permagentd lifecycle for the Tauri shell.
//!
//! Three modes, decided at startup in this order:
//!
//! 1. **Already listening on :3001** — a user-managed daemon (launchd, or a
//!    dev running `permagentd agent` by hand) always wins. We connect to it
//!    and never spawn.
//! 2. **`~/Library/LaunchAgents/ai.permagent.daemon.plist` exists** — launchd
//!    owns the daemon lifecycle (dev machines that ran `permagent setup` /
//!    `permagent daemon start`). The port may be momentarily free while
//!    launchd (re)starts it; spawning here would race launchd for port 3001
//!    and recreate the KeepAlive crash loop that got the original sidecar
//!    spawn removed (cbded9ac: both spawners fought for the port, the loser
//!    was respawned every ~10 s). Wait-only, exactly as before.
//! 3. **Fresh Mac** (no daemon, no plist — a stranger's DMG install): spawn
//!    the bundled `permagentd` sidecar as a child process with the same
//!    args/env the launchd plist would use, and wait for it to become
//!    healthy. The child dies with the app: `RunEvent::Exit` in `main.rs`
//!    calls [`stop_daemon`] (SIGTERM, then SIGKILL as a backstop). Note the
//!    implication: daemon-side scheduled jobs stop when the app closes —
//!    a persistent LaunchAgent install is the long-term answer (see PR).
//!
//! The daemon self-initializes an empty `~/.permagent` (schema, workspace
//! seeding, `secrets/daemon_token.json`), so the spawn needs no first-run
//! provisioning beyond the defaults. Sidecar stdout/stderr is captured to
//! `~/.permagent/logs/daemon-sidecar.log` (previous run rotated to `.old`)
//! and the stderr tail is echoed to the app's stderr on failure so a dead
//! first launch is diagnosable.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri_plugin_shell::process::{CommandChild, CommandEvent, TerminatedPayload};
use tauri_plugin_shell::ShellExt;

/// Where the daemon serves. Mirrors goose-server's defaults (configuration.rs
/// `default_host`/`default_port`) and the launchd plist's explicit flags.
const DAEMON_ADDR: &str = "127.0.0.1:3001";

/// How long to wait for an externally-managed daemon (unchanged behavior).
const EXTERNAL_WAIT_SECS: u64 = 10;

/// How long to wait for a sidecar we spawned. Longer than the external wait:
/// a fresh `~/.permagent` runs schema creation + workspace seeding on first
/// boot before the port binds.
const SPAWNED_WAIT_SECS: u64 = 20;

/// Poll interval for both waits.
const POLL_STEP_MS: u64 = 100;

/// stderr lines kept in memory for the failure diagnostics.
const STDERR_TAIL_LINES: usize = 30;

/// Sidecar output log, under `~/.permagent/logs/`.
const SIDECAR_LOG_NAME: &str = "daemon-sidecar.log";

/// The spawned sidecar child, if any. Killed by [`stop_daemon`] on app exit.
/// (tauri-plugin-shell's own exit hook only kills JS-spawned children — a
/// Rust-side spawn is never in its tracked-children map, so kill-on-exit is
/// on us.)
static DAEMON_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// One spawn attempt per app process, ever — makes `start_daemon` idempotent
/// even if a future caller re-invokes it mid-startup.
static SPAWN_ATTEMPTED: AtomicBool = AtomicBool::new(false);

/// Handles to a spawned sidecar for health monitoring.
struct SpawnedDaemon {
    /// Set by the output-reader task when the child terminates.
    exited: Arc<AtomicBool>,
    /// Ring buffer of the last [`STDERR_TAIL_LINES`] stderr lines.
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// Where the full output is being written.
    log_path: PathBuf,
}

/// Outcome of polling for daemon health.
#[derive(Debug, PartialEq, Eq)]
enum PollOutcome {
    Healthy,
    Aborted,
    TimedOut,
}

/// Ensure a daemon is available: prefer an existing one, else launchd's, else
/// spawn the bundled sidecar. Never returns `Err` for a missing/unhealthy
/// daemon — the app must still open so the failure is visible and diagnosable.
pub fn start_daemon(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if is_daemon_running() {
        eprintln!("[permagent-app] daemon detected on :3001");
        return Ok(());
    }

    if launchd_plist_path().exists() {
        // Mode 2: launchd owns the daemon. It may be mid-(re)start — wait,
        // never spawn (see module docs for the crash-loop history).
        eprintln!("[permagent-app] launchd plist present — waiting for the launchd-managed daemon");
        if wait_for_daemon(EXTERNAL_WAIT_SECS) {
            return Ok(());
        }
        surface_daemon_failure(
            app,
            "backend not running — try `permagent daemon start` (logs: ~/.permagent/logs/)",
        );
        return Ok(());
    }

    // Mode 3: fresh machine — spawn the bundled sidecar.
    match spawn_sidecar(app) {
        Ok(spawned) => match wait_for_spawned(&spawned) {
            PollOutcome::Healthy => {}
            PollOutcome::Aborted => {
                eprintln!(
                    "[permagent-app] ERROR: spawned daemon exited before becoming healthy; stderr tail:"
                );
                dump_stderr_tail(&spawned);
                surface_daemon_failure(
                    app,
                    "backend crashed on startup (see ~/.permagent/logs/daemon-sidecar.log)",
                );
            }
            PollOutcome::TimedOut => {
                eprintln!(
                    "[permagent-app] WARNING: spawned daemon not healthy after {SPAWNED_WAIT_SECS}s; stderr tail:"
                );
                dump_stderr_tail(&spawned);
                // Leave the child running — first boots can be slow and the
                // frontend keeps retrying; the title is corrected too late in
                // that case, but honesty beats a silent hang.
                surface_daemon_failure(
                    app,
                    "backend slow to start (see ~/.permagent/logs/daemon-sidecar.log)",
                );
            }
        },
        Err(e) => {
            // Dev runs (non-bundled): the sidecar binary doesn't exist next
            // to the app binary. Degrade to the historical wait-only
            // behavior — never a crash.
            eprintln!(
                "[permagent-app] sidecar unavailable ({e}); waiting for an externally-run daemon"
            );
            if !wait_for_daemon(EXTERNAL_WAIT_SECS) {
                surface_daemon_failure(app, "backend not running (start permagentd manually)");
            }
        }
    }
    Ok(())
}

/// Stop the sidecar we spawned, if any. Called from `RunEvent::Exit` in
/// `main.rs`. SIGTERM first so the daemon shuts down gracefully (WAL settle,
/// `daemon_stopped` event — the same signal `launchctl unload` sends), with a
/// bounded grace period and a SIGKILL backstop. No-op when the daemon is
/// launchd/user-managed (we never stored a child).
pub fn stop_daemon(_app: &tauri::AppHandle) {
    let child = DAEMON_CHILD.lock().unwrap().take();
    let Some(child) = child else {
        return;
    };
    let pid = child.pid() as i32;
    eprintln!("[permagent-app] stopping spawned daemon (pid {pid})");
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    // Give the graceful shutdown up to 2 s (it normally completes in well
    // under one — by exit time our webviews' SSE connections are gone, so
    // the axum drain is immediate).
    for _ in 0..20 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            eprintln!("[permagent-app] spawned daemon exited cleanly");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("[permagent-app] spawned daemon still up after SIGTERM grace; killing");
    let _ = child.kill();
}

/// Spawn the bundled `permagentd` sidecar with launchd-plist-parity args/env
/// and start a reader task that mirrors its output to the sidecar log.
fn spawn_sidecar(app: &tauri::AppHandle) -> Result<SpawnedDaemon, Box<dyn std::error::Error>> {
    if SPAWN_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return Err("sidecar spawn already attempted this run".into());
    }

    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let logs_dir = PathBuf::from(&home).join(".permagent").join("logs");
    std::fs::create_dir_all(&logs_dir)?;
    let log_path = logs_dir.join(SIDECAR_LOG_NAME);
    rotate_log(&log_path);
    let mut log_file = std::fs::File::create(&log_path)?;

    // Args + env mirror the launchd plist `permagent daemon start` generates
    // (goose-cli daemon.rs `generate_plist`): explicit host/port beat any
    // stray HOST/PORT env, and the two PERMAGENT_* vars keep observability
    // identical between launchd-managed and app-spawned daemons. The daemon
    // resolves its data dir (`~/.permagent`) itself and self-initializes an
    // empty one — no other env is needed.
    let config_path = format!("{home}/.permagent/config.yaml");
    let spectral_db = format!("{home}/.permagent/spectral/permagent.db");
    let (mut rx, child) = app
        .shell()
        .sidecar("permagentd")?
        .args(["agent", "--host", "127.0.0.1", "--port", "3001"])
        .env("PERMAGENT_CONFIG", &config_path)
        .env("PERMAGENT_SPECTRAL_DB", &spectral_db)
        .spawn()?;

    let pid = child.pid();
    *DAEMON_CHILD.lock().unwrap() = Some(child);
    eprintln!(
        "[permagent-app] spawned bundled permagentd (pid {pid}); log: {}",
        log_path.display()
    );
    let _ = writeln!(
        log_file,
        "--- permagentd sidecar spawned by Permagent.app (pid {pid}) ---"
    );

    let exited = Arc::new(AtomicBool::new(false));
    let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
    let exited_writer = exited.clone();
    let tail_writer = stderr_tail.clone();

    // Reader task: runs on the Tokio pool, so it keeps draining while
    // `start_daemon` blocks the main thread polling for health.
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let line = String::from_utf8_lossy(&line);
                    let _ = writeln!(log_file, "[out] {}", line.trim_end());
                }
                CommandEvent::Stderr(line) => {
                    let line = String::from_utf8_lossy(&line).trim_end().to_string();
                    let _ = writeln!(log_file, "[err] {line}");
                    push_tail(&tail_writer, line);
                }
                CommandEvent::Error(e) => {
                    let _ = writeln!(log_file, "[proc-error] {e}");
                    eprintln!("[permagentd] process error: {e}");
                }
                CommandEvent::Terminated(payload) => {
                    let status = format_termination(&payload);
                    let _ = writeln!(log_file, "--- permagentd {status} ---");
                    eprintln!("[permagent-app] spawned daemon {status}");
                    exited_writer.store(true, Ordering::SeqCst);
                    // Don't SIGTERM a dead pid on app exit.
                    *DAEMON_CHILD.lock().unwrap() = None;
                    break;
                }
                _ => {}
            }
        }
    });

    Ok(SpawnedDaemon {
        exited,
        stderr_tail,
        log_path,
    })
}

/// Poll until the spawned daemon is healthy, it dies, or we time out.
fn wait_for_spawned(spawned: &SpawnedDaemon) -> PollOutcome {
    let exited = spawned.exited.clone();
    let outcome = poll_until(
        SPAWNED_WAIT_SECS,
        is_daemon_running,
        move || exited.load(Ordering::SeqCst),
        POLL_STEP_MS,
    );
    if outcome == PollOutcome::Healthy {
        eprintln!("[permagent-app] spawned daemon is healthy on :3001");
    }
    outcome
}

/// Poll port 3001 until an externally-managed daemon is ready. Returns
/// whether it became healthy (the historical `wait_for_daemon`, with the
/// result now reported instead of swallowed).
fn wait_for_daemon(timeout_secs: u64) -> bool {
    match poll_until(timeout_secs, is_daemon_running, || false, POLL_STEP_MS) {
        PollOutcome::Healthy => {
            eprintln!("[permagent-app] daemon ready");
            true
        }
        _ => {
            eprintln!(
                "[permagent-app] WARNING: daemon did not become ready within {timeout_secs}s"
            );
            false
        }
    }
}

/// Generic bounded poll: `Healthy` as soon as `healthy()`, `Aborted` as soon
/// as `aborted()`, `TimedOut` after `timeout_secs`.
fn poll_until(
    timeout_secs: u64,
    healthy: impl Fn() -> bool,
    aborted: impl Fn() -> bool,
    step_ms: u64,
) -> PollOutcome {
    let steps = (timeout_secs * 1000) / step_ms.max(1);
    for _ in 0..steps {
        if healthy() {
            return PollOutcome::Healthy;
        }
        if aborted() {
            return PollOutcome::Aborted;
        }
        std::thread::sleep(std::time::Duration::from_millis(step_ms));
    }
    if healthy() {
        PollOutcome::Healthy
    } else if aborted() {
        PollOutcome::Aborted
    } else {
        PollOutcome::TimedOut
    }
}

/// Quick check: is something listening on the daemon port? TCP accept only
/// starts after the daemon has fully initialized (bind is the last step of
/// startup), so this is a faithful health probe.
fn is_daemon_running() -> bool {
    std::net::TcpStream::connect_timeout(
        &DAEMON_ADDR.parse().unwrap(),
        std::time::Duration::from_millis(50),
    )
    .is_ok()
}

/// The launchd LaunchAgent written by `permagent setup` / `permagent daemon
/// start`. Its presence means launchd owns the daemon lifecycle.
fn launchd_plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/LaunchAgents/ai.permagent.daemon.plist")
}

/// Keep exactly one previous run's log: rename the current log to `.old`
/// (best-effort) so each app run starts a fresh, bounded file.
fn rotate_log(path: &std::path::Path) {
    if path.exists() {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
}

/// Append to the bounded stderr ring buffer.
fn push_tail(tail: &Mutex<VecDeque<String>>, line: String) {
    let mut tail = tail.lock().unwrap();
    if tail.len() >= STDERR_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

/// Human-readable exit description for the log + stderr.
fn format_termination(payload: &TerminatedPayload) -> String {
    match (payload.code, payload.signal) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(sig)) => format!("terminated by signal {sig}"),
        (None, None) => "exited (unknown status)".to_string(),
    }
}

/// Echo the captured stderr tail so the failure is visible in Console.app
/// next to the pointer to the full log.
fn dump_stderr_tail(spawned: &SpawnedDaemon) {
    for line in spawned.stderr_tail.lock().unwrap().iter() {
        eprintln!("[permagentd:err] {line}");
    }
    eprintln!(
        "[permagent-app] full sidecar log: {}",
        spawned.log_path.display()
    );
}

/// Honest failure surface: the shell drives no in-app UI, so put the state
/// where the user will see it — the window title — alongside the stderr
/// diagnostics. The SPA keeps retrying and takes over normally if the daemon
/// shows up late.
fn surface_daemon_failure(app: &tauri::AppHandle, detail: &str) {
    eprintln!("[permagent-app] ERROR: {detail}");
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&format!("Permagent — {detail}"));
    }
}

/// Read the daemon Bearer token from ~/.permagent/secrets/daemon_token.json.
/// Returns the token string if available, or an error message.
#[tauri::command]
pub async fn get_daemon_token() -> Result<String, String> {
    read_daemon_token()
}

/// Synchronous core of [`get_daemon_token`] — also used by the supervised-
/// session tee (`terminal.rs`), which runs on a plain thread.
pub fn read_daemon_token() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let token_path = std::path::PathBuf::from(home)
        .join(".permagent")
        .join("secrets")
        .join("daemon_token.json");

    let content = std::fs::read_to_string(&token_path)
        .map_err(|e| format!("Failed to read daemon token: {}", e))?;

    parse_daemon_token(&content)
}

/// Extract the `token` field from daemon_token.json content.
pub fn parse_daemon_token(content: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Failed to parse daemon token: {}", e))?;

    parsed
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "daemon_token.json missing 'token' field".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_until_healthy_immediately() {
        assert_eq!(poll_until(1, || true, || false, 1), PollOutcome::Healthy);
    }

    #[test]
    fn poll_until_aborts_on_child_death() {
        assert_eq!(poll_until(1, || false, || true, 1), PollOutcome::Aborted);
    }

    #[test]
    fn poll_until_health_wins_over_abort() {
        // A daemon that binds the port and then our stale exited flag must
        // report healthy — health is checked first.
        assert_eq!(poll_until(1, || true, || true, 1), PollOutcome::Healthy);
    }

    #[test]
    fn poll_until_times_out() {
        assert_eq!(poll_until(0, || false, || false, 1), PollOutcome::TimedOut);
    }

    #[test]
    fn push_tail_caps_at_limit() {
        let tail = Mutex::new(VecDeque::new());
        for i in 0..(STDERR_TAIL_LINES + 10) {
            push_tail(&tail, format!("line {i}"));
        }
        let tail = tail.lock().unwrap();
        assert_eq!(tail.len(), STDERR_TAIL_LINES);
        assert_eq!(tail.front().unwrap(), "line 10");
        assert_eq!(
            tail.back().unwrap(),
            &format!("line {}", STDERR_TAIL_LINES + 9)
        );
    }

    #[test]
    fn rotate_log_moves_previous_run_aside() {
        let dir = std::env::temp_dir().join(format!("permagent-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join(SIDECAR_LOG_NAME);
        std::fs::write(&log, "previous run").unwrap();

        rotate_log(&log);

        assert!(!log.exists());
        let old = log.with_extension("log.old");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "previous run");
        // Rotating again over an existing .old replaces it (bounded at one).
        std::fs::write(&log, "next run").unwrap();
        rotate_log(&log);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "next run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotate_log_noop_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("permagent-rotate-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        rotate_log(&dir.join(SIDECAR_LOG_NAME)); // must not panic
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_termination_variants() {
        let exit = TerminatedPayload {
            code: Some(1),
            signal: None,
        };
        assert_eq!(format_termination(&exit), "exited with code 1");
        let sig = TerminatedPayload {
            code: None,
            signal: Some(15),
        };
        assert_eq!(format_termination(&sig), "terminated by signal 15");
        let unknown = TerminatedPayload {
            code: None,
            signal: None,
        };
        assert_eq!(format_termination(&unknown), "exited (unknown status)");
    }

    #[test]
    fn plist_path_is_the_cli_generated_launchagent() {
        // Must match goose-cli's `plist_path()` (daemon.rs) and setup.rs —
        // the no-spawn guard hinges on this exact filename.
        let p = launchd_plist_path();
        assert!(p
            .to_string_lossy()
            .ends_with("Library/LaunchAgents/ai.permagent.daemon.plist"));
    }
}
