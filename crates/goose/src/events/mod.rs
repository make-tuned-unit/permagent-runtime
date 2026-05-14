//! Permagent Event Bus — global broadcast channel for runtime events.
//!
//! Every meaningful runtime action emits a [`PermagentEvent`] via [`emit()`].
//! WebSocket handlers and other consumers call [`subscribe()`] to receive a
//! live stream plus access to the replay buffer (last 1000 events).

pub mod activity;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Buffer size for the broadcast channel and replay buffer (Section I risk mitigation).
const EVENT_BUFFER_SIZE: usize = 1000;

// ── Global singleton ────────────────────────────────────────────────────────

struct EventBusInner {
    tx: broadcast::Sender<PermagentEvent>,
    buffer: Mutex<VecDeque<PermagentEvent>>,
}

static EVENT_BUS: LazyLock<EventBusInner> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(EVENT_BUFFER_SIZE);
    EventBusInner {
        tx,
        buffer: Mutex::new(VecDeque::with_capacity(EVENT_BUFFER_SIZE)),
    }
});

// ── Public API ──────────────────────────────────────────────────────────────

/// Emit an event to all subscribers. Silently drops if no receivers.
pub fn emit(event: PermagentEvent) {
    // Buffer for replay
    if let Ok(mut buf) = EVENT_BUS.buffer.lock() {
        buf.push_back(event.clone());
        while buf.len() > EVENT_BUFFER_SIZE {
            buf.pop_front();
        }
    }
    let _ = EVENT_BUS.tx.send(event);
}

/// Subscribe to the live event stream.
pub fn subscribe() -> broadcast::Receiver<PermagentEvent> {
    EVENT_BUS.tx.subscribe()
}

/// Get a snapshot of buffered events (up to last 1000).
pub fn buffered_events() -> Vec<PermagentEvent> {
    EVENT_BUS
        .buffer
        .lock()
        .map(|buf| buf.iter().cloned().collect())
        .unwrap_or_default()
}

/// Get buffered events after a given event ID. Returns None if the ID is not
/// found in the buffer (gap detected).
pub fn buffered_events_after(resume_from: &str) -> Option<Vec<PermagentEvent>> {
    let buf = EVENT_BUS.buffer.lock().ok()?;
    let pos = buf.iter().position(|e| e.id == resume_from);
    pos.map(|idx| buf.iter().skip(idx + 1).cloned().collect())
}

// ── Event types ─────────────────────────────────────────────────────────────

/// The canonical event envelope (Section C.3).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermagentEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: PermagentEventType,
    pub timestamp: DateTime<Utc>,
    pub payload: Value,
}

impl PermagentEvent {
    /// Create a new event with auto-generated UUIDv7 and current timestamp.
    pub fn new(event_type: PermagentEventType, payload: Value) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            event_type,
            timestamp: Utc::now(),
            payload,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermagentEventType {
    // Daemon lifecycle
    DaemonStarted,
    DaemonStopped,
    // Task lifecycle
    TaskCreated,
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    // Memory
    MemoryAdded,
    // Skills
    SkillProposed,
    SkillSaved,
    SkillTriggered,
    // Session / Chat
    MessageReceived,
    StreamChunk,
    // Integration
    IntegrationConnected,
    IntegrationError,
    // Activity awareness layer (Phase 1)
    Activity,
    // Librarian
    LibrarianDescribeStarted,
    LibrarianDescribeToken,
    LibrarianDescribeRetry,
    LibrarianDescribeCompleted,
}

// ── Convenience constructors ────────────────────────────────────────────────

pub fn daemon_started(version: &str, config_path: &str, spectral_path: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::DaemonStarted,
        serde_json::json!({
            "version": version,
            "config_path": config_path,
            "spectral_path": spectral_path,
        }),
    )
}

pub fn daemon_stopped(reason: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::DaemonStopped,
        serde_json::json!({ "reason": reason }),
    )
}

pub fn task_created(task_id: &str, description: &str, tool: Option<&str>) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TaskCreated,
        serde_json::json!({
            "task_id": task_id,
            "description": description,
            "tool": tool,
        }),
    )
}

pub fn task_started(task_id: &str, session_id: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TaskStarted,
        serde_json::json!({
            "task_id": task_id,
            "session_id": session_id,
        }),
    )
}

