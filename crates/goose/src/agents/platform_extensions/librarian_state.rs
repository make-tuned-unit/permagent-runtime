//! Librarian runtime state — observable by the status endpoint and HUD.
//!
//! The global singleton is written to (briefly) by `warm_and_run` / `describe_one`
//! and read by the status handler. Lock discipline: write lock held only for
//! microseconds (field mutation), never across awaited Ollama calls.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

// ── Public state types ──────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarianPhase {
    Idle,
    Warming,
    Describing,
    BatchComplete,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct CurrentMemory {
    pub key: String,
    pub content_preview: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionStats {
    pub batch_started_at: Option<DateTime<Utc>>,
    pub memories_described_this_session: usize,
    pub avg_seconds_per_memory: Option<f64>,
    /// Descriptions this window whose model output could not be parsed into the
    /// three-field contract — including the ones the salvage path rescued.
    ///
    /// A prior health review asked for the malformed-output RATE, not just the
    /// individual log lines: a single unparseable response is a weak local
    /// model having a bad night, while "40 of 60" is a broken prompt or a
    /// mis-quantised model, and only a counter next to
    /// `memories_described_this_session` tells the two apart. Window-scoped,
    /// like every other field here — `set_warming` resets it.
    pub parse_failures_this_session: usize,
    /// The subset of `parse_failures_this_session` that no amount of salvaging
    /// could rescue, so the raw model dump was stored as the description.
    pub unsalvageable_parse_failures_this_session: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct LifetimeStats {
    pub total_memories: usize,
    pub described: usize,
    pub pending: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct LibrarianRuntimeState {
    pub phase: LibrarianPhase,
    pub current_task: String,
    pub current_memory: Option<CurrentMemory>,
    pub retry_in_progress: bool,
    pub error_message: Option<String>,
    pub session_stats: SessionStats,
    pub lifetime_stats: LifetimeStats,
    /// #387 v2 — graph entities the entity sweep could not truthfully describe
    /// (INSUFFICIENT_CONTEXT with real mention signal). Rendered in the
    /// capabilities brief as "K awaiting your context" (omitted at 0) so the
    /// agent knows to ask the user about them. Set by the entity sweep and
    /// re-seeded from the sidecar ledger at daemon boot.
    pub entities_awaiting_context: usize,
}

impl Default for LibrarianRuntimeState {
    fn default() -> Self {
        Self {
            phase: LibrarianPhase::Idle,
            current_task: "Idle — waiting for next scheduled window".to_string(),
            current_memory: None,
            retry_in_progress: false,
            error_message: None,
            session_stats: SessionStats {
                batch_started_at: None,
                memories_described_this_session: 0,
                avg_seconds_per_memory: None,
                parse_failures_this_session: 0,
                unsalvageable_parse_failures_this_session: 0,
            },
            lifetime_stats: LifetimeStats {
                total_memories: 0,
                described: 0,
                pending: 0,
            },
            entities_awaiting_context: 0,
        }
    }
}

// ── Global singleton ────────────────────────────────────────────────

static LIBRARIAN_STATE: LazyLock<Arc<RwLock<LibrarianRuntimeState>>> =
    LazyLock::new(|| Arc::new(RwLock::new(LibrarianRuntimeState::default())));

/// Get a read snapshot of the current state.
pub fn get_state() -> LibrarianRuntimeState {
    LIBRARIAN_STATE.read().unwrap().clone()
}

/// Transition to warming phase at batch start.
pub fn set_warming(total_memories: usize, described: usize) {
    reset_dedicated_endpoint_gate();
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Warming;
    state.current_task = "Warming model — loading qwen2.5:3b into memory".to_string();
    state.current_memory = None;
    state.error_message = None;
    state.session_stats = SessionStats {
        batch_started_at: Some(Utc::now()),
        memories_described_this_session: 0,
        avg_seconds_per_memory: None,
        parse_failures_this_session: 0,
        unsalvageable_parse_failures_this_session: 0,
    };
    state.lifetime_stats = LifetimeStats {
        total_memories,
        described,
        pending: total_memories.saturating_sub(described),
    };
}

/// Transition to describing phase (after warm completes, before first describe_one).
pub fn set_describing(pending_count: usize) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Describing;
    state.current_task = format!("Describing memories — {} pending", pending_count);
    // Update pending in lifetime stats
    state.lifetime_stats.pending = pending_count;
}

/// Set the current memory being described.
pub fn set_current_memory(key: &str, content: &str) {
    let preview: String = content.chars().take(80).collect();
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.current_memory = Some(CurrentMemory {
        key: key.to_string(),
        content_preview: preview,
    });
    let described = state.session_stats.memories_described_this_session;
    let pending = state.lifetime_stats.pending;
    state.current_task = format!(
        "Describing memory {} of ~{}",
        described + 1,
        described + pending
    );
}

/// Mark retry in progress for the current memory.
pub fn set_retry_in_progress(in_progress: bool) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.retry_in_progress = in_progress;
}

/// Called after a successful describe_one. Updates session/lifetime stats.
pub fn record_describe_success(duration_secs: f64) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.current_memory = None;
    state.retry_in_progress = false;
    state.session_stats.memories_described_this_session += 1;
    state.lifetime_stats.described += 1;
    state.lifetime_stats.pending = state.lifetime_stats.pending.saturating_sub(1);

    // Rolling average
    let n = state.session_stats.memories_described_this_session as f64;
    state.session_stats.avg_seconds_per_memory =
        Some(match state.session_stats.avg_seconds_per_memory {
            Some(prev) => prev + (duration_secs - prev) / n,
            None => duration_secs,
        });
}

