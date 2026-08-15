//! Local crash capture (#299 — local half).
//!
//! A global panic hook writes a human-readable crash report (message, location,
//! thread, backtrace) to the state dir whenever the process panics. The reports
//! are surfaced to support **only** through the diagnostic bundle, and **only
//! when the user has consented** — the gate is off by default, reusing the
//! existing PostHog telemetry opt-in (`crate::posthog::is_telemetry_enabled`).
//!
//! Local capture is always on (it is just a log file on the user's own disk).
//! The consent gate controls *sharing* (inclusion in the bundle), not capture.
//!
//! Out of scope (→ #327): uploading crash reports anywhere. This lane captures
//! locally and bundles-on-consent; there is no network path.

use std::backtrace::Backtrace;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::paths::Paths;
use crate::config::Config;

const CRASH_SUBDIR: &str = "crashes";
/// Subdirectory for user-triggered redacted exports (#327 MVP).
const CRASH_EXPORT_SUBDIR: &str = "crash-exports";
/// Keep only the most recent N crash reports on disk.
const MAX_CRASH_FILES: usize = 20;

/// Config key for crash-report sharing consent — **separate** from the
/// product-analytics telemetry opt-in (`GOOSE_TELEMETRY_ENABLED`) as of the
/// #327 split. Two distinct consent asks: "help fix crashes" vs. "share usage
/// analytics." Default OFF (explicit opt-in). Because `Config::get_param`
/// checks the uppercased env var first, `CRASH_REPORTS_CONSENT=false` also works
/// as a kill-switch.
pub const CRASH_REPORTS_CONSENT_KEY: &str = "crash_reports_consent";

/// Panic circuit-breaker defaults (durability F1). A single panic in a spawned
/// task unwinds only that task — the process limps on "half-dead", invisible to
/// launchd (which restarts only on process *exit*). To fail-fast-recover-clean
/// instead, we count panics in a sliding window; once a cluster forms (the
/// systemic "something is spiralling" signal), we force a clean `exit(1)` so
/// launchd relaunches a fresh process. A single isolated panic does NOT exit —
/// that would over-react to a locally-recoverable task panic — but a burst does.
/// Tune via env for ops; the launchd `ThrottleInterval` provides the across-
/// restart backoff so a crash-loop can't tight-loop.
const PANIC_BREAKER_MAX_DEFAULT: usize = 3;
const PANIC_BREAKER_WINDOW_SECS_DEFAULT: u64 = 60;

/// Sliding window of recent panic unix-timestamps (seconds). Guarded by a Mutex;
/// poison is recovered (a poisoned lock just means a prior panic held it).
static PANIC_TIMES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// Directory where crash reports are written (`<state>/crashes`).
pub fn crash_dir() -> PathBuf {
    Paths::in_state_dir(CRASH_SUBDIR)
}

/// Whether crash reports may be **shared** (bundled today; uploaded if an
/// ambient path is ever built).
///
/// Off by default, explicit opt-in. As of #327 this is a **dedicated** consent
/// key ([`CRASH_REPORTS_CONSENT_KEY`]) — no longer piggy-backed on the product-
/// analytics telemetry opt-in — so a user can help fix crashes without sharing
/// usage analytics, or vice versa. Reading config directly (not the `telemetry`
/// feature) keeps the gate available even in builds compiled without telemetry.
///
/// Note: this gates *ambient sharing*. The user-triggered redacted export
/// ([`export_redacted_bundle`]) is not gated on it — the user is the actor and
/// the export never leaves the machine on its own.
pub fn crash_reports_consented() -> bool {
    Config::global()
        .get_param::<bool>(CRASH_REPORTS_CONSENT_KEY)
        .unwrap_or(false)
}

