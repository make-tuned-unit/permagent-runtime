//! Execution receipts for dispatched goals (#210).
//!
//! A Kanban state transition proves the card MOVED, not that the worker actually
//! consumed the goal and advanced it. An [`ExecutionReceipt`] records the
//! ground truth of one dispatch attempt: which worker was selected, the
//! capability snapshot used to route it (#211), the worker session/run id, the
//! daemon lifecycle that owns the dispatch, a liveness heartbeat, and — once the
//! attempt ends — its terminal state.
//!
//! HEARTBEAT SEMANTICS. The heartbeat proves the *dispatching lifecycle's
//! completion tracker* is alive and still awaiting this worker — it is NOT a
//! worker-output liveness signal (a hung-but-alive worker is out of scope here;
//! that needs per-engine output hooks). Combined with `dispatched_lifecycle`,
//! the heartbeat gives restart recovery a clean rebind-vs-stale test: a receipt
//! whose lifecycle matches the current process and whose heartbeat is recent is
//! a LIVE dispatch to rebind; a receipt from a different lifecycle (or with a
//! stale heartbeat) is an orphan to reclaim.
//!
//! The receipt is stored on the goal card's `metadata_json.execution_receipt`
//! (a plain JSON bag) — no schema change.

use serde::{Deserialize, Serialize};

/// Terminal / in-flight state of a single dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    /// Dispatched and the worker is (believed to be) running.
    Running,
    /// Parked awaiting a human decision (e.g. budget exhaustion).
    Waiting,
    /// Worker finished; goal advanced to Review.
    Completed,
    /// Worker abandoned by a daemon restart / lifecycle change.
    StaleWorker,
    /// Worker exceeded its wall-clock bound.
    Timeout,
    /// Worker blocked by a guard (e.g. a credential leak).
    Blocked,
    /// Attempt failed (retriable or terminal per the goal budget).
    Failed,
}

impl ReceiptState {
    /// Terminal states no longer accept a heartbeat.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, ReceiptState::Running | ReceiptState::Waiting)
    }
}

/// A per-attempt execution receipt (#210). Serialized onto the goal card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    /// The selected worker key.
    pub worker_key: String,
    /// The worker session / external run id for this attempt.
    pub session_id: String,
    /// The routing snapshot used to pick the worker (#211).
    #[serde(default)]
    pub capability_snapshot: serde_json::Value,
    /// Daemon lifecycle id that owns this dispatch's in-process tracker.
    pub dispatched_lifecycle: String,
    /// When the goal was dispatched (RFC3339).
    pub dispatched_at: String,
    /// Last liveness beat from the dispatching lifecycle's tracker (RFC3339).
    pub last_heartbeat_at: String,
    /// First observed worker output after dispatch, if any (RFC3339). Reserved
    /// for a future per-engine output hook; `None` until then.
    #[serde(default)]
    pub first_output_at: Option<String>,
    /// Current attempt state.
    pub state: ReceiptState,
    /// When the attempt reached a terminal state (RFC3339), if it has.
    #[serde(default)]
    pub terminal_at: Option<String>,
    /// The attempt number this receipt covers (1-based).
    #[serde(default)]
    pub attempt: u64,
}

impl ExecutionReceipt {
    /// Build the initial receipt at dispatch time (state = Running).
    pub fn new(
        worker_key: impl Into<String>,
        session_id: impl Into<String>,
        capability_snapshot: serde_json::Value,
        dispatched_lifecycle: impl Into<String>,
        dispatched_at: impl Into<String>,
        attempt: u64,
    ) -> Self {
        let dispatched_at = dispatched_at.into();
        Self {
            worker_key: worker_key.into(),
            session_id: session_id.into(),
            capability_snapshot,
            dispatched_lifecycle: dispatched_lifecycle.into(),
            last_heartbeat_at: dispatched_at.clone(),
            dispatched_at,
            first_output_at: None,
            state: ReceiptState::Running,
            terminal_at: None,
            attempt,
        }
    }

    /// Record a liveness beat. No-op once terminal.
    pub fn heartbeat(&mut self, now_rfc3339: impl Into<String>) {
        if !self.state.is_terminal() {
            self.last_heartbeat_at = now_rfc3339.into();
        }
    }

    /// Record worker stdout. The first observed timestamp is write-once, while
    /// every read refreshes liveness at the time the event is persisted.
    pub fn observe_output(
        &mut self,
        observed_at_rfc3339: impl Into<String>,
        heartbeat_at_rfc3339: impl Into<String>,
    ) {
        if self.state.is_terminal() {
            return;
        }
        if self.first_output_at.is_none() {
            self.first_output_at = Some(observed_at_rfc3339.into());
        }
        self.last_heartbeat_at = heartbeat_at_rfc3339.into();
    }

