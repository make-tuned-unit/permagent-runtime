//! Permagent Event Bus — global broadcast channel for runtime events.
//!
//! Every meaningful runtime action emits a [`PermagentEvent`] via [`emit()`].
//! WebSocket handlers and other consumers call [`subscribe()`] to receive a
//! live stream plus access to the replay buffer (last 1000 events).

pub mod activity;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Self-knowledge descriptor for the World View surface — the live visualization
/// fed by this event bus over `/events`. Co-located here; aggregated by
/// `crate::agents::self_knowledge`. Static — always-on, editorial-only.
pub const WORLD_VIEW_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "world_view",
        display_name: "World View",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A live visualization of your internal activity — memory recall, worker state, and events — streamed over /events",
        why_it_matters:
            "The user can watch what you are doing in real time, so your background activity is visible to them",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

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

// ── Agent runtime-state registry (#348) ──────────────────────────────────────
//
// The authoritative, real-lifecycle source of an agent's coarse runtime state.
// Fed by the actual reply loop (`crate::agents::agent::Agent::reply_internal` via
// a Drop guard) — `working` is a live in-flight ref-count and `error` is a real
// failure latch, NOT the #288 interim-A derived-on-tick guess (which capped at
// `available`/`working` and could only SIMULATE error). Read by the World View
// signals: the `/api/henry/status` poll and the agent-state tick both consult
// this so the live `/events` push and the 2s poll agree on a real error instead
// of clobbering it.

/// An agent's coarse runtime state, as rendered by the World View HUD.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentRuntimeState {
    /// At least one reply turn is in flight (amber).
    Working,
    /// Idle, ready, no error (cyan steady-state).
    Available,
    /// The last reply turn ended in a real failure; latched until the next turn
    /// starts (red).
    Error,
}

impl AgentRuntimeState {
    /// HUD state string consumed directly by the frontend `agent_state_changed`
    /// handler and `mapHenryState`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Available => "available",
            Self::Error => "error",
        }
    }
}

#[derive(Default)]
struct AgentRuntime {
    /// In-flight reply turns. Ref-counted so concurrent sessions compose.
    active: i64,
    /// Latched after a real failure; cleared when the next turn starts.
    errored: bool,
}

static AGENT_RUNTIME: LazyLock<Mutex<HashMap<String, AgentRuntime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Real lifecycle hook: an agent has started a reply turn. Increments the
/// in-flight count and clears any prior error latch (new work = recovery).
pub fn record_agent_working(agent_id: &str) {
    if let Ok(mut m) = AGENT_RUNTIME.lock() {
        let e = m.entry(agent_id.to_string()).or_default();
        e.active += 1;
        e.errored = false;
    }
}

/// Real lifecycle hook: an agent finished a reply turn cleanly.
pub fn record_agent_done(agent_id: &str) {
    if let Ok(mut m) = AGENT_RUNTIME.lock() {
        if let Some(e) = m.get_mut(agent_id) {
            e.active = (e.active - 1).max(0);
        }
    }
}

/// Real lifecycle hook: an agent's reply turn ended in a real failure. Latches
/// `error` until the next turn starts.
pub fn record_agent_error(agent_id: &str) {
    if let Ok(mut m) = AGENT_RUNTIME.lock() {
        let e = m.entry(agent_id.to_string()).or_default();
        e.active = (e.active - 1).max(0);
        e.errored = true;
    }
}

/// Current runtime state, or `None` if the agent has never run a turn (callers
/// fall back to their own heuristic). An in-flight turn outranks a stale error
/// latch — work in progress means the agent is working, not failed.
pub fn agent_runtime_state(agent_id: &str) -> Option<AgentRuntimeState> {
    let m = AGENT_RUNTIME.lock().ok()?;
    let e = m.get(agent_id)?;
    Some(if e.active > 0 {
        AgentRuntimeState::Working
    } else if e.errored {
        AgentRuntimeState::Error
    } else {
        AgentRuntimeState::Available
    })
}

/// Whether the agent is currently latched in a real error state.
pub fn agent_errored(agent_id: &str) -> bool {
    matches!(
        agent_runtime_state(agent_id),
        Some(AgentRuntimeState::Error)
    )
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
    MemoryRecalled,
    // Entity / knowledge graph
    EntityAdded,
    EntityUpdated,
    // Decision inbox
    DecisionCreated,
    DecisionResolved,
    // Agent runtime state
    AgentStateChanged,
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
    // Browser content extraction
    BrowserContentRequested,
    // App navigation (chat agent → frontend)
    AppNavigate,
    // Project terminal launch (chat agent → frontend Build tab)
    ProjectLaunch,
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

/// Emitted when an agent actively recalls memories via Brain search.
/// Payload discipline: query/counts/source only — never the recalled content.
pub fn memory_recalled(query: &str, hit_count: usize, source: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::MemoryRecalled,
        serde_json::json!({
            "query": query,
            "hit_count": hit_count,
            "source": source,
        }),
    )
}

