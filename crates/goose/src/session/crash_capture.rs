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
use std::sync::Once;

use crate::config::paths::Paths;

const CRASH_SUBDIR: &str = "crashes";
/// Keep only the most recent N crash reports on disk.
const MAX_CRASH_FILES: usize = 20;

/// Directory where crash reports are written (`<state>/crashes`).
pub fn crash_dir() -> PathBuf {
    Paths::in_state_dir(CRASH_SUBDIR)
}

/// Whether crash reports may be included in the diagnostic bundle.
///
/// Off by default — reuses the PostHog telemetry opt-in so a single privacy
/// choice governs both. The `posthog` module is `#[cfg(feature = "telemetry")]`;
/// without it there is no opt-in to honor, so consent is denied (crash reports
/// are never bundled) — safe by construction.
pub fn crash_reports_consented() -> bool {
    #[cfg(feature = "telemetry")]
    {
        crate::posthog::is_telemetry_enabled()
    }
    #[cfg(not(feature = "telemetry"))]
    {
        false
    }
}

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

/// Install the global panic hook (idempotent). Writes crash reports to
/// [`crash_dir`] and then chains to the previous hook so stderr output is
/// unchanged.
pub fn install_panic_hook() {
    install_panic_hook_to(crash_dir());
}

/// Install the panic hook targeting a specific directory. Used by tests and the
/// evidence harness; production calls [`install_panic_hook`].
pub fn install_panic_hook_to(dir: PathBuf) {
    INSTALL_ONCE.call_once(move || {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let bt = Backtrace::force_capture();
            let report = report_from_panic(info, &bt);
            let _ = record_crash(&dir, &report);
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(feature = "telemetry")]
    #[test]
    fn consent_gate_delegates_to_telemetry_optin() {
        // The gate must equal the telemetry opt-in (off by default until the
        // user explicitly chooses).
        assert_eq!(
            crash_reports_consented(),
            crate::posthog::is_telemetry_enabled()
        );
    }

    #[cfg(not(feature = "telemetry"))]
    #[test]
    fn consent_denied_without_telemetry_feature() {
        // No telemetry module compiled in → no opt-in to honor → never bundle.
        assert!(!crash_reports_consented());
    }
}
