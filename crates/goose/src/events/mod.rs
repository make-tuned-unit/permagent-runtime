//! Permagent Event Bus — global broadcast channel for runtime events.
//!
//! Every meaningful runtime action emits a [`PermagentEvent`] via [`emit()`].
//! WebSocket handlers and other consumers call [`subscribe()`] to receive a
//! live stream plus access to the replay buffer (last 1000 events).

pub mod activity;
pub mod clipboard_intercept;
pub mod nav_intercept;
pub mod voice_origin;
pub mod voice_pronounce;
pub mod voice_remainder;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Self-knowledge descriptor for the World View surface — the live 3D
/// visualization fed by this event bus over `/events`. Co-located here;
/// aggregated by `crate::agents::self_knowledge`. Static — always-on,
/// editorial-only.
pub const WORLD_VIEW_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "world_view",
        display_name: "World View",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A live 3D rotunda ringed by a colonnade where your three agents — you (the orchestrator), the Librarian, and the Reader — are embodied and animate from your real memory recall, worker state, and events streamed over /events. Real active goals light plaques over the working bay's benches, the Librarian pulls a book from the mezzanine wall during real describe runs, clicking a pedestal glides the camera to it, and the Mesh Stargate stands at the colonnade opening with an honest Forum plaque",
        why_it_matters:
            "The user can watch your background activity in real time, press T for a guided camera tour, pick any agent from the roster to follow in third-person and open its live HUD, and drive the followed agent on foot with the arrow keys or WASD",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

/// Self-knowledge descriptor for the Execution trace surface — a live view over
/// this same event bus. Lets the agent point the user at the raw event stream to
/// inspect what the runtime is doing. Static: editorial, no live status claim.
/// Lives as the Activity page inside Settings (2026-08 Console consolidation) —
/// the agent opens it via `navigate_app("Trace")`, which deep-links to
/// Settings → Activity.
pub const TRACE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "trace",
        display_name: "Execution trace",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A live, chronological readout of the runtime's most recent events straight off the running system's event streams — the Activity page in Settings, each entry a timestamp and event type as tool calls, worker activity, navigations, and lifecycle signals fire in real time. It reflects the whole running system and needs no session id",
        why_it_matters:
            "It is the low-level, in-the-moment 'what is the system doing right now' view for inspecting or debugging behavior as it happens — distinct from the Activity timeline, which is the curated, durable record of what your agents did; when the user wants to watch the raw event stream or see what just fired under the hood, bring them here",
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

/// Is anything listening to the event stream right now?
///
/// `emit` silently drops when there are no receivers, which is correct for
/// fire-and-forget notifications but a LIE for the request/response bridges
/// that ride this bus (open a website, read the page, drive a terminal): with
/// the desktop app closed — a phone-only session, the daemon being launchd-
/// managed and independent — those requests vanish and the agent still reports
/// success. Callers that need a UI must check this first and say so plainly.
pub fn has_listeners() -> bool {
    UI_CLIENTS.load(std::sync::atomic::Ordering::Relaxed) > 0
}

/// Connected UI clients on the `/events` WebSocket.
///
/// NOT `tx.receiver_count()`: the daemon itself holds permanent subscribers
/// (the notification router, the state-sync loops), so that count is always
/// non-zero and answered "yes, a UI is attached" even with every window shut.
/// Only a real websocket client counts.
static UI_CLIENTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// RAII guard: the `/events` handler holds one for the life of a connection,
/// so a dropped socket — closed, crashed, or network-lost — decrements without
/// the handler having to remember an unregister on every exit path.
pub struct UiClientGuard;

