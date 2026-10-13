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
//!
//! Startup runs on a worker after the main window exists. The frontend boot
//! screen owns connection status and its deadline, independently of these waits.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::Emitter;
use tauri_plugin_shell::process::{CommandChild, CommandEvent, TerminatedPayload};
use tauri_plugin_shell::ShellExt;

/// Where the daemon serves. Mirrors goose-server's defaults (configuration.rs
/// `default_host`/`default_port`) and the launchd plist's explicit flags.
// The production target remains 127.0.0.1:3001; review targets are compile-time only.

/// How long to wait for an externally-managed daemon (unchanged behavior).
const EXTERNAL_WAIT_SECS: u64 = 10;

/// How long to wait for a sidecar we spawned. Longer than the external wait:
/// a fresh `~/.permagent` runs schema creation + workspace seeding on first
/// boot before the port binds.
const SPAWNED_WAIT_SECS: u64 = 20;

/// Poll interval for both waits.
const POLL_STEP_MS: u64 = 100;

/// Bound each token request well below the frontend's 10 s boot deadline.
const TOKEN_WAIT_MS: u64 = 1_000;
const TOKEN_NOT_READY: &str =
    "Daemon token not yet available; the background service may still be starting. Retry shortly.";

/// stderr lines kept in memory for the failure diagnostics.
const STDERR_TAIL_LINES: usize = 30;

/// Sidecar output log, under `~/.permagent/logs/`.
const SIDECAR_LOG_NAME: &str = "daemon-sidecar.log";

/// The spawned sidecar child, if any. Killed by [`stop_daemon`] on app exit.
/// (tauri-plugin-shell's own exit hook only kills JS-spawned children — a
/// Rust-side spawn is never in its tracked-children map, so kill-on-exit is
/// on us.)
static DAEMON_CHILD: Mutex<Option<CommandChild>> = Mutex::new(None);

/// Guards the INITIAL spawn so `start_daemon` is idempotent mid-startup. The
/// supervisor clears it before each deliberate restart — see
/// [`supervise_sidecar`].
static SPAWN_ATTEMPTED: AtomicBool = AtomicBool::new(false);

/// Set by [`stop_daemon`] so startup and supervision stop on app exit. The
/// child mutex serializes spawn/publication with shutdown taking the child.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// How often the supervisor checks whether the child is still alive.
const SUPERVISE_POLL_MS: u64 = 500;
/// Consecutive restarts before we stop trying. A daemon that cannot stay up is
/// a real fault; restarting it forever would hide that and burn CPU.
const MAX_RESTARTS: u32 = 5;
/// Restart backoff, indexed by consecutive failure count.
const RESTART_BACKOFF_SECS: [u64; MAX_RESTARTS as usize] = [1, 2, 5, 15, 30];
/// A daemon that stayed up this long is considered recovered, so the next
/// crash starts from a clean budget rather than the tail of an old one.
const HEALTHY_RUN_SECS: u64 = 120;

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

/// Queue startup after the window exists (RunEvent::Ready). All health waits,
/// sidecar setup and process spawning stay off the AppKit/event-loop thread.
pub fn start_daemon(app: &tauri::AppHandle) {
    let handle = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("permagent-daemon-startup".into())
        .spawn(move || {
            if let Err(error) = start_daemon_blocking(&handle) {
                surface_daemon_failure(&handle, &format!("backend startup failed: {error}"));
            }
        })
    {
        surface_daemon_failure(app, &format!("could not start backend worker: {error}"));
    }
}