/// Called when describe_one fails for a memory.
pub fn record_describe_failure(error: &str) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.current_memory = None;
    state.retry_in_progress = false;
    // Don't change phase to error for individual failures — only if the whole batch errors.
    // Just log the error context for the current task display.
    state.current_task = "Skipped memory (error) — continuing batch".to_string();
    state.error_message = Some(error.to_string());
}

/// Count one malformed-model-output parse failure for this window.
///
/// `salvaged` is `true` when the salvage path still produced usable index
/// fields — the memory stays searchable, but the model still broke its
/// contract, so it belongs in the rate. `false` means the raw dump was stored
/// and the memory is effectively unsearchable.
///
/// Returns the running count so the caller can put the rate on the log line
/// without taking the lock twice.
pub fn record_parse_failure(salvaged: bool) -> usize {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.session_stats.parse_failures_this_session += 1;
    if !salvaged {
        state
            .session_stats
            .unsalvageable_parse_failures_this_session += 1;
    }
    state.session_stats.parse_failures_this_session
}

/// Transition to batch_complete.
pub fn set_batch_complete() {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::BatchComplete;
    let n = state.session_stats.memories_described_this_session;
    state.current_task = format!("Batch complete — described {} memories", n);
    state.current_memory = None;
    state.error_message = None;
}

/// Transition to error (fatal batch-level error like warm-load failure).
pub fn set_error(message: &str) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Error;
    state.current_task = format!("Error: {}", message);
    state.current_memory = None;
    state.error_message = Some(message.to_string());
}

/// #387 v2 — update the count of entities awaiting user context (the entity
/// sweep's `needs_context` queue length). Rendered in the capabilities brief
/// only when > 0, so the zero state stays snapshot-identical.
pub fn set_entities_awaiting_context(count: usize) {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.entities_awaiting_context = count;
}

/// Transition back to idle (e.g., after batch_complete timeout or scheduler resets).
pub fn set_idle() {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Idle;
    state.current_task = "Idle — waiting for next scheduled window".to_string();
    state.current_memory = None;
    state.error_message = None;
}

// ── Dedicated-endpoint circuit (loopback connect storms) ─────────────
//
// Observed 2026-08-22: PERMAGENT_LIBRARIAN_ENDPOINT=http://127.0.0.1:8080
// with nothing listening produced 259 identical "llama-server unreachable"
// warnings over 88 minutes. Each memory re-probed loopback, failed, and
// silently fell back to qwen2.5:7b. A refused connection on this machine
// cannot recover mid-pass — nothing starts a service that is not running.
// Trip after a handful of loopback connect failures, alert once, skip the
// rest of the pass. A new nightly window (`set_warming`) resets so tomorrow
// gets another three probes if the operator brought the split up.