/// Emitted when a memory annotation surfaces an entity reference for the first
/// time (a new node on the knowledge graph). Payload: id/type only, no content.
pub fn entity_added(entity_id: &str, entity_type: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::EntityAdded,
        serde_json::json!({
            "entity_id": entity_id,
            "entity_type": entity_type,
        }),
    )
}

/// Emitted when an already-known entity is referenced again by a new annotation.
pub fn entity_updated(entity_id: &str, entity_type: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::EntityUpdated,
        serde_json::json!({
            "entity_id": entity_id,
            "entity_type": entity_type,
        }),
    )
}

/// Emitted when a decision lands in the inbox. Payload: id/kind/tier only —
/// never the headline/detail (those are user-facing content).
pub fn decision_created(decision_id: &str, kind: &str, tier: i64) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::DecisionCreated,
        serde_json::json!({
            "decision_id": decision_id,
            "kind": kind,
            "tier": tier,
        }),
    )
}

/// Emitted when a decision is answered. Payload: ids/classifications only.
pub fn decision_resolved(
    decision_id: &str,
    kind: &str,
    answer: &str,
    acted_by: &str,
    tier: i64,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::DecisionResolved,
        serde_json::json!({
            "decision_id": decision_id,
            "kind": kind,
            "answer": answer,
            "acted_by": acted_by,
            "tier": tier,
        }),
    )
}

/// Emitted when an agent's coarse runtime state transitions (idle/working/
/// available/error). `state` is a HUD state string the World View renders
/// directly. As of #348 the emitter sources `state` from the real lifecycle
/// registry ([`agent_runtime_state`]) rather than the #288 interim-A derive.
pub fn agent_state_changed(agent_id: &str, name: &str, state: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::AgentStateChanged,
        serde_json::json!({
            "agent_id": agent_id,
            "name": name,
            "state": state,
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

pub fn app_navigate(
    tab: &str,
    tool_type: &str,
    panel_type: &str,
    section: Option<&str>,
    state: Option<&serde_json::Value>,
    reason: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::AppNavigate,
        serde_json::json!({
            "tab": tab,
            "tool_type": tool_type,
            "panel_type": panel_type,
            "section": section,
            "state": state,
            "reason": reason,
        }),
    )
}

/// Emitted when the chat agent asks the frontend to open a project-aware
/// terminal in the Build tab. Mirrors [`app_navigate`]: the agent does not
/// spawn the PTY directly — the command-center catches this and calls the
/// existing `createProjectTab` launch path (BuildView → terminal.rs).
pub fn project_launch(
    root_path: &str,
    label: &str,
    command: Option<&str>,
    project_slug: &str,
    reason: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::ProjectLaunch,
        serde_json::json!({
            "root_path": root_path,
            "label": label,
            "command": command,
            "project_slug": project_slug,
            "reason": reason,
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

    #[test]
    fn test_agent_runtime_state_lifecycle() {
        // Distinct id per test — the registry is a process-global.
        let id = "test_agent_lifecycle";
        assert_eq!(agent_runtime_state(id), None, "unknown agent → None");

        record_agent_working(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Working));
        assert!(!agent_errored(id));

        record_agent_done(id);
        assert_eq!(
            agent_runtime_state(id),
            Some(AgentRuntimeState::Available),
            "clean finish → available"
        );

        record_agent_working(id);
        record_agent_error(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Error));
        assert!(agent_errored(id), "real failure latches error");

        // A new turn starting clears the stale error latch (recovery).
        record_agent_working(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Working));
        assert!(!agent_errored(id));
        record_agent_done(id);
    }

    #[test]
    fn test_agent_runtime_state_refcount() {
        // Concurrent reply turns compose: working until the last one finishes.
        let id = "test_agent_refcount";
        record_agent_working(id);
        record_agent_working(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Working));
        record_agent_done(id);
        assert_eq!(
            agent_runtime_state(id),
            Some(AgentRuntimeState::Working),
            "still one turn in flight"
        );
        record_agent_done(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Available));
        // Saturating: extra done never drives the count negative.
        record_agent_done(id);
        assert_eq!(agent_runtime_state(id), Some(AgentRuntimeState::Available));
    }

    #[test]
    fn test_agent_runtime_state_str() {
        assert_eq!(AgentRuntimeState::Working.as_str(), "working");
        assert_eq!(AgentRuntimeState::Available.as_str(), "available");
        assert_eq!(AgentRuntimeState::Error.as_str(), "error");
    }
}
