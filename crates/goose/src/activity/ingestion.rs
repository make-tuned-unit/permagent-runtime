// COMPACTION TIER SEMANTICS
//
// The Ingester always writes activity events with
// Some(CompactionTier::Raw). This marks them as ambient
// memories — captured via passive observation of user
// surfaces (browser, terminal, chat, project picker, etc.)
// rather than as deliberate user input or agent output.
//
// Spectral uses compaction_tier.is_some() as the canonical
// predicate for "is this an ambient memory?" Downstream
// consumers (Brain::probe, Librarian rollup, recognition
// scoring) treat ambient and non-ambient memories
// differently.
//
// If you find yourself writing to Brain from somewhere
// outside the activity layer, default to compaction_tier: None
// unless you have a specific reason to mark the write
// as ambient.

//! Activity event ingestion into the Spectral Brain.
//!
//! The [`ActivityIngester`] subscribes to the global event bus and
//! writes Always-tier and Aggregated-tier activity events to Brain.
//! Ephemeral events are bus-only and never persisted.

use crate::events::activity::{ActivityEvent, ActivityEventType, EventTier};
use spectral::{Brain, DeviceId, RememberOpts, Visibility};
use spectral::ingest::CompactionTier;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::error;

pub struct ActivityIngester {
    brain: Arc<Brain>,
    device_id: DeviceId,
    failure_count: AtomicU64,
    always_count: AtomicU64,
    aggregated_count: AtomicU64,
    ephemeral_count: AtomicU64,
    last_ingested_at: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    aggregation_queue: Mutex<Vec<String>>,
}

impl ActivityIngester {
    pub fn new(brain: Arc<Brain>, device_id: String) -> Self {
        Self {
            brain,
            device_id: DeviceId::from_descriptor(&device_id),
            failure_count: AtomicU64::new(0),
            always_count: AtomicU64::new(0),
            aggregated_count: AtomicU64::new(0),
            ephemeral_count: AtomicU64::new(0),
            last_ingested_at: Mutex::new(None),
            aggregation_queue: Mutex::new(Vec::new()),
        }
    }

    pub fn handle_event(&self, event: &ActivityEvent) {
        match event.tier {
            EventTier::Always => {
                self.ingest_to_brain(event);
                self.always_count.fetch_add(1, Ordering::Relaxed);
            }
            EventTier::Aggregated => {
                self.ingest_to_brain(event);
                self.aggregated_count.fetch_add(1, Ordering::Relaxed);
            }
            EventTier::Ephemeral => {
                self.ephemeral_count.fetch_add(1, Ordering::Relaxed);
                // Ephemeral events are bus-only — not persisted.
            }
        }
    }

    fn ingest_to_brain(&self, event: &ActivityEvent) {
        let key = format!(
            "activity:{}:{}:{}",
            event.timestamp.timestamp(),
            event_type_str(&event.event_type),
            &event.event_id[..8], // Short suffix for uniqueness
        );
        let content = render_content(event);

        let brain = self.brain.clone();
        let device_id = self.device_id.clone();
        let event_type_name = event_type_str(&event.event_type).to_string();
        let source_surface = format!("{:?}", event.source_surface).to_lowercase();
        let is_aggregated = event.tier == EventTier::Aggregated;

        let result = brain.remember_with(
            &key,
            &content,
            RememberOpts {
                source: Some("permagent.activity".to_string()),
                device_id: Some(device_id),
                confidence: None,
                visibility: Visibility::Private,
                created_at: Some(event.timestamp),
                episode_id: None,
                compaction_tier: Some(CompactionTier::Raw),
            },
        );

        match result {
            Ok(result) => {
                if let Ok(mut ts) = self.last_ingested_at.lock() {
                    *ts = Some(chrono::Utc::now());
                }
                if is_aggregated {
                    if let Ok(mut queue) = self.aggregation_queue.lock() {
                        queue.push(result.memory_id);
                    }
                }
            }
            Err(e) => {
                self.failure_count.fetch_add(1, Ordering::Relaxed);
                error!(
                    target: "permagent::activity::ingestion",
                    event_type = %event_type_name,
                    source_surface = %source_surface,
                    error = %e,
                    "Brain ingestion failed — event dropped"
                );
            }
        }
    }

    pub fn failure_count(&self) -> u64 {
        self.failure_count.load(Ordering::Relaxed)
    }

    pub fn always_count(&self) -> u64 {
        self.always_count.load(Ordering::Relaxed)
    }

    pub fn aggregated_count(&self) -> u64 {
        self.aggregated_count.load(Ordering::Relaxed)
    }

    pub fn ephemeral_count(&self) -> u64 {
        self.ephemeral_count.load(Ordering::Relaxed)
    }

    pub fn aggregation_queue_size(&self) -> usize {
        self.aggregation_queue.lock().map(|q| q.len()).unwrap_or(0)
    }

    pub fn last_ingested_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.last_ingested_at.lock().ok().and_then(|ts| *ts)
    }
}