/// Self-knowledge descriptor for the daemon's durability supervision. A
/// `Guard`, not a tool: Henry does not *call* this — the daemon runs it
/// *around* him. Co-located here with the panic circuit-breaker (the F1 core);
/// the same capability also covers the scheduler startup reconciliation, the
/// WAL-checkpoint timer, and the external wedge watchdog + metrics probe.
///
/// The prose below is deliberately conditional on two counts. The watchdog is a
/// separate LaunchAgent that only `permagent setup` installs, so a desktop
/// install that never ran setup has the circuit-breaker but no wedge cover. And
/// `wal_checkpoint(TRUNCATE)` returns `busy=1` against a pinned reader, so the
/// WAL is retried rather than guaranteed bounded. Static — a live/queryable
/// version awaits the `/api/health/durability` endpoint (follow-up). Aggregated
/// by `crate::agents::self_knowledge::GUARD_DESCRIPTORS`.
pub const DURABILITY_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "durability_supervision",
        display_name: "Durability supervision",
        category: crate::agents::self_knowledge::FeatureCategory::Guard,
        what_it_does:
            "Keeps your daemon healthy for weeks unattended: a panic circuit-breaker forces a clean restart instead of limping half-dead, and — once `permagent setup` has registered you under launchd — an external watchdog restarts you if you stop answering while your process is still alive; the databases' write-ahead logs are checkpointed on a timer, retried on the next tick when a reader pins the log, and scheduled work is reconciled after every restart",
        why_it_matters:
            "It is why the user can leave you running and reach you days later and you just work — a crash restarts you cleanly, a wedge is caught by the watchdog once it is installed, the write-ahead logs are checkpointed rather than left to grow untended, and scheduled work resumes",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// A captured crash, rendered to a stable text report.
#[derive(Debug, Clone)]
pub struct CrashReport {
    pub timestamp: String,
    pub thread: String,
    pub location: String,
    pub message: String,
    pub backtrace: String,
}

impl CrashReport {
    pub fn to_text(&self) -> String {
        format!(
            "Permagent crash report\n\
             ======================\n\
             Timestamp: {}\n\
             Thread:    {}\n\
             Location:  {}\n\
             Message:   {}\n\n\
             Backtrace:\n{}\n",
            self.timestamp, self.thread, self.location, self.message, self.backtrace
        )
    }
}

/// Build a sortable, unique-ish filename for a report.
fn report_filename(timestamp: &str) -> String {
    // Sanitize the timestamp for a filename and tack on the pid to avoid
    // collisions when two threads crash in the same instant.
    let safe: String = timestamp
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("crash-{}-{}.log", safe, std::process::id())
}

/// Write a crash report to `dir`, creating it if needed, then prune to the most
/// recent [`MAX_CRASH_FILES`]. Returns the path written.
pub fn record_crash(dir: &Path, report: &CrashReport) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(report_filename(&report.timestamp));
    std::fs::write(&path, report.to_text())?;
    prune(dir, MAX_CRASH_FILES);
    Ok(path)
}

/// Keep only the `max` most recent `crash-*.log` files in `dir`.
fn prune(dir: &Path, max: usize) {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log"))
            })
            .collect(),
        Err(_) => return,
    };
    if files.len() <= max {
        return;
    }
    // Oldest first by mtime; remove the excess from the front.
    files.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for p in files.iter().take(files.len() - max) {
        let _ = std::fs::remove_file(p);
    }
}

/// Read crash reports from `dir`, newest first, capped at [`MAX_CRASH_FILES`].
/// Returns `(filename, bytes)` pairs for bundling.
pub fn collect_crash_logs(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log"))
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    // Newest first.
    files.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    files.reverse();
    files
        .into_iter()
        .take(MAX_CRASH_FILES)
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let bytes = std::fs::read(&p).ok()?;
            Some((name, bytes))
        })
        .collect()
}

/// Directory where user-triggered redacted exports are written
/// (`<state>/crash-exports`).
pub fn export_dir() -> PathBuf {
    Paths::in_state_dir(CRASH_EXPORT_SUBDIR)
}

/// The result of a user-triggered redacted crash-report export (#327 MVP).
#[derive(Debug, Clone)]
pub struct RedactedCrashExport {
    /// Absolute path of the redacted bundle written to local disk.
    pub path: PathBuf,
    /// How many captured crash logs were included.
    pub report_count: usize,
    /// The full redacted bundle text (identical to the file contents) so the UI
    /// can show the user *exactly* what would be shared before they attach it.
    pub content: String,
}