impl UiClientGuard {
    pub fn register() -> Self {
        UI_CLIENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for UiClientGuard {
    fn drop(&mut self) {
        UI_CLIENTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
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
    /// Server-side replay marker (#770 follow-up). `Some(true)` ONLY on frames
    /// a WebSocket handler re-delivers from its replay buffer on (re)connect;
    /// `None` everywhere else, which serializes to *no field at all* — live
    /// frames stay byte-identical to the pre-marker wire, and older clients
    /// simply ignore the extra key on replayed ones.
    ///
    /// Never set by emitters: replayedness is a property of one *delivery*,
    /// not of the event (the same buffered event replays to a reconnecting
    /// client while another client received it live), so [`emit`] traffic and
    /// the buffer always carry `None` and the `/events` route stamps its own
    /// clone per delivery via [`PermagentEvent::into_replayed`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub replayed: Option<bool>,
}

impl PermagentEvent {
    /// Create a new event with auto-generated UUIDv7 and current timestamp.
    pub fn new(event_type: PermagentEventType, payload: Value) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            event_type,
            timestamp: Utc::now(),
            payload,
            replayed: None,
        }
    }

    /// Stamp this delivery as a buffer replay (see the `replayed` field doc).
    /// Consumes and returns the event so replay loops can mark their owned
    /// clone inline without touching the buffered original.
    pub fn into_replayed(mut self) -> Self {
        self.replayed = Some(true);
        self
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
    /// Distinguishes malformed model output from tool execution failure because
    /// those failure classes require different fixes.
    ToolArgumentsInvalid,
    // Skills
    SkillProposed,
    SkillSaved,
    SkillTriggered,
    SkillRetired,
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
    /// The agent asked the in-app browser to open a URL (#567). The frontend
    /// bridge listens and routes it to the Build tab's browser.
    BrowserNavigateRequested,
    /// The agent asked for a snapshot of the open page's interactive elements
    /// (#649). The frontend bridge injects the grounding script and POSTs the
    /// stamped a11y refs back.
    BrowserSnapshotRequested,
    /// The agent asked to act on a ref — click / type / select (#649). Payload
    /// carries the ref, action and value; the frontend performs it and POSTs a
    /// fresh snapshot back.
    BrowserActRequested,
    // App navigation (chat agent → frontend)
    AppNavigate,
    // App action — act WITHIN a surface, not just navigate to it (chat agent →
    // frontend): toggle a Build pane, open/close/detach the chat dock, etc.
    AppAction,
    // App open-item — the last mile past a tab: open a SPECIFIC item by id (chat
    // agent → frontend): a goal's detail modal, a project's Grow planner, etc.
    AppOpenItem,
    // App clipboard — paste-ready text the user asked to copy (chat agent →
    // frontend). Voice turns intercept this and send it on `/voice` instead so
    // the copy lands on the device that is listening, not the daemon host.
    AppClipboard,
    // Project terminal launch (chat agent → frontend Build tab)
    ProjectLaunch,
    // Terminal supervision (S2, #428): a supervised Claude Code session hit a
    // permission gate (`control_request`/`can_use_tool`) — detected by the
    // deterministic stream-json parser, zero LLM. S3's inbox bridge consumes
    // this.
    TerminalGateDetected,
    // Terminal supervision (S2, #428): a previously detected gate is no longer
    // pending — an answer was observed in the session's stream (`answered`) or
    // the session reached a terminal state (`session_ended`).
    TerminalGateCleared,
    // Goal lifecycle (create / transition / park / requeue / failure / delete)
    GoalStateChanged,
    // Echo/Watcher — the agent proactively resurfaces something worth your
    // attention (a dormant Brain thread today; project news/analytics later).
    ProactiveNudge,
    // Notification routing (#66). These are delivery instructions emitted by
    // the daemon router, never raw workflow facts. Clients should notify only
    // from these events, which keeps per-user policy out of every producer.
    NotificationRouted,
    NotificationDigestReady,
    // ── Multi-client liveness (#629): emitted on REAL writes only, so a second
    // open client refreshes the affected surface instead of going stale. ──
    /// A workspace's persisted layout changed (PUT /api/workspaces/{id}/layout).
    WorkspaceChanged,
    /// A project or one of its owned collections (tags / people-assoc /
    /// memories-assoc / documents / notes) changed. Payload's `change` names
    /// the collection so clients can refresh narrowly.
    ProjectChanged,
    /// A person's project association changed (associate / disassociate).
    PersonChanged,
    /// The agent's primary persona was edited (PUT /api/agent/identity).
    IdentityChanged,
    /// The chat-session list changed (created / deleted / renamed / forked).
    SessionChanged,
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

/// Emitted whenever a goal card's lifecycle changes: creation, a checked
/// transition, park, requeue, a failure-latch, or deletion. Consumers refetch
/// the active-goal set, so the payload is informational. `from` is `None` on
/// creation; `to` is `"deleted"` on removal.
pub fn goal_state_changed(
    goal_id: &str,
    project_id: Option<&str>,
    from: Option<&str>,
    to: &str,
    actor: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::GoalStateChanged,
        serde_json::json!({
            "goal_id": goal_id,
            "project_id": project_id,
            "from": from,
            "to": to,
            "actor": actor,
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

/// A saved skill was auto-archived by the retirement sweep for never firing
/// within the grace window.
pub fn skill_retired(skill_id: &str, name: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::SkillRetired,
        serde_json::json!({
            "skill_id": skill_id,
            "name": name,
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

/// Emitted when the chat agent wants to ACT within a surface rather than just
/// navigate to it — toggle a Build pane, open/close/detach the chat dock, etc.
/// Sibling to [`app_navigate`]: the daemon never touches the DOM; the frontend
/// dispatcher catches this and calls the matching store action. `surface` +
/// `action` are validated against the app_conductor action catalog before this
/// is emitted, so the frontend can trust the pair.
pub fn app_action(
    surface: &str,
    action: &str,
    params: Option<&serde_json::Value>,
    reason: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::AppAction,
        serde_json::json!({
            "surface": surface,
            "action": action,
            "params": params,
            "reason": reason,
        }),
    )
}

/// Emitted when the chat agent wants the user's local clipboard to hold
/// paste-ready text (a post, a speech, a blurb they asked to copy). The
/// daemon never writes the pasteboard itself — that would copy on the hub
/// Mac, not an iPhone talking over `/voice`. Voice turns intercept this
/// (see [`clipboard_intercept`]) and flush the body down `/voice` as soon
/// as the tool returns, so the listening device can copy while it is still
/// in the foreground. Text chat emits here so the Command Center copies in
/// the focused window.
pub fn app_clipboard(text: &str, reason: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::AppClipboard,
        serde_json::json!({
            "text": text,
            "reason": reason,
        }),
    )
}

/// Emitted when the chat agent wants to open a SPECIFIC item — the last mile
/// past a tab. Sibling to [`app_action`]: the daemon never touches the DOM; the
/// frontend dispatcher catches this and calls the matching store seam that
/// already backs the human button (goal → `openGoalDetail`, grow →
/// `growProject`). `kind` is validated against the app_conductor item catalog
/// before this is emitted, so the frontend can trust it. `card_id` is only set
/// for kinds that need a second id (a goal's card); `project_id` is required for
/// every kind.
pub fn app_open_item(
    kind: &str,
    project_id: &str,
    card_id: Option<&str>,
    reason: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::AppOpenItem,
        serde_json::json!({
            "kind": kind,
            "project_id": project_id,
            "card_id": card_id,
            "reason": reason,
        }),
    )
}

/// Emitted when the chat agent asks the frontend to open a project-aware
/// terminal in the Build tab. Mirrors [`app_navigate`]: the agent does not
/// spawn the PTY directly — the command-center catches this and calls the
/// existing `createProjectTab` launch path (BuildView → terminal.rs).
///
/// `supervised_session_id` (S1, #427) tags a SUPERVISED stream-json Claude
/// Code launch with its loop session id (`sup-<uuid>`). The frontend ignores
/// it today; S2's session registry (#428) uses it to correlate the loop
/// session with the PTY the tab spawns. `None` for plain launches.
pub fn project_launch(
    root_path: &str,
    label: &str,
    command: Option<&str>,
    project_slug: &str,
    reason: &str,
    supervised_session_id: Option<&str>,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::ProjectLaunch,
        serde_json::json!({
            "root_path": root_path,
            "label": label,
            "command": command,
            "project_slug": project_slug,
            "reason": reason,
            "supervised_session_id": supervised_session_id,
        }),
    )
}

/// S2 (#428): a supervised Claude Code session hit a permission gate — a
/// `control_request`/`can_use_tool` NDJSON line arrived on its PTY stream and
/// the deterministic parser classified it. Everything S3 needs to raise a
/// `session_gate` decision rides in the payload: the addressing pair
/// (`supervised_session_id` + `pty_session_id`, the S5 relay address), the
/// project/goal association, and the gate itself (`request_id`, `tool_name`,
/// `input`, `tool_use_id` — echoed back verbatim in an `allow` answer).
pub fn terminal_gate_detected(
    supervised_session_id: &str,
    pty_session_id: Option<&str>,
    project_slug: &str,
    kind: crate::agents::platform_extensions::terminal_supervision::SupervisedSessionKind,
    root_path: &str,
    gate: &crate::agents::platform_extensions::terminal_supervision::PendingGate,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TerminalGateDetected,
        serde_json::json!({
            "supervised_session_id": supervised_session_id,
            "pty_session_id": pty_session_id,
            "project_slug": project_slug,
            "session_kind": kind,
            "root_path": root_path,
            "request_id": gate.request_id,
            "tool_name": gate.tool_name,
            "input": gate.input,
            "tool_use_id": gate.tool_use_id,
            "detected_at": gate.detected_at.to_rfc3339(),
        }),
    )
}

/// S2 (#428): a detected gate stopped being pending. `reason` is `"answered"`
/// (a `control_response` for its `request_id` was observed in the session's
/// stream — hand-typed today, S5-relayed later) or `"session_ended"` (the
/// session reached a terminal state with the gate still open).
pub fn terminal_gate_cleared(
    supervised_session_id: &str,
    request_id: &str,
    reason: &str,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::TerminalGateCleared,
        serde_json::json!({
            "supervised_session_id": supervised_session_id,
            "request_id": request_id,
            "reason": reason,
        }),
    )
}

/// Echo/Watcher (#672): the agent proactively surfaces something worth
/// interrupting for — a dormant Brain thread, project news, or (with the
/// Financier) an overbought sell signal on an open holding. News/dormant
/// nudges are gentle and rare; `sell_signal` is daily-per-symbol and does
/// not consume that taste budget. `kind` names the signal source so the UI
/// can style/route it.
pub fn proactive_nudge(
    kind: &str,
    subject: &str,
    message: &str,
    count: i64,
    last_ts: &str,
    url: Option<&str>,
    project: Option<(&str, &str)>,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::ProactiveNudge,
        serde_json::json!({
            "kind": kind,
            "subject": subject,
            "message": message,
            "count": count,
            "last_ts": last_ts,
            // The active project this nudge is grounded in, when there is one
            // (project-news grounding, audit 2026-08-11) — lets a client
            // deep-link to the project rather than a generic tab.
            "project_id": project.map(|(id, _)| id),
            "project_name": project.map(|(_, name)| name),
            // The thing the nudge is ABOUT. A news nudge that cannot be opened
            // is a strictly worse version of not being told: it spends the
            // user's attention and gives them nowhere to put it. The client has
            // read this key since the feature shipped (`notifications.ts`
            // `p.url ?? p.link ?? p.source_url` → `openInBrowser`); the server
            // just never sent it, so every article nudge was a dead end.
            "url": url,
        }),
    )
}

/// A workflow event passed the user's notification policy for an immediate
/// channel. `source_event_id` makes delivery idempotent for clients.
pub fn notification_routed(
    source_event_id: &str,
    user_id: &str,
    severity: &str,
    channel: &str,
    source_type: &str,
    source_payload: &Value,
) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::NotificationRouted,
        serde_json::json!({
            "source_event_id": source_event_id,
            "user_id": user_id,
            "severity": severity,
            "channel": channel,
            "source_type": source_type,
            "source_payload": source_payload,
        }),
    )
}