/// Ensure a daemon is available: prefer an existing one, else launchd's, else
/// spawn the bundled sidecar. Never returns `Err` for a missing/unhealthy
/// daemon — the app must still open so the failure is visible and diagnosable.
fn start_daemon_blocking(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Ok(());
    }
    if crate::runtime_target::target().review {
        // Review daemons are launched explicitly with a fresh Paths root by the
        // coordinator. Never consult production launchd or spawn a bundled fallback.
        if !wait_for_daemon(EXTERNAL_WAIT_SECS) {
            surface_daemon_failure(
                app,
                "isolated review backend is unavailable; start its explicitly configured daemon",
            );
        }
        return Ok(());
    }
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
            PollOutcome::Healthy => {
                supervise_sidecar(app.clone(), spawned);
            }
            PollOutcome::Aborted => {
                if SHUTTING_DOWN.load(Ordering::SeqCst) {
                    return Ok(());
                }
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
                // frontend can retry. Its boot screen owns the visible state.
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

/// Keep the sidecar alive for as long as the app is.
///
/// Mode 3 spawned the daemon once and never looked again. If it died
/// mid-session the app simply reported a dead backend and the user had to
/// restart the whole thing — and because the status the sidebar renders is
/// really the per-session event stream, they were told "daemon offline" in
/// cases where nothing was wrong at all. Neither is something a user should
/// ever have to reason about: if the app is running, the backend is running.
///
/// Deliberately bounded. Unlimited restarts would turn a genuine crash loop
/// into a silent CPU burn and hide the fault; after [`MAX_RESTARTS`]
/// consecutive failures the supervisor stops and surfaces it. A child that
/// stays up for [`HEALTHY_RUN_SECS`] resets the budget, so an unrelated crash
/// weeks later is not charged against an old one.
fn supervise_sidecar(app: tauri::AppHandle, spawned: SpawnedDaemon) {
    std::thread::spawn(move || {
        let mut exited = spawned.exited;
        let mut consecutive: u32 = 0;

        loop {
            let started = std::time::Instant::now();
            // Wait for this child to die (or the app to quit).
            while !exited.load(Ordering::SeqCst) {
                if SHUTTING_DOWN.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(SUPERVISE_POLL_MS));
            }
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                return; // expected: the app is quitting
            }

            if started.elapsed().as_secs() >= HEALTHY_RUN_SECS {
                consecutive = 0;
            }
            if consecutive >= MAX_RESTARTS {
                eprintln!(
                    "[permagent-app] daemon exited {MAX_RESTARTS}x in a row; not restarting again"
                );
                surface_daemon_failure(
                    &app,
                    "backend keeps stopping (see ~/.permagent/logs/daemon-sidecar.log)",
                );
                return;
            }

            let wait = RESTART_BACKOFF_SECS[consecutive as usize];
            consecutive += 1;
            eprintln!(
                "[permagent-app] daemon exited unexpectedly; restarting in {wait}s (attempt {consecutive})"
            );
            std::thread::sleep(std::time::Duration::from_secs(wait));
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                return;
            }

            // Another daemon may have taken :3001 in the meantime (a user ran
            // one by hand, or launchd stepped in). That one wins — the module
            // contract is that a user-managed daemon is never fought over.
            if is_daemon_running() {
                eprintln!("[permagent-app] :3001 is served again; supervisor standing down");
                return;
            }

            SPAWN_ATTEMPTED.store(false, Ordering::SeqCst);
            match spawn_sidecar(&app) {
                Ok(next) => {
                    eprintln!("[permagent-app] daemon restarted");
                    exited = next.exited;
                }
                Err(e) => {
                    eprintln!("[permagent-app] daemon restart failed: {e}");
                    return;
                }
            }
        }
    });
}

/// Stop the sidecar we spawned, if any. Called from `RunEvent::Exit` in
/// `main.rs`. SIGTERM first so the daemon shuts down gracefully (WAL settle,
/// `daemon_stopped` event — the same signal `launchctl unload` sends), with a
/// bounded grace period and a SIGKILL backstop. No-op when the daemon is
/// launchd/user-managed (we never stored a child).
pub fn stop_daemon(_app: &tauri::AppHandle) {
    // Also cancel a review-mode wait; review still never owns a child.
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    if crate::runtime_target::target().review {
        return;
    }
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
    if crate::runtime_target::target().review {
        return Err("review mode never spawns a bundled daemon".into());
    }
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
    // Exit can now race initial startup as well as a supervised restart.
    // Hold the child slot through spawn + publication: either shutdown sees
    // this child and stops it, or its flag prevents any new spawn.
    let mut daemon_child = DAEMON_CHILD.lock().unwrap();
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err("app is shutting down".into());
    }
    let (mut rx, child) = app
        .shell()
        .sidecar("permagentd")?
        .args(["agent", "--host", "127.0.0.1", "--port", "3001"])
        .env("PERMAGENT_CONFIG", &config_path)
        .env("PERMAGENT_SPECTRAL_DB", &spectral_db)
        .spawn()?;

    let pid = child.pid();
    *daemon_child = Some(child);
    drop(daemon_child);
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
    // the startup worker polls for health.
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
                    // Don't SIGTERM a dead pid on app exit.
                    let mut daemon_child = DAEMON_CHILD.lock().unwrap();
                    if daemon_child.as_ref().map(|child| child.pid()) == Some(pid) {
                        *daemon_child = None;
                    }
                    // Publish the exit only after clearing this child's slot,
                    // so the supervisor cannot have its replacement erased.
                    exited_writer.store(true, Ordering::SeqCst);
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
        move || SHUTTING_DOWN.load(Ordering::SeqCst) || exited.load(Ordering::SeqCst),
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
    match poll_until(
        timeout_secs,
        is_daemon_running,
        || SHUTTING_DOWN.load(Ordering::SeqCst),
        POLL_STEP_MS,
    ) {
        PollOutcome::Healthy => {
            eprintln!("[permagent-app] daemon ready");
            true
        }
        PollOutcome::Aborted => false,
        PollOutcome::TimedOut => {
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
        &crate::runtime_target::target().address().parse().unwrap(),
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

/// Preserve diagnostics without competing with the frontend boot/retry UI.
/// The event carries a detail string for frontend listeners; it is best-effort
/// (a webview may not be listening yet), so stderr remains the fallback report.
fn surface_daemon_failure(app: &tauri::AppHandle, detail: &str) {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return;
    }
    eprintln!("[permagent-app] ERROR: {detail}");
    if let Err(error) = app.emit("daemon-failure", detail) {
        eprintln!("[permagent-app] could not emit backend failure: {error}");
    }
}

/// Read the daemon Bearer token from ~/.permagent/secrets/daemon_token.json.
/// A fresh sidecar may still be creating it now that the window opens first.
/// Wait briefly on the blocking pool, never on the UI/async executor thread.
#[tauri::command]
pub async fn get_daemon_token() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        wait_for_daemon_token(
            &crate::runtime_target::target().token_path(),
            Duration::from_millis(TOKEN_WAIT_MS),
        )
    })
    .await
    .map_err(|error| format!("Daemon token lookup failed: {error}"))?
}

