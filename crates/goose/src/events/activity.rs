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
//!   AutomationJobCompleted, AutomationJobFailed, PersonaConfigured,
//!   DecisionResolved, DevicesPaired
//!
//! **Aggregated** (rolled up over time windows):
//!   BrowserNavigated, BrowserFormSubmitted, WebSearchPerformed
//!
//! **Ephemeral** (live-only, never persisted):
//!   ChatTurnStarted, BrowserSessionStarted, BrowserSessionEnded,
//!   FileOpened, ProjectOpened, TerminalCommandStarted,
//!   TerminalSessionStarted, TerminalSessionEnded, AgentContextProbed,
//!   AutomationJobStarted, TerminalProcessExited, DictationCompleted,
//!   PairingLinkCopied, WorldViewOpened, InboxOpened, GrowOpened, BrainOpened
//!
//! The onboarding-engagement events (persona/decision/device-pairing +
//! pairing-link-copy/web-search/dictation + the four `*Opened` navigations)
//! additionally feed feature-usage tracking via `emit_activity` →
//! `usage::record_from_activity`, regardless of tier — so a usage-only "open"
//! is Ephemeral (bus-only) yet still marks the feature learned.

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
    /// A goal's fix was auto-escalated to a stronger configured model after
    /// repeated verify failures (the live verifier-driven escalation). Ambient
    /// and non-blocking — surfaced for cost transparency ("no surprises"), never
    /// an interrupt; the goal keeps running on the stronger model.
    GoalEscalated,

    // ── Agent-led onboarding: genuine engagement signals for surfaces that
    // previously emitted nothing. Each evidences the user actually *using* a
    // capability (not an inferred proxy), so `capability_for_activity` can mark
    // it learned and the coach stops re-recommending it. Consumed by the same
    // `emit_activity` seam — no new bus.
    /// The user configured their agent's identity/persona (name / voice) in
    /// Settings → Identity. A deliberate act — evidences the `persona` capability.
    PersonaConfigured,
    /// The user resolved a Decision Inbox item (approve / reject / approve-with-
    /// edits). Emitted only on a *user* resolution — genuine engagement with the
    /// `decision_inbox` capability, never on a system-created decision.
    DecisionResolved,
    /// A companion device actually completed pairing with this hub: it opened
    /// the pairing URL, captured the daemon token from the `#token=` fragment
    /// on first load, and proved the credential works — the event rides the
    /// bearer-authenticated `POST /activity/emit`, so an invalid or rotated
    /// token can never mint it. Emitted from the served UI's token-capture
    /// seam (`ui/command-center/src/lib/api.ts`), once per newly captured
    /// credential per device (a history re-open of the same URL does not
    /// re-claim a pairing). Evidences the `devices` capability.
    DevicesPaired,
    /// The user copied the device-pairing URL on the hub (Settings → Devices).
    /// Deliberate engagement with the `devices` feature, but only *intent* —
    /// not a completed pairing (that is `DevicesPaired`, emitted by the new
    /// device itself) — so it stays Ephemeral and never becomes a Brain memory.
    PairingLinkCopied,
    /// The agent performed a web search on the user's behalf (the `web_search`
    /// tool ran). Evidences the `web_search` capability.
    WebSearchPerformed,
    /// The user completed a voice dictation (mic → transcript). Evidences the
    /// `voice` capability. Usage-only: the transcript reaches Brain via the
    /// resulting chat/notes turn, so this signal carries no content.
    DictationCompleted,
    /// The user opened the World view. Evidences the `world_view` surface.
    WorldViewOpened,
    /// The user opened the Inbox. Evidences the `inbox` surface.
    InboxOpened,
    /// The user opened the Grow tab. Evidences the `grow` surface.
    GrowOpened,
    /// The user opened the Brain view. Evidences the `brain` surface.
    BrainOpened,
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
    // Onboarding-engagement surfaces (previously silent). One per user-facing
    // view so the activity feed and awareness layer can attribute the signal.
    Settings,
    Dashboard,
    World,
    Inbox,
    Grow,
    Brain,
    Voice,
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
        | ActivityEventType::StarterRecipeUpgraded
        | ActivityEventType::GoalEscalated
        // Substantive engagement worth remembering (a config change, a user's
        // decision, a paired device).
        | ActivityEventType::PersonaConfigured
        | ActivityEventType::DecisionResolved
        | ActivityEventType::DevicesPaired => EventTier::Always,

        // High-frequency, rolled up over a window (web activity family).
        ActivityEventType::BrowserNavigated
        | ActivityEventType::BrowserFormSubmitted
        | ActivityEventType::WebSearchPerformed => EventTier::Aggregated,

        ActivityEventType::ChatTurnStarted
        | ActivityEventType::BrowserSessionStarted
        | ActivityEventType::BrowserSessionEnded
        | ActivityEventType::FileOpened
        | ActivityEventType::ProjectOpened
        | ActivityEventType::TerminalCommandStarted
        | ActivityEventType::TerminalSessionStarted
        | ActivityEventType::TerminalSessionEnded
        | ActivityEventType::TerminalProcessExited
        | ActivityEventType::AgentContextProbed
        | ActivityEventType::AutomationJobStarted
        // Usage-only navigation / dictation: mark the feature engaged (usage is
        // recorded in `emit_activity` regardless of tier) but keep the bus-only,
        // never-persisted contract so opens don't flood Brain and dictation does
        // not duplicate the transcript already captured by the chat/notes turn.
        // The pairing-link copy likewise marks `devices` engaged without ever
        // claiming a completed pairing (that is `DevicesPaired`, Always).
        | ActivityEventType::DictationCompleted
        | ActivityEventType::PairingLinkCopied
        | ActivityEventType::WorldViewOpened
        | ActivityEventType::InboxOpened
        | ActivityEventType::GrowOpened
        | ActivityEventType::BrainOpened => EventTier::Ephemeral,
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

    // Agent-led onboarding: derive feature-usage from the activity the real
    // surfaces already emit. We consume this existing stream — no new event is
    // fabricated — and map each signal to the capability it evidences.
    crate::agents::self_knowledge::usage::record_from_activity(
        &event.event_type,
        &event.source_surface,
    );

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

