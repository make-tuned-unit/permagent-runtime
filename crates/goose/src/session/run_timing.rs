//! Eval-only, process-local timing from CLI entry to the end of headless drain.
//! The sidecar is produced by native code, never parsed from model output.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const PHASE: &str = "cli_entry_to_post_drain_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTimingRecord {
    pub phase: String,
    pub run_key: String,
    pub pid: u32,
    pub root_session_id: String,
    pub first_tool_ms: Option<u64>,
    pub native_wall_ms: u64,
    #[serde(default)]
    pub other_session_first_tool_ms: BTreeMap<String, u64>,
}

struct RunClock {
    started: Instant,
    sidecar_base: PathBuf,
    run_key: String,
    first_by_session: BTreeMap<String, u64>,
}

impl RunClock {
    fn record_dispatch(&mut self, session_id: &str) {
        if !session_id.is_empty() {
            let elapsed = self.started.elapsed().as_millis() as u64;
            self.first_by_session
                .entry(session_id.to_owned())
                .or_insert(elapsed);
        }
    }

    fn snapshot(&self, root_session_id: &str) -> Option<RunTimingRecord> {
        if root_session_id.is_empty() {
            return None;
        }
        let native_wall_ms = self.started.elapsed().as_millis() as u64;
        let first_tool_ms = self.first_by_session.get(root_session_id).copied();
        let other_session_first_tool_ms = self
            .first_by_session
            .iter()
            .filter(|(id, _)| id.as_str() != root_session_id)
            .map(|(id, ms)| (id.clone(), *ms))
            .collect();
        Some(RunTimingRecord {
            phase: PHASE.to_owned(),
            run_key: self.run_key.clone(),
            pid: std::process::id(),
            root_session_id: root_session_id.to_owned(),
            first_tool_ms,
            native_wall_ms,
            other_session_first_tool_ms,
        })
    }
}

static CLOCK: OnceLock<Mutex<RunClock>> = OnceLock::new();

#[cfg(test)]
pub fn activate_for_test() {
    CLOCK.get_or_init(|| {
        Mutex::new(RunClock {
            started: Instant::now(),
            sidecar_base: PathBuf::from("/tmp/unused-native-timing"),
            run_key: "test".into(),
            first_by_session: BTreeMap::new(),
        })
    });
}

#[cfg(test)]
pub fn first_for_test(session_id: &str) -> Option<u64> {
    CLOCK
        .get()?
        .lock()
        .ok()?
        .first_by_session
        .get(session_id)
        .copied()
}

/// Called at CLI entry, before logging/configuration. Only eval children with
/// an explicit sidecar path activate this process-local recorder.
pub fn begin_from_env() {
    let (Ok(base), Ok(run_key)) = (
        std::env::var("PERMAGENT_EVAL_TIMING_SIDECAR"),
        std::env::var("PERMAGENT_EVAL_TIMING_RUN_KEY"),
    ) else {
        return;
    };
    if base.is_empty() || run_key.is_empty() {
        return;
    }
    let clock = RunClock {
        started: Instant::now(),
        sidecar_base: PathBuf::from(base),
        run_key,
        first_by_session: BTreeMap::new(),
    };
    let path = sidecar_path(&clock.sidecar_base, std::process::id());
    let _ = std::fs::remove_file(path); // a crashed earlier process cannot supply this run
    let _ = CLOCK.set(Mutex::new(clock));
}

pub fn sidecar_path(base: &std::path::Path, pid: u32) -> PathBuf {
    PathBuf::from(format!("{}.{}.json", base.display(), pid))
}

/// Called only after extension resolution, scope/argument validation, and
/// inspector approval, immediately before the actual client call is polled.
pub fn record_approved_dispatch(session_id: &str) {
    if let Some(lock) = CLOCK.get() {
        if let Ok(mut clock) = lock.lock() {
            clock.record_dispatch(session_id);
        }
    }
}

/// Snapshot the root session and separate other-session diagnostics (which
/// include workers in this process). The primary values share one clock.
pub fn finish(root_session_id: &str) -> Option<RunTimingRecord> {
    let lock = CLOCK.get()?;
    let clock = lock.lock().ok()?;
    clock.snapshot(root_session_id)
}

/// Write only after the complete headless drain. A failed write leaves the
/// candidate Unrated; it cannot be replaced by text printed by the model.
pub fn finish_to_sidecar(root_session_id: &str) -> std::io::Result<()> {
    let Some(record) = finish(root_session_id) else {
        return Ok(());
    };
    let Some(lock) = CLOCK.get() else {
        return Ok(());
    };
    let clock = lock
        .lock()
        .map_err(|_| std::io::Error::other("run clock poisoned"))?;
    let path = sidecar_path(&clock.sidecar_base, record.pid);
    let tmp = path.with_extension(format!("json.tmp.{}", record.pid));
    let bytes = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn clock() -> RunClock {
        RunClock {
            started: Instant::now(),
            sidecar_base: PathBuf::from("/tmp/unused-native-timing"),
            run_key: "one-run".into(),
            first_by_session: BTreeMap::new(),
        }
    }

    #[test]
    fn no_approved_tool_keeps_first_tool_unmeasured() {
        let record = clock().snapshot("parent").unwrap();
        assert_eq!(record.first_tool_ms, None);
        assert!(record.other_session_first_tool_ms.is_empty());
        assert_eq!(record.phase, PHASE);
    }

    #[test]
    fn first_approved_dispatch_precedes_slow_result_and_children_stay_separate() {
        let mut clock = clock();
        clock.record_dispatch("parent");
        let first = clock.snapshot("parent").unwrap().first_tool_ms.unwrap();
        std::thread::sleep(Duration::from_millis(5)); // stand-in for tool result latency
        clock.record_dispatch("parent"); // later tool cannot replace the first
        clock.record_dispatch("child");
        let record = clock.snapshot("parent").unwrap();
        assert_eq!(record.first_tool_ms, Some(first));
        assert!(record.native_wall_ms >= first + 5);
        assert!(record.other_session_first_tool_ms.contains_key("child"));
        assert_eq!(clock.snapshot("no-tools").unwrap().first_tool_ms, None);
    }
}
