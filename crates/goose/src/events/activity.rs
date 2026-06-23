//! Activity event taxonomy for the Permagent awareness layer.
//!
//! Every in-app surface emits structured activity events onto the shared
//! event bus. A future ContextBuilder (Phase 2) will subscribe to these
//! events and feed the agent ambient awareness.
//!
//! # Event tier mapping
//!
//! **Always** (substantive — will become Brain memory in Phase 2):
//!   ChatTurnCompleted, TerminalCommandCompleted, FileEdited,
//!   ProjectSelected, SkillExecuted, IntegrationTokenRefreshed,
//!   AutomationJobCompleted, AutomationJobFailed
//!
//! **Aggregated** (rolled up over time windows):
//!   BrowserNavigated, BrowserFormSubmitted
//!
//! **Ephemeral** (live-only, never persisted):
//!   ChatTurnStarted, BrowserSessionStarted, BrowserSessionEnded,
//!   FileOpened, ProjectOpened, TerminalCommandStarted,
//!   TerminalSessionStarted, TerminalSessionEnded, AgentContextProbed,
//!   AutomationJobStarted, TerminalWaitingForInput, TerminalProcessExited

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{emit, PermagentEvent, PermagentEventType};

// ── Enums ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEventType {
    ChatTurnStarted,
    ChatTurnCompleted,
    BrowserNavigated,
    BrowserFormSubmitted,
    BrowserSessionStarted,
    BrowserSessionEnded,
    TerminalCommandStarted,
    TerminalCommandCompleted,
    TerminalSessionStarted,
    TerminalSessionEnded,
    /// A raw-mode program (e.g. Claude Code) has gone quiescent while holding
    /// input — a heuristic "waiting for the user" signal. See the frontend
    /// quiescence detector in `Terminal.tsx`. L2-observe only (#400).
    TerminalWaitingForInput,
    /// The PTY's foreground process (the shell) exited. Distinct from
    /// `TerminalSessionEnded` (webview unmount) and `TerminalCommandCompleted`
    /// (a command returned to the prompt). L2-observe only (#400).
    TerminalProcessExited,
    ProjectSelected,
    ProjectOpened,
    FileOpened,
    FileEdited,
    SkillExecuted,
    IntegrationTokenRefreshed,
    AgentContextProbed,
    AutomationJobStarted,
    AutomationJobCompleted,
    AutomationJobFailed,
    StarterRecipeUpgraded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceSurface {
    Chat,
    Browser,
    Terminal,
    ProjectPicker,
    FileViewer,
    IntegrationsPanel,
    SkillsEngine,
    Agent,
    External,
    Scheduler,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventTier {
    Always,
    Aggregated,
    Ephemeral,
}

// ── Envelope ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub event_id: String,
    pub event_type: ActivityEventType,
    pub source_surface: SourceSurface,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub payload: serde_json::Value,
    pub tier: EventTier,
}

impl ActivityEvent {
    pub fn new(
        event_type: ActivityEventType,
        source_surface: SourceSurface,
        payload: serde_json::Value,
    ) -> Self {
        let tier = default_tier(&event_type);
        Self {
            event_id: Uuid::now_v7().to_string(),
            event_type,
            source_surface,
            timestamp: Utc::now(),
            session_id: None,
            project_id: None,
            payload,
            tier,
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
}

/// Returns the canonical tier for an event type. Used server-side to
/// enforce correct tier regardless of what the frontend sends.
pub fn canonical_tier(event_type: &ActivityEventType) -> EventTier {
    default_tier(event_type)
}

fn default_tier(event_type: &ActivityEventType) -> EventTier {
    match event_type {
        ActivityEventType::ChatTurnCompleted
        | ActivityEventType::TerminalCommandCompleted
        | ActivityEventType::FileEdited
        | ActivityEventType::ProjectSelected
        | ActivityEventType::SkillExecuted
        | ActivityEventType::IntegrationTokenRefreshed
        | ActivityEventType::AutomationJobCompleted
        | ActivityEventType::AutomationJobFailed
        | ActivityEventType::StarterRecipeUpgraded => EventTier::Always,

        ActivityEventType::BrowserNavigated | ActivityEventType::BrowserFormSubmitted => {
            EventTier::Aggregated
        }

        ActivityEventType::ChatTurnStarted
        | ActivityEventType::BrowserSessionStarted
        | ActivityEventType::BrowserSessionEnded
        | ActivityEventType::FileOpened
        | ActivityEventType::ProjectOpened
        | ActivityEventType::TerminalCommandStarted
        | ActivityEventType::TerminalSessionStarted
        | ActivityEventType::TerminalSessionEnded
        | ActivityEventType::TerminalWaitingForInput
        | ActivityEventType::TerminalProcessExited
        | ActivityEventType::AgentContextProbed
        | ActivityEventType::AutomationJobStarted => EventTier::Ephemeral,
    }
}

// ── Ring buffer for /activity/recent debug endpoint ─────────────────────

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

const ACTIVITY_BUFFER_SIZE: usize = 500;

static ACTIVITY_BUFFER: LazyLock<Mutex<VecDeque<ActivityEvent>>> =
    LazyLock::new(|| Mutex::new(VecDeque::with_capacity(ACTIVITY_BUFFER_SIZE)));

/// Emit an activity event: stores in the activity ring buffer and
/// forwards to the global PermagentEvent bus with channel "activity".
pub fn emit_activity(event: ActivityEvent) {
    // Buffer for /activity/recent
    if let Ok(mut buf) = ACTIVITY_BUFFER.lock() {
        buf.push_back(event.clone());
        while buf.len() > ACTIVITY_BUFFER_SIZE {
            buf.pop_front();
        }
    }

    // Forward to the global event bus as a PermagentEvent
    let payload = serde_json::json!({
        "channel": "activity",
        "event": event,
    });
    emit(PermagentEvent::new(PermagentEventType::Activity, payload));
}

/// Get the last N activity events from the ring buffer.
pub fn recent_activity(limit: usize) -> Vec<ActivityEvent> {
    ACTIVITY_BUFFER
        .lock()
        .map(|buf| {
            buf.iter()
                .rev()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
        .unwrap_or_default()
}

// ── Convenience constructors ────────────────────────────────────────────

pub fn chat_turn_started(session_id: &str) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::ChatTurnStarted,
        SourceSurface::Chat,
        serde_json::json!({}),
    )
    .with_session(session_id)
}

pub fn chat_turn_completed(
    session_id: &str,
    duration_ms: u64,
    input_tokens: i32,
    output_tokens: i32,
) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::ChatTurnCompleted,
        SourceSurface::Chat,
        serde_json::json!({
            "duration_ms": duration_ms,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        }),
    )
    .with_session(session_id)
}