/// The ambient, non-blocking note emitted on each live verifier-driven
/// escalation (R3): goal X climbed tier `from`→`to` on a stronger configured
/// model after repeated verify failures, at the given running session spend.
/// Visible in the activity feed for cost transparency; it never gates or
/// interrupts the run (only a ceiling — spend cap / max-escalations / no stronger
/// tier — parks to the Decision Inbox).
pub fn goal_escalated(
    goal_id: &str,
    goal_title: &str,
    from_tier: Option<&str>,
    to_tier: &str,
    model: &str,
    session_spent_usd: f64,
) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::GoalEscalated,
        SourceSurface::Agent,
        serde_json::json!({
            "goal_id": goal_id,
            "goal_title": goal_title,
            "from_tier": from_tier,
            "to_tier": to_tier,
            "model": model,
            "session_spent_usd": session_spent_usd,
        }),
    )
}

/// The user's agent ran a web search (the `web_search` tool). Emitted from the
/// tool seam so "the user used web search" is a real engagement signal rather
/// than an inferred proxy. Aggregated (web-activity family): the query is kept
/// as ambient context, rolled up over a window. `web_search` has no user-facing
/// frontend surface, so — unlike the other onboarding signals — it is emitted
/// server-side where the tool actually executes.
pub fn web_search_performed(query: &str, backend: &str) -> ActivityEvent {
    ActivityEvent::new(
        ActivityEventType::WebSearchPerformed,
        SourceSurface::Agent,
        serde_json::json!({
            "query": query,
            "backend": backend,
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
    fn goal_escalated_is_an_always_visible_ambient_note() {
        // R3: the escalation note is visible (Always tier) but non-blocking — an
        // activity event, never a gating decision — and carries the tier climb +
        // spend for cost transparency.
        let event = goal_escalated(
            "goal-1",
            "Fix the parser",
            Some("local_free"),
            "cheap_cloud",
            "claude-haiku-4-5",
            0.42,
        );
        assert_eq!(event.event_type, ActivityEventType::GoalEscalated);
        assert_eq!(
            event.tier,
            EventTier::Always,
            "escalations are always surfaced"
        );
        assert_eq!(event.source_surface, SourceSurface::Agent);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event_type\":\"goal_escalated\""));
        assert!(json.contains("\"from_tier\":\"local_free\""));
        assert!(json.contains("\"to_tier\":\"cheap_cloud\""));
        assert!(json.contains("claude-haiku-4-5"));
        assert_eq!(
            event
                .payload
                .get("session_spent_usd")
                .and_then(|v| v.as_f64()),
            Some(0.42)
        );
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
    fn terminal_process_exited_is_ephemeral_and_reaches_the_bus() {
        // L2-observe (#400): the process-exit signal must be Ephemeral (never
        // persisted) and must reach a bus consumer via the ring buffer.
        let exited = terminal_process_exited("sess-exit", Some(0));
        assert_eq!(exited.tier, EventTier::Ephemeral);
        assert_eq!(
            exited.payload.get("exit_code").and_then(|v| v.as_i64()),
            Some(0)
        );

        emit_activity(exited);
        let recent = recent_activity(20);
        assert!(recent
            .iter()
            .any(|e| e.event_type == ActivityEventType::TerminalProcessExited));
    }

    #[test]
    fn onboarding_engagement_events_have_expected_tiers() {
        use ActivityEventType::*;
        // Substantive user engagement → persisted to Brain (Always).
        for ev in [PersonaConfigured, DecisionResolved, DevicesPaired] {
            assert_eq!(canonical_tier(&ev), EventTier::Always, "{ev:?} → Always");
        }
        // Web search rolls up like the browser family.
        assert_eq!(canonical_tier(&WebSearchPerformed), EventTier::Aggregated);
        // Opens + dictation + the pairing-link copy are usage-only (bus-only,
        // never persisted to Brain). In particular PairingLinkCopied MUST stay
        // Ephemeral: copying the URL is intent, not a completed pairing, and an
        // Always tier here would write a false "paired a device" Brain memory.
        for ev in [
            DictationCompleted,
            PairingLinkCopied,
            WorldViewOpened,
            InboxOpened,
            GrowOpened,
            BrainOpened,
        ] {
            assert_eq!(
                canonical_tier(&ev),
                EventTier::Ephemeral,
                "{ev:?} → Ephemeral"
            );
        }
    }

    #[test]
    fn web_search_performed_carries_query_and_backend() {
        let e = web_search_performed("rust async traits", "venice");
        assert_eq!(e.event_type, ActivityEventType::WebSearchPerformed);
        assert_eq!(e.source_surface, SourceSurface::Agent);
        assert_eq!(e.tier, EventTier::Aggregated);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event_type\":\"web_search_performed\""));
        assert!(json.contains("rust async traits"));
        assert!(json.contains("venice"));
    }

    /// The frontend emits these events/surfaces as snake_case strings through
    /// `/activity/emit`; lock the wire spelling so an emit site can never drift
    /// out of sync with the enum (a typo would silently deserialize-fail).
    #[test]
    fn onboarding_wire_spellings_are_stable() {
        for (ev, wire) in [
            (ActivityEventType::PersonaConfigured, "persona_configured"),
            (ActivityEventType::DecisionResolved, "decision_resolved"),
            (ActivityEventType::DevicesPaired, "devices_paired"),
            (ActivityEventType::PairingLinkCopied, "pairing_link_copied"),
            (
                ActivityEventType::WebSearchPerformed,
                "web_search_performed",
            ),
            (ActivityEventType::DictationCompleted, "dictation_completed"),
            (ActivityEventType::WorldViewOpened, "world_view_opened"),
            (ActivityEventType::InboxOpened, "inbox_opened"),
            (ActivityEventType::GrowOpened, "grow_opened"),
            (ActivityEventType::BrainOpened, "brain_opened"),
        ] {
            let json = serde_json::to_string(&ev).unwrap();
            assert_eq!(json, format!("\"{wire}\""), "{ev:?} wire spelling");
            let back: ActivityEventType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ev, "{wire} must round-trip");
        }
        for (surface, wire) in [
            (SourceSurface::Settings, "settings"),
            (SourceSurface::Dashboard, "dashboard"),
            (SourceSurface::World, "world"),
            (SourceSurface::Inbox, "inbox"),
            (SourceSurface::Grow, "grow"),
            (SourceSurface::Brain, "brain"),
            (SourceSurface::Voice, "voice"),
        ] {
            assert_eq!(
                serde_json::to_string(&surface).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }
}