    /// Move to a terminal state, stamping `terminal_at` and a final heartbeat.
    pub fn finalize(&mut self, state: ReceiptState, now_rfc3339: impl Into<String>) {
        let now = now_rfc3339.into();
        self.last_heartbeat_at = now.clone();
        self.terminal_at = Some(now);
        self.state = state;
    }

    /// Rebind-vs-stale test for restart recovery. A receipt is a LIVE dispatch
    /// (rebind) only when it is non-terminal, was dispatched in the current
    /// lifecycle, and its heartbeat is within `staleness_secs`. Anything else is
    /// stale (reclaim).
    pub fn is_live(
        &self,
        current_lifecycle: &str,
        now: chrono::DateTime<chrono::Utc>,
        staleness_secs: i64,
    ) -> bool {
        if self.state.is_terminal() || self.dispatched_lifecycle != current_lifecycle {
            return false;
        }
        match chrono::DateTime::parse_from_rfc3339(&self.last_heartbeat_at) {
            Ok(beat) => (now - beat.with_timezone(&chrono::Utc)).num_seconds() <= staleness_secs,
            Err(_) => false,
        }
    }
}

/// How often the completion tracker beats an in-flight goal's receipt.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Default heartbeat-staleness window for the rebind-vs-stale test — a couple of
/// missed beats.
pub const DEFAULT_STALENESS_SECS: i64 = (HEARTBEAT_INTERVAL_SECS as i64) * 3;

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> serde_json::Value {
        serde_json::json!({ "worker_key": "codex", "required_kinds": ["code_edit"] })
    }

    #[test]
    fn new_receipt_is_running_and_beats_at_dispatch() {
        let r = ExecutionReceipt::new(
            "codex",
            "sess-1",
            snapshot(),
            "life-1",
            "2026-07-22T10:00:00+00:00",
            1,
        );
        assert_eq!(r.state, ReceiptState::Running);
        assert!(!r.state.is_terminal());
        assert_eq!(r.last_heartbeat_at, r.dispatched_at);
        assert_eq!(r.terminal_at, None);
    }

    #[test]
    fn heartbeat_advances_until_terminal_then_freezes() {
        let mut r = ExecutionReceipt::new(
            "codex",
            "sess-1",
            snapshot(),
            "life-1",
            "2026-07-22T10:00:00+00:00",
            1,
        );
        r.heartbeat("2026-07-22T10:00:30+00:00");
        assert_eq!(r.last_heartbeat_at, "2026-07-22T10:00:30+00:00");

        r.finalize(ReceiptState::Completed, "2026-07-22T10:05:00+00:00");
        assert_eq!(r.state, ReceiptState::Completed);
        assert_eq!(r.terminal_at.as_deref(), Some("2026-07-22T10:05:00+00:00"));

        // A late beat after terminal is ignored.
        r.heartbeat("2026-07-22T10:06:00+00:00");
        assert_eq!(r.last_heartbeat_at, "2026-07-22T10:05:00+00:00");
    }

    #[test]
    fn rebind_vs_stale() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-22T10:01:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut r = ExecutionReceipt::new(
            "codex",
            "sess-1",
            snapshot(),
            "life-1",
            "2026-07-22T10:00:50+00:00",
            1,
        );

        // Same lifecycle, fresh heartbeat → live (rebind).
        assert!(r.is_live("life-1", now, DEFAULT_STALENESS_SECS));

        // Different lifecycle → stale.
        assert!(!r.is_live("life-2", now, DEFAULT_STALENESS_SECS));

        // Same lifecycle but a heartbeat older than the window → stale.
        r.last_heartbeat_at = "2026-07-22T09:00:00+00:00".to_string();
        assert!(!r.is_live("life-1", now, DEFAULT_STALENESS_SECS));

        // Terminal receipts are never live.
        r.finalize(ReceiptState::Completed, "2026-07-22T10:00:59+00:00");
        assert!(!r.is_live("life-1", now, DEFAULT_STALENESS_SECS));
    }

    #[test]
    fn serde_round_trip_preserves_fields() {
        let mut r = ExecutionReceipt::new(
            "codex",
            "sess-1",
            snapshot(),
            "life-1",
            "2026-07-22T10:00:00+00:00",
            2,
        );
        r.finalize(ReceiptState::Timeout, "2026-07-22T10:30:00+00:00");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["state"], "timeout");
        let back: ExecutionReceipt = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }
}