const LOOPBACK_CONNECT_FAILS_BEFORE_TRIP: u32 = 3;

#[derive(Default)]
pub(crate) struct LoopbackFailBudget {
    fails: u32,
    tripped: bool,
}

impl LoopbackFailBudget {
    /// Returns `true` the moment this fail trips the circuit (alert once).
    pub(crate) fn note_fail(&mut self) -> bool {
        if self.tripped {
            return false;
        }
        self.fails = self.fails.saturating_add(1);
        if self.fails < LOOPBACK_CONNECT_FAILS_BEFORE_TRIP {
            return false;
        }
        self.tripped = true;
        true
    }

    pub(crate) fn is_tripped(&self) -> bool {
        self.tripped
    }
}

#[derive(Default)]
struct DedicatedEndpointGate {
    endpoint: String,
    budget: LoopbackFailBudget,
}

static DEDICATED_GATE: LazyLock<Mutex<DedicatedEndpointGate>> =
    LazyLock::new(|| Mutex::new(DedicatedEndpointGate::default()));

/// True once a loopback dedicated endpoint has been marked down for this pass.
pub fn dedicated_endpoint_is_skipped(endpoint: &str) -> bool {
    let gate = DEDICATED_GATE.lock().unwrap();
    gate.budget.is_tripped() && gate.endpoint == endpoint
}

/// Record a connect failure to a loopback dedicated endpoint.
///
/// Returns `true` the moment the circuit trips (caller should alert once).
/// Non-loopback or non-connect failures do not increment — a remote split
/// can come up mid-window, and a 5xx means something is listening.
pub fn note_dedicated_loopback_connect_fail(endpoint: &str) -> bool {
    let mut gate = DEDICATED_GATE.lock().unwrap();
    if gate.endpoint != endpoint {
        *gate = DedicatedEndpointGate {
            endpoint: endpoint.to_string(),
            ..DedicatedEndpointGate::default()
        };
    }
    let just_tripped = gate.budget.note_fail();
    drop(gate);
    if !just_tripped {
        return false;
    }
    // Keep the batch running on the fallback — this is not a fatal Librarian
    // failure, just a dead local sidecar. Surface it once on the HUD.
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.error_message = Some(format!(
        "Dedicated endpoint {endpoint} is down on loopback — \
         not retrying this pass (nothing is listening)"
    ));
    true
}

/// A successful dedicated-endpoint call clears the storm counter.
pub fn note_dedicated_endpoint_ok(endpoint: &str) {
    let mut gate = DEDICATED_GATE.lock().unwrap();
    *gate = DedicatedEndpointGate {
        endpoint: endpoint.to_string(),
        ..DedicatedEndpointGate::default()
    };
}

/// New nightly window: three probes again, in case the split came up overnight.
pub fn reset_dedicated_endpoint_gate() {
    *DEDICATED_GATE.lock().unwrap() = DedicatedEndpointGate::default();
}

/// Trip the circuit outright, without spending the fail budget one memory at a
/// time. This is what the ONE readiness probe at the top of a nightly window
/// calls when loopback refuses.
///
/// The circuit alone already bounded the 2026-08-22 storm from 259 warnings to
/// three, but three is still three log lines and three dead HTTP attempts every
/// night for a service the operator has not started. A single probe before the
/// pass makes a down endpoint cost exactly ONE line per day, and the pass runs
/// on the Ollama fallback from the very first memory rather than the fourth.
///
/// Returns `true` when this call is what tripped it (so the caller logs once);
/// `false` if it was already tripped for this endpoint.
pub fn mark_dedicated_endpoint_down(endpoint: &str) -> bool {
    {
        let mut gate = DEDICATED_GATE.lock().unwrap();
        if gate.endpoint == endpoint && gate.budget.is_tripped() {
            return false;
        }
        *gate = DedicatedEndpointGate {
            endpoint: endpoint.to_string(),
            budget: LoopbackFailBudget {
                fails: LOOPBACK_CONNECT_FAILS_BEFORE_TRIP,
                tripped: true,
            },
        };
    }
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.error_message = Some(format!(
        "Dedicated endpoint {endpoint} did not answer a readiness probe — \
         skipping it for this pass (nothing is listening)"
    ));
    true
}