/// Immediate read for native callers such as the supervised-session tee.
pub fn read_daemon_token() -> Result<String, String> {
    wait_for_daemon_token(
        &crate::runtime_target::target().token_path(),
        Duration::ZERO,
    )
}

fn wait_for_daemon_token(token_path: &Path, timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match std::fs::read_to_string(token_path) {
            Ok(content) => return parse_daemon_token(&content),
            // The daemon publishes this file atomically. Only absence is a
            // startup race; permission/parse errors keep their diagnostics.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if SHUTTING_DOWN.load(Ordering::SeqCst) {
                    return Err("Daemon token unavailable: app is shutting down".into());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    // Keep Result<String, String> for existing IPC callers;
                    // the frontend clears failed lookups and retries later.
                    return Err(TOKEN_NOT_READY.into());
                }
                std::thread::sleep(remaining.min(Duration::from_millis(POLL_STEP_MS)));
            }
            Err(error) => return Err(format!("Failed to read daemon token: {error}")),
        }
    }
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
    fn parse_daemon_token_extracts_token() {
        assert_eq!(
            parse_daemon_token(r#"{"token":"abc123"}"#).unwrap(),
            "abc123"
        );
        assert!(parse_daemon_token(r#"{"nope":true}"#)
            .unwrap_err()
            .contains("missing 'token'"));
        assert!(parse_daemon_token("not json").is_err());
    }

    fn token_test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("permagent-token-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn token_available_without_waiting() {
        let dir = token_test_dir();
        let path = dir.join("daemon_token.json");
        std::fs::write(&path, r#"{"token":"already-ready"}"#).unwrap();

        assert_eq!(
            wait_for_daemon_token(&path, Duration::ZERO).unwrap(),
            "already-ready"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn token_created_after_request_is_picked_up() {
        let dir = token_test_dir();
        let path = dir.join("daemon_token.json");
        // The shell is already accepting IPC while the daemon initializes.
        assert_eq!(
            wait_for_daemon_token(&path, Duration::ZERO).unwrap_err(),
            TOKEN_NOT_READY
        );
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let staged = writer_path.with_extension("tmp");
            std::fs::write(&staged, r#"{"token":"fresh-sidecar"}"#).unwrap();
            std::fs::rename(staged, writer_path).unwrap();
        });

        let result = wait_for_daemon_token(&path, Duration::from_secs(2));
        writer.join().unwrap();
        assert_eq!(result.unwrap(), "fresh-sidecar");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_token_times_out_with_retryable_message_and_can_recover() {
        let dir = token_test_dir();
        let path = dir.join("daemon_token.json");
        let started = Instant::now();
        let timeout = Duration::from_millis(20);
        assert_eq!(
            wait_for_daemon_token(&path, timeout).unwrap_err(),
            TOKEN_NOT_READY
        );
        assert!(started.elapsed() >= timeout);

        // A timed-out request must not cache a permanent token failure.
        std::fs::write(&path, r#"{"token":"late-daemon"}"#).unwrap();
        assert_eq!(
            wait_for_daemon_token(&path, Duration::ZERO).unwrap(),
            "late-daemon"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_token_and_io_errors_are_not_reported_as_pending() {
        let dir = token_test_dir();
        let path = dir.join("daemon_token.json");
        std::fs::write(&path, "invalid JSON").unwrap();
        assert!(wait_for_daemon_token(&path, Duration::ZERO)
            .unwrap_err()
            .contains("Failed to parse daemon token"));

        // A directory is an I/O error, distinct from a missing startup token.
        assert!(wait_for_daemon_token(&dir, Duration::ZERO)
            .unwrap_err()
            .contains("Failed to read daemon token"));
        std::fs::remove_dir_all(dir).unwrap();
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