/// Batch boundary emitted after the day's individually routed digest entries.
/// `count` lets clients present one grouped affordance instead of N toasts.
pub fn notification_digest_ready(user_id: &str, count: i64) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::NotificationDigestReady,
        serde_json::json!({ "user_id": user_id, "count": count }),
    )
}

// ── Multi-client liveness constructors (#629) ───────────────────────────────
// Discipline: call these ONLY after the write succeeded — events fire on real
// mutations, never on attempts. Payloads carry ids + a `change` discriminator,
// never row bodies (payload discipline: clients refetch, the bus doesn't carry
// state).

/// A workspace's persisted layout changed. `change` is `"layout"` today.
pub fn workspace_changed(workspace_id: &str, change: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::WorkspaceChanged,
        serde_json::json!({
            "workspace_id": workspace_id,
            "change": change,
        }),
    )
}

/// A project (or an owned collection of it) changed. `change` ∈
/// `created|updated|deleted|touched|tags|memories|documents|notes`.
pub fn project_changed(project_id: &str, change: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::ProjectChanged,
        serde_json::json!({
            "project_id": project_id,
            "change": change,
        }),
    )
}

/// A person changed. `change` ∈ `associated|disassociated|created|updated|meeting`.
pub fn person_changed(project_id: &str, entity_uuid: &str, change: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::PersonChanged,
        serde_json::json!({
            "project_id": project_id,
            "entity_uuid": entity_uuid,
            "change": change,
        }),
    )
}