/// Assemble the redacted bundle text from `(filename, bytes)` crash logs. Pure
/// (no I/O) so it is directly unit-testable: each log is scrubbed through the
/// shared redactor ([`crate::privacy::redact`]) and framed with a header. Empty
/// input yields an honest "no crash reports" bundle so the export action always
/// produces a real, inspectable artifact.
pub fn build_redacted_bundle(logs: &[(String, Vec<u8>)]) -> String {
    let mut out = String::new();
    out.push_str("Permagent redacted crash-report export\n");
    out.push_str("======================================\n");
    out.push_str(
        "All home paths, keys, tokens, emails, and UUIDs below have been redacted.\n\
         This file is LOCAL — nothing was uploaded. Attach it yourself if you\n\
         choose to share it.\n\n",
    );
    if logs.is_empty() {
        out.push_str("(No crash reports have been captured on this machine.)\n");
        return out;
    }
    out.push_str(&format!("{} crash report(s) included.\n\n", logs.len()));
    for (name, bytes) in logs {
        let text = String::from_utf8_lossy(bytes);
        out.push_str(&format!("===== {name} =====\n"));
        out.push_str(&crate::privacy::redact(&text));
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Produce a **redacted** crash-report bundle on local disk and return its path
/// and content (#327 MVP). Reads the captured crash logs from [`crash_dir`],
/// scrubs every one through the shared redactor, writes the concatenated bundle
/// to `<state>/crash-exports/crash-report-<ts>.txt`, and returns it for preview.
///
/// **No network path.** This is the user-triggered, sovereign-safe reporting
/// surface: the user is the actor, the bytes stay on their disk, and they decide
/// whether to attach the file to a support channel. It is intentionally *not*
/// gated on [`crash_reports_consented`] (that gate governs ambient sharing) and
/// is never suppressed by sovereign mode (a local file write is not egress).
pub fn export_redacted_bundle() -> std::io::Result<RedactedCrashExport> {
    export_redacted_bundle_in(&crash_dir(), &export_dir())
}

/// [`export_redacted_bundle`] against explicit source/destination dirs. Split
/// out so tests can drive the whole write path against temp dirs without
/// mutating the process-global `PERMAGENT_PATH_ROOT` (which would race other
/// tests reading the state dir).
fn export_redacted_bundle_in(
    crash_dir: &Path,
    export_dir: &Path,
) -> std::io::Result<RedactedCrashExport> {
    let logs = collect_crash_logs(crash_dir);
    let content = build_redacted_bundle(&logs);

    std::fs::create_dir_all(export_dir)?;
    let ts = chrono::Utc::now().to_rfc3339();
    let safe_ts: String = ts
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = export_dir.join(format!("crash-report-{safe_ts}.txt"));
    std::fs::write(&path, content.as_bytes())?;

    Ok(RedactedCrashExport {
        path,
        report_count: logs.len(),
        content,
    })
}

/// Render a panic into a [`CrashReport`].
fn report_from_panic(info: &std::panic::PanicHookInfo<'_>, backtrace: &Backtrace) -> CrashReport {
    let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());
    let thread = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    CrashReport {
        timestamp: chrono::Utc::now().to_rfc3339(),
        thread,
        location,
        message,
        backtrace: format!("{backtrace}"),
    }
}

static INSTALL_ONCE: Once = Once::new();

/// Read the circuit-breaker thresholds (max panics, window seconds), env-first
/// with the compile-time defaults as the floor. Explicit config over hidden
/// magic; env keeps it out of the async config path (this runs inside the panic
/// hook, during unwind).
fn breaker_config() -> (usize, u64) {
    let max = std::env::var("PERMAGENT_PANIC_BREAKER_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(PANIC_BREAKER_MAX_DEFAULT);
    let window = std::env::var("PERMAGENT_PANIC_BREAKER_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(PANIC_BREAKER_WINDOW_SECS_DEFAULT);
    (max, window)
}

/// Record a panic at `now_secs` and return `true` if the number of panics within
/// the trailing `window_secs` has reached `max` (the crash-loop threshold).
/// Pure and deterministic — the caller owns the `exit` decision so this is unit-
/// testable without terminating the test process.
fn record_panic_and_check(now_secs: u64, max: usize, window_secs: u64) -> bool {
    let mut times = PANIC_TIMES.lock().unwrap_or_else(|e| e.into_inner());
    check_window(&mut times, now_secs, max, window_secs)
}

/// Pure sliding-window check: prune timestamps older than `window_secs`, record
/// `now_secs`, and report whether the count has reached `max`. Split out so it is
/// testable against a local buffer (no shared global state).
fn check_window(times: &mut Vec<u64>, now_secs: u64, max: usize, window_secs: u64) -> bool {
    times.retain(|&t| now_secs.saturating_sub(t) < window_secs);
    times.push(now_secs);
    times.len() >= max
}

/// Install the global panic hook (idempotent). Writes crash reports to
/// [`crash_dir`], chains to the previous hook so stderr output is unchanged, and
/// arms the panic circuit-breaker (a panic cluster forces a clean `exit(1)` for
/// launchd to relaunch). Production entrypoint.
pub fn install_panic_hook() {
    install_panic_hook_inner(crash_dir(), true);
}

/// Install the panic hook targeting a specific directory, WITHOUT the exit-on-
/// cluster circuit-breaker. Used by tests and the evidence harness so a panicking
/// test can't tear down the test process; production calls [`install_panic_hook`].
pub fn install_panic_hook_to(dir: PathBuf) {
    install_panic_hook_inner(dir, false);
}

fn install_panic_hook_inner(dir: PathBuf, breaker: bool) {
    INSTALL_ONCE.call_once(move || {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let bt = Backtrace::force_capture();
            let report = report_from_panic(info, &bt);
            let _ = record_crash(&dir, &report);
            previous(info);
            if breaker {
                let (max, window) = breaker_config();
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if record_panic_and_check(now, max, window) {
                    // Loud, unmissable, on both stderr (launchd → daemon.err) and
                    // the structured log — silence is impossible by design.
                    eprintln!(
                        "[panic-circuit-breaker] {max} panics within {window}s — forcing clean exit(1) so launchd relaunches a fresh daemon instead of limping half-dead"
                    );
                    tracing::error!(
                        target: "durability",
                        max, window_secs = window,
                        "panic circuit-breaker tripped; forcing clean exit(1) for launchd relaunch"
                    );
                    std::process::exit(1);
                }
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_trips_only_on_cluster_within_window() {
        // A single isolated panic does not trip (max=3).
        let mut t = Vec::new();
        assert!(!check_window(&mut t, 100, 3, 60));
        assert!(!check_window(&mut t, 110, 3, 60));
        // Third panic inside the 60s window trips.
        assert!(check_window(&mut t, 120, 3, 60));
    }

    #[test]
    fn breaker_prunes_stale_panics_outside_window() {
        let mut t = Vec::new();
        assert!(!check_window(&mut t, 0, 3, 60));
        assert!(!check_window(&mut t, 30, 3, 60));
        // This panic is >60s after the first, which is pruned — only 2 remain, no trip.
        assert!(!check_window(&mut t, 70, 3, 60));
        // A cluster that stays inside the window still trips.
        assert!(check_window(&mut t, 80, 3, 60));
    }

    fn report(msg: &str, ts: &str) -> CrashReport {
        CrashReport {
            timestamp: ts.to_string(),
            thread: "main".to_string(),
            location: "src/foo.rs:1:1".to_string(),
            message: msg.to_string(),
            backtrace: "  0: frame\n  1: frame".to_string(),
        }
    }

    #[test]
    fn record_then_collect_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = record_crash(dir.path(), &report("boom", "2026-06-15T10:00:00Z")).unwrap();
        assert!(p.exists());

        let logs = collect_crash_logs(dir.path());
        assert_eq!(logs.len(), 1);
        let (name, bytes) = &logs[0];
        assert!(name.starts_with("crash-") && name.ends_with(".log"));
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("boom"));
        assert!(text.contains("src/foo.rs:1:1"));
        assert!(text.contains("Backtrace:"));
    }

    #[test]
    fn collect_is_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        // Distinct pids aren't available, so write via distinct filenames by
        // forcing distinct mtimes through ordered writes.
        let a = dir.path().join("crash-aaa.log");
        let b = dir.path().join("crash-bbb.log");
        std::fs::write(&a, report("first", "t1").to_text()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&b, report("second", "t2").to_text()).unwrap();

        let logs = collect_crash_logs(dir.path());
        assert_eq!(logs.len(), 2);
        assert!(String::from_utf8(logs[0].1.clone())
            .unwrap()
            .contains("second"));
    }

    #[test]
    fn prune_keeps_only_max() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_CRASH_FILES + 5) {
            let p = dir.path().join(format!("crash-{i:03}.log"));
            std::fs::write(&p, "x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        prune(dir.path(), MAX_CRASH_FILES);
        let remaining = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(remaining, MAX_CRASH_FILES);
    }

    #[test]
    fn collect_on_missing_dir_is_empty() {
        let logs = collect_crash_logs(Path::new("/nonexistent/path/xyz"));
        assert!(logs.is_empty());
    }

    #[test]
    fn crash_consent_key_is_split_from_analytics_telemetry() {
        // #327 split: crash-report consent is a DEDICATED key, not the product-
        // analytics telemetry opt-in. The two must be distinct config keys so a
        // user can consent to one without the other.
        assert_eq!(CRASH_REPORTS_CONSENT_KEY, "crash_reports_consent");
        assert_ne!(
            CRASH_REPORTS_CONSENT_KEY, "GOOSE_TELEMETRY_ENABLED",
            "crash-report consent must not reuse the analytics telemetry key"
        );
    }

    #[test]
    fn redacted_bundle_scrubs_home_paths_keys_and_emails() {
        // A synthetic crash log carrying exactly the sensitive shapes the export
        // must never leak.
        let raw = report_from_leaky();
        let logs = vec![("crash-leaky.log".to_string(), raw.into_bytes())];
        let bundle = build_redacted_bundle(&logs);

        assert!(bundle.contains("[REDACTED]"), "must redact");
        assert!(!bundle.contains("/Users/jesse"), "home path must not leak");
        assert!(!bundle.contains("jesse@example.com"), "email must not leak");
        assert!(
            !bundle.contains("sk-abcdefghijklmnopqrstuvwxyz012345"),
            "api key must not leak"
        );
        // Still useful: the framing + the panic location survive.
        assert!(bundle.contains("crash-leaky.log"));
        assert!(bundle.contains("src/foo.rs:1:1"));
        assert!(bundle.contains("nothing was uploaded") || bundle.contains("LOCAL"));
    }

    #[test]
    fn redacted_bundle_is_honest_when_empty() {
        let bundle = build_redacted_bundle(&[]);
        assert!(bundle.contains("No crash reports"));
    }

    #[test]
    fn export_writes_a_redacted_file_to_disk() {
        // Drive the whole export against temp source/dest dirs — no env mutation,
        // so this can't race other tests reading the real state dir.
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("crash-x.log"), report_from_leaky()).unwrap();

        let export = export_redacted_bundle_in(src.path(), dst.path()).unwrap();
        assert!(export.path.exists(), "the redacted bundle must be written");
        assert_eq!(export.report_count, 1);
        let on_disk = std::fs::read_to_string(&export.path).unwrap();
        assert_eq!(on_disk, export.content, "returned content == file content");
        assert!(!on_disk.contains("/Users/jesse"), "file must be redacted");
        assert!(on_disk.contains("[REDACTED]"));
    }

    /// A crash-report text carrying every sensitive shape the export must scrub.
    fn report_from_leaky() -> String {
        CrashReport {
            timestamp: "2026-07-22T10:00:00Z".to_string(),
            thread: "main".to_string(),
            location: "src/foo.rs:1:1".to_string(),
            message: "boom for jesse@example.com".to_string(),
            backtrace: "0: at /Users/jesse/dev/permagent/crate.rs\n\
                 1: key sk-abcdefghijklmnopqrstuvwxyz012345"
                .to_string(),
        }
        .to_text()
    }
}