/// A raw-mode terminal program has gone quiescent while holding input —
/// the "waiting for the user" signal (L2-observe, #400).
pub fn terminal_waiting_for_input(session_id: &str) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::TerminalWaitingForInput,
        SourceSurface::Terminal,
        serde_json::json!({}),
    )
    .with_session(session_id)
}

/// The PTY's foreground process exited (L2-observe, #400). `exit_code` is
/// `None` when the platform did not surface a status.
pub fn terminal_process_exited(session_id: &str, exit_code: Option<i32>) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::TerminalProcessExited,
        SourceSurface::Terminal,
        serde_json::json!({ "exit_code": exit_code }),
    )
    .with_session(session_id)
}

pub fn automation_job_started(job_id: &str, job_name: &str) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::AutomationJobStarted,
        SourceSurface::Scheduler,
        serde_json::json!({
            "job_id": job_id,
            "job_name": job_name,
        }),
    )
}

pub fn automation_job_completed(
    job_id: &str,
    job_name: &str,
    session_id: &str,
    duration_ms: u64,
    message_count: usize,
) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::AutomationJobCompleted,
        SourceSurface::Scheduler,
        serde_json::json!({
            "job_id": job_id,
            "job_name": job_name,
            "session_id": session_id,
            "duration_ms": duration_ms,
            "message_count": message_count,
        }),
    )
}

pub fn automation_job_failed(job_id: &str, job_name: &str, error: &str) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::AutomationJobFailed,
        SourceSurface::Scheduler,
        serde_json::json!({
            "job_id": job_id,
            "job_name": job_name,
            "error": error,
        }),
    )
}

pub fn starter_recipe_upgraded(
    starter_id: &str,
    from_version: &str,
    to_version: &str,
) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::StarterRecipeUpgraded,
        SourceSurface::Scheduler,
        serde_json::json!({
            "starter_id": starter_id,
            "from_version": from_version,
            "to_version": to_version,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_event_serializes_correctly() {
        let event = chat_turn_started("test-session");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event_type\":\"chat_turn_started\""));
        assert!(json.contains("\"source_surface\":\"chat\""));
        assert!(json.contains("\"tier\":\"ephemeral\""));
        assert!(json.contains("\"session_id\":\"test-session\""));
    }

    #[test]
    fn chat_turn_completed_is_always_tier() {
        let event = chat_turn_completed("s1", 100, 50, 25);
        assert_eq!(event.tier, EventTier::Always);
    }

    #[test]
    fn emit_activity_populates_ring_buffer() {
        let event = chat_turn_started("buf-test");
        emit_activity(event);
        let recent = recent_activity(10);
        assert!(recent
            .iter()
            .any(|e| e.session_id.as_deref() == Some("buf-test")));
    }

    #[test]
    fn terminal_observe_signals_are_ephemeral_and_reach_the_bus() {
        // L2-observe (#400): the new semantic signals must be Ephemeral (never
        // persisted) and must reach a bus consumer via the ring buffer.
        let waiting = terminal_waiting_for_input("sess-wait");
        assert_eq!(waiting.tier, EventTier::Ephemeral);
        assert_eq!(
            waiting.event_type,
            ActivityEventType::TerminalWaitingForInput
        );

        let exited = terminal_process_exited("sess-exit", Some(0));
        assert_eq!(exited.tier, EventTier::Ephemeral);
        assert_eq!(
            exited.payload.get("exit_code").and_then(|v| v.as_i64()),
            Some(0)
        );

        emit_activity(waiting);
        emit_activity(exited);
        let recent = recent_activity(20);
        assert!(recent
            .iter()
            .any(|e| e.event_type == ActivityEventType::TerminalWaitingForInput));
        assert!(recent
            .iter()
            .any(|e| e.event_type == ActivityEventType::TerminalProcessExited));
    }
}