#[cfg(test)]
mod dedicated_endpoint_gate_tests {
    use super::*;

    const EP: &str = "http://127.0.0.1:8080";

    /// The cap: the Nth identical loopback failure trips the circuit, and every
    /// failure after it is silent. 2026-08-22 produced 259 of these; the budget
    /// is what turns that into three.
    #[test]
    fn fail_budget_trips_once_then_stays_quiet() {
        let mut budget = LoopbackFailBudget::default();
        let mut alerts = 0;
        for _ in 0..100 {
            if budget.note_fail() {
                alerts += 1;
            }
        }
        assert_eq!(alerts, 1, "the circuit must alert exactly once");
        assert!(budget.is_tripped());
        assert_eq!(
            budget.fails, LOOPBACK_CONNECT_FAILS_BEFORE_TRIP,
            "the counter must stop at the cap, not keep climbing"
        );
    }

    /// The probe path: one call marks the endpoint down for the whole pass, so
    /// a down service costs ONE log line per day instead of three.
    #[test]
    #[serial_test::serial(librarian_global_state)]
    fn readiness_probe_failure_skips_the_whole_pass() {
        reset_dedicated_endpoint_gate();
        assert!(!dedicated_endpoint_is_skipped(EP));

        assert!(
            mark_dedicated_endpoint_down(EP),
            "the first probe failure must report that it tripped, so it is logged"
        );
        assert!(dedicated_endpoint_is_skipped(EP));

        for _ in 0..50 {
            assert!(
                !mark_dedicated_endpoint_down(EP),
                "a tripped endpoint must never alert again in the same pass"
            );
        }
        reset_dedicated_endpoint_gate();
    }

    /// A tripped circuit is scoped to the endpoint that failed — reconfiguring
    /// to a different host must get its own budget, not inherit a trip.
    #[test]
    #[serial_test::serial(librarian_global_state)]
    fn trip_is_scoped_to_one_endpoint() {
        reset_dedicated_endpoint_gate();
        mark_dedicated_endpoint_down(EP);
        assert!(dedicated_endpoint_is_skipped(EP));
        assert!(!dedicated_endpoint_is_skipped("http://100.74.232.95:8080"));
        reset_dedicated_endpoint_gate();
    }

    /// A new nightly window gets a fresh probe: the operator may have brought
    /// the split up during the day.
    #[test]
    #[serial_test::serial(librarian_global_state)]
    fn a_new_window_resets_the_gate() {
        reset_dedicated_endpoint_gate();
        mark_dedicated_endpoint_down(EP);
        assert!(dedicated_endpoint_is_skipped(EP));

        set_warming(10, 0);
        assert!(
            !dedicated_endpoint_is_skipped(EP),
            "set_warming starts a new window and must clear the trip"
        );
        reset_dedicated_endpoint_gate();
        set_idle();
    }

    /// A success clears the storm counter, so an endpoint that flaps once does
    /// not spend the budget it needs later in the pass.
    #[test]
    #[serial_test::serial(librarian_global_state)]
    fn success_clears_the_budget() {
        reset_dedicated_endpoint_gate();
        assert!(!note_dedicated_loopback_connect_fail(EP));
        assert!(!note_dedicated_loopback_connect_fail(EP));
        note_dedicated_endpoint_ok(EP);
        // Budget is back to full: two more fails still must not trip.
        assert!(!note_dedicated_loopback_connect_fail(EP));
        assert!(!note_dedicated_loopback_connect_fail(EP));
        assert!(!dedicated_endpoint_is_skipped(EP));
        reset_dedicated_endpoint_gate();
    }
}
