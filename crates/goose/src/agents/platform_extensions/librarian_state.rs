//! Librarian runtime state — observable by the status endpoint and HUD.
//!
//! The global singleton is written to (briefly) by `warm_and_run` / `describe_one`
//! and read by the status handler. Lock discipline: write lock held only for
//! microseconds (field mutation), never across awaited Ollama calls.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::{Arc, LazyLock, RwLock};

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
            },
            lifetime_stats: LifetimeStats {
                total_memories: 0,
                described: 0,
                pending: 0,
            },
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
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Warming;
    state.current_task = "Warming model — loading qwen2.5:3b into memory".to_string();
    state.current_memory = None;
    state.error_message = None;
    state.session_stats = SessionStats {
        batch_started_at: Some(Utc::now()),
        memories_described_this_session: 0,
        avg_seconds_per_memory: None,
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

/// Transition back to idle (e.g., after batch_complete timeout or scheduler resets).
pub fn set_idle() {
    let mut state = LIBRARIAN_STATE.write().unwrap();
    state.phase = LibrarianPhase::Idle;
    state.current_task = "Idle — waiting for next scheduled window".to_string();
    state.current_memory = None;
    state.error_message = None;
}