pub fn task_completed(task_id: &str, output: &Value, duration_ms: u64) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TaskCompleted,
        serde_json::json!({
            "task_id": task_id,
            "output": output,
            "duration_ms": duration_ms,
        }),
    )
}

pub fn task_failed(task_id: &str, error: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TaskFailed,
        serde_json::json!({
            "task_id": task_id,
            "error": error,
        }),
    )
}

pub fn memory_added(
    memory_id: &str,
    key: &str,
    category: &str,
    wing: Option<&str>,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::MemoryAdded,
        serde_json::json!({
            "memory_id": memory_id,
            "key": key,
            "category": category,
            "wing": wing,
        }),
    )
}

pub fn skill_proposed(
    description: &str,
    tool_used: &str,
    argument_shape_hash: &str,
    occurrence_count: i64,
    source_task_ids: &[String],
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::SkillProposed,
        serde_json::json!({
            "description": description,
            "tool_used": tool_used,
            "argument_shape_hash": argument_shape_hash,
            "occurrence_count": occurrence_count,
            "source_task_ids": source_task_ids,
        }),
    )
}

pub fn skill_saved(skill_id: &str, name: &str, trigger_type: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::SkillSaved,
        serde_json::json!({
            "skill_id": skill_id,
            "name": name,
            "trigger_type": trigger_type,
        }),
    )
}

pub fn skill_triggered(skill_id: &str, execution_id: &str, trigger_type: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::SkillTriggered,
        serde_json::json!({
            "skill_id": skill_id,
            "execution_id": execution_id,
            "trigger_type": trigger_type,
        }),
    )
}

pub fn message_received(session_id: &str, role: &str, content_preview: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::MessageReceived,
        serde_json::json!({
            "session_id": session_id,
            "role": role,
            "content_preview": content_preview,
        }),
    )
}

pub fn stream_chunk(session_id: &str, content: &str, done: bool) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::StreamChunk,
        serde_json::json!({
            "session_id": session_id,
            "content": content,
            "done": done,
        }),
    )
}

pub fn integration_connected(provider: &str, scopes: &[String]) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::IntegrationConnected,
        serde_json::json!({
            "provider": provider,
            "scopes": scopes,
        }),
    )
}

pub fn integration_error(provider: &str, error: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::IntegrationError,
        serde_json::json!({
            "provider": provider,
            "error": error,
        }),
    )
}

pub fn librarian_describe_started(memory_key: &str, started_at: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::LibrarianDescribeStarted,
        serde_json::json!({ "memory_key": memory_key, "started_at": started_at }),
    )
}

pub fn librarian_describe_token(memory_key: &str, token: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::LibrarianDescribeToken,
        serde_json::json!({ "memory_key": memory_key, "token": token }),
    )
}

pub fn librarian_describe_retry(memory_key: &str, attempt: u32) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::LibrarianDescribeRetry,
        serde_json::json!({ "memory_key": memory_key, "attempt": attempt }),
    )
}

pub fn librarian_describe_completed(
    memory_key: &str,
    description: &str,
    duration_ms: u64,
    quality: crate::agents::platform_extensions::librarian::DescriptionQuality,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::LibrarianDescribeCompleted,
        serde_json::json!({
            "memory_key": memory_key,
            "description": description,
            "duration_ms": duration_ms,
            "quality": quality,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_serialization() {
        let event = daemon_started("0.1.0", "/tmp/config.yaml", "/tmp/spectral.db");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"daemon_started\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
    }

    #[test]
    fn test_emit_and_subscribe() {
        let _rx = subscribe();
        let event = daemon_started("0.1.0", "/test", "/test");
        emit(event.clone());

        // Try to receive (non-blocking would need tokio runtime)
        // Just verify buffer works
        let buffered = buffered_events();
        assert!(!buffered.is_empty());
        let last = buffered.last().unwrap();
        assert_eq!(last.event_type, PermagentEventType::DaemonStarted);
    }

    #[test]
    fn test_buffered_events_after() {
        // Emit a few events
        let e1 = daemon_started("0.1.0", "/test", "/test");
        let e1_id = e1.id.clone();
        emit(e1);
        let e2 = daemon_stopped("test");
        emit(e2);

        let after = buffered_events_after(&e1_id);
        assert!(after.is_some());
        let events = after.unwrap();
        // Should have at least 1 event after e1
        assert!(events
            .iter()
            .any(|e| e.event_type == PermagentEventType::DaemonStopped));
    }
}