/// The primary persona was edited — clients re-read `/api/agent/identity`.
pub fn identity_changed(display_name: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::IdentityChanged,
        serde_json::json!({
            "display_name": display_name,
        }),
    )
}

/// The chat-session list changed. `change` ∈ `created|deleted|renamed|forked`.
pub fn session_changed(session_id: &str, change: &str) -> PermagentEvent {
    PermagentEvent::new(
        PermagentEventType::SessionChanged,
        serde_json::json!({
            "session_id": session_id,
            "change": change,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A news nudge must deliver the link, on the key the client reads.
    ///
    /// The URL was fetched, used for dedup, and then dropped at emit — so
    /// Henry could say "there's a fresh piece worth reading" with nowhere to
    /// go. The client has read `url` since the feature shipped
    /// (`notifications.ts` → `p.url ?? p.link ?? p.source_url` →
    /// `openInBrowser`), so this asserts the DOCUMENTED WIRE KEY, not merely
    /// that some field exists — the serde-field-never-bound rule.
    #[test]
    fn proactive_nudge_carries_the_link_on_the_key_the_client_reads() {
        let event = proactive_nudge(
            "news",
            "AI and software jobs",
            "There's a fresh piece worth a read.",
            1,
            "2026-08-08T08:35:00Z",
            Some("https://example.com/article"),
            Some(("proj-1", "Job search")),
        );
        let v = serde_json::to_value(&event).unwrap();
        assert_eq!(
            v["payload"]["url"], "https://example.com/article",
            "the client reads payload.url; anything else is a dead-end nudge"
        );
        assert_eq!(v["payload"]["project_id"], "proj-1");
        assert_eq!(v["payload"]["project_name"], "Job search");
    }

    /// A nudge with nothing to open must send `url: null`, not omit it — an
    /// absent key and a null one must not be distinguishable to the client,
    /// which treats both as "no link".
    #[test]
    fn proactive_nudge_without_a_link_is_explicitly_null() {
        let event = proactive_nudge(
            "brain",
            "a dormant thread",
            "Worth revisiting.",
            3,
            "t",
            None,
            None,
        );
        let v = serde_json::to_value(&event).unwrap();
        assert!(v["payload"].get("url").is_some());
        assert!(v["payload"]["url"].is_null());
        assert!(v["payload"]["project_id"].is_null());
    }

    #[test]
    fn test_event_serialization() {
        let event = daemon_started("0.1.0", "/tmp/config.yaml", "/tmp/spectral.db");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"daemon_started\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
    }

    #[test]
    fn app_clipboard_wire_type_is_snake_case_and_carries_text() {
        let event = app_clipboard("paste me", "You asked for the caption.");
        let v = serde_json::to_value(&event).unwrap();
        assert_eq!(v["type"], "app_clipboard");
        assert_eq!(v["payload"]["text"], "paste me");
        assert_eq!(v["payload"]["reason"], "You asked for the caption.");
    }

    /// The replay-marker wire contract (#770 follow-up): a plain (emitted /
    /// live) event serializes with NO `replayed` key at all — byte-compatible
    /// with the pre-marker wire — while a delivery stamped via
    /// `into_replayed` carries `"replayed":true`. Deserializing legacy JSON
    /// without the field yields `None` (older daemons / stored frames).
    #[test]
    fn replay_marker_wire_contract() {
        let event = daemon_started("0.1.0", "/tmp/config.yaml", "/tmp/spectral.db");
        assert_eq!(event.replayed, None, "constructors must never pre-mark");
        let live_json = serde_json::to_string(&event).unwrap();
        assert!(
            !live_json.contains("replayed"),
            "live frame must omit the marker entirely: {live_json}"
        );

        let marked = event.clone().into_replayed();
        let replay_json = serde_json::to_string(&marked).unwrap();
        assert!(
            replay_json.contains("\"replayed\":true"),
            "replayed delivery must carry the marker: {replay_json}"
        );

        // Legacy frames (no field) still deserialize; the marker reads None.
        let legacy: PermagentEvent = serde_json::from_str(&live_json).unwrap();
        assert_eq!(legacy.replayed, None);
        let roundtrip: PermagentEvent = serde_json::from_str(&replay_json).unwrap();
        assert_eq!(roundtrip.replayed, Some(true));
    }

    /// The buffer stores what `emit` produced — never a pre-marked frame — so
    /// every subscriber's replay decision is its own (marking is per delivery,
    /// in the route, not global state).
    #[test]
    fn buffer_never_stores_marked_frames() {
        let event = daemon_started("0.1.0", "/test-replay-buf", "/test-replay-buf");
        let id = event.id.clone();
        emit(event);
        let stored = buffered_events()
            .into_iter()
            .find(|e| e.id == id)
            .expect("emitted event present in buffer");
        assert_eq!(stored.replayed, None);
    }

    #[test]
    fn test_emit_and_subscribe() {
        let _rx = subscribe();
        let event = daemon_started("0.1.0", "/test", "/test");
        emit(event.clone());

        // Try to receive (non-blocking would need tokio runtime)
        // Just verify buffer works. The buffer is process-global and sibling
        // tests emit concurrently, so assert OUR event is present by id —
        // `.last()` races with whatever another test emitted after us.
        let buffered = buffered_events();
        assert!(!buffered.is_empty());
        assert!(
            buffered.iter().any(|e| e.id == event.id),
            "emitted event not found in buffer"
        );
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