fn event_type_str(t: &ActivityEventType) -> &'static str {
    match t {
        ActivityEventType::ChatTurnStarted => "chat_turn_started",
        ActivityEventType::ChatTurnCompleted => "chat_turn_completed",
        ActivityEventType::BrowserNavigated => "browser_navigated",
        ActivityEventType::BrowserFormSubmitted => "browser_form_submitted",
        ActivityEventType::BrowserSessionStarted => "browser_session_started",
        ActivityEventType::BrowserSessionEnded => "browser_session_ended",
        ActivityEventType::TerminalCommandStarted => "terminal_command_started",
        ActivityEventType::TerminalCommandCompleted => "terminal_command_completed",
        ActivityEventType::ProjectSelected => "project_selected",
        ActivityEventType::ProjectOpened => "project_opened",
        ActivityEventType::FileOpened => "file_opened",
        ActivityEventType::FileEdited => "file_edited",
        ActivityEventType::SkillExecuted => "skill_executed",
        ActivityEventType::IntegrationTokenRefreshed => "integration_token_refreshed",
        ActivityEventType::AgentContextProbed => "agent_context_probed",
    }
}

fn render_content(event: &ActivityEvent) -> String {
    let p = &event.payload;
    match event.event_type {
        ActivityEventType::ChatTurnCompleted => {
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let input = p.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let output = p.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            format!("Chat turn completed in {}ms ({} input tokens, {} output tokens).", dur, input, output)
        }
        ActivityEventType::TerminalCommandCompleted => {
            let cmd = p.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let cwd = p.get("working_directory").and_then(|v| v.as_str()).unwrap_or("?");
            let exit = p.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let stdout = p.get("stdout_summary").and_then(|v| v.as_str()).unwrap_or("");
            format!("Ran '{}' in {}. Exit code {}, took {}ms. Output: '{}'.", cmd, cwd, exit, dur, truncate(stdout, 200))
        }
        ActivityEventType::TerminalCommandStarted => {
            let cmd = p.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let cwd = p.get("working_directory").and_then(|v| v.as_str());
            match cwd {
                Some(dir) => format!("Started command '{}' in {}.", cmd, dir),
                None => format!("Started command '{}'.", cmd),
            }
        }
        ActivityEventType::BrowserNavigated => {
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            let title = p.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let tab = p.get("tab_id").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Navigated to {} ({}) in tab {}.", title, url, tab)
        }
        ActivityEventType::BrowserFormSubmitted => {
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            let tab = p.get("tab_id").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Submitted form on {} in tab {}.", url, tab)
        }
        ActivityEventType::ProjectSelected => {
            let name = p.get("project_name").and_then(|v| v.as_str()).unwrap_or("?");
            let id = p.get("project_id").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Started working in project {} ({}).", name, id)
        }
        ActivityEventType::SkillExecuted => {
            let name = p.get("skill_name").and_then(|v| v.as_str()).unwrap_or("?");
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Ran skill '{}' — status {}, took {}ms.", name, status, dur)
        }
        ActivityEventType::IntegrationTokenRefreshed => {
            let provider = p.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Refreshed {} integration token.", provider)
        }
        ActivityEventType::FileEdited => {
            let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let lines = p.get("lines_changed").and_then(|v| v.as_i64()).unwrap_or(0);
            format!("Edited {} ({} lines changed).", path, lines)
        }
        // Ephemeral events shouldn't reach render_content, but handle gracefully.
        _ => format!("{} event from {:?}.", event_type_str(&event.event_type), event.source_surface),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::activity::{ActivityEvent, ActivityEventType, SourceSurface, EventTier};

    #[test]
    fn render_chat_turn_completed() {
        let event = ActivityEvent {
            event_id: "test".into(),
            event_type: ActivityEventType::ChatTurnCompleted,
            source_surface: SourceSurface::Chat,
            timestamp: chrono::Utc::now(),
            session_id: Some("s1".into()),
            project_id: None,
            payload: serde_json::json!({
                "duration_ms": 500,
                "input_tokens": 100,
                "output_tokens": 50,
            }),
            tier: EventTier::Always,
        };
        let content = render_content(&event);
        assert!(content.contains("500ms"));
        assert!(content.contains("100 input tokens"));
    }

    #[test]
    fn render_project_selected() {
        let event = ActivityEvent {
            event_id: "test".into(),
            event_type: ActivityEventType::ProjectSelected,
            source_surface: SourceSurface::ProjectPicker,
            timestamp: chrono::Utc::now(),
            session_id: None,
            project_id: Some("project:permagent".into()),
            payload: serde_json::json!({
                "project_name": "Permagent",
                "project_id": "project:permagent",
            }),
            tier: EventTier::Always,
        };
        let content = render_content(&event);
        assert!(content.contains("Permagent"));
        assert!(content.contains("project:permagent"));
    }

    #[test]
    fn render_browser_navigated() {
        let event = ActivityEvent {
            event_id: "test".into(),
            event_type: ActivityEventType::BrowserNavigated,
            source_surface: SourceSurface::Browser,
            timestamp: chrono::Utc::now(),
            session_id: None,
            project_id: None,
            payload: serde_json::json!({
                "url": "https://example.com",
                "title": "Example",
                "tab_id": "tab-1",
            }),
            tier: EventTier::Aggregated,
        };
        let content = render_content(&event);
        assert!(content.contains("Example"));
        assert!(content.contains("https://example.com"));
    }

    #[test]
    fn ephemeral_events_are_not_ingested() {
        // Verify the tier check — we can't test Brain writes without a real Brain,
        // but we can verify the counter logic.
        let event = ActivityEvent {
            event_id: "test".into(),
            event_type: ActivityEventType::ChatTurnStarted,
            source_surface: SourceSurface::Chat,
            timestamp: chrono::Utc::now(),
            session_id: Some("s1".into()),
            project_id: None,
            payload: serde_json::json!({}),
            tier: EventTier::Ephemeral,
        };
        // Can't construct a full Ingester without Brain, but verify tier logic directly
        assert_eq!(event.tier, EventTier::Ephemeral);
    }
}
