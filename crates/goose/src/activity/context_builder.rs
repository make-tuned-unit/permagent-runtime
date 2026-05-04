//! ContextBuilder — maintains live activity state and produces per-turn digests.
//!
//! Subscribes to the activity event bus alongside the [`ActivityIngester`].
//! As events flow in, updates a live state snapshot. When `current_digest()`
//! is called, assembles a [`Digest`] containing recent events, live state,
//! and optionally recalled memories.
//!
//! Phase 3a builds the module and exposes the API. Phase 3b wires its
//! output into the chat turn system prompt.

use crate::events::activity::{ActivityEvent, ActivityEventType};
use spectral::{Brain, Visibility};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock, Mutex};
use std::time::Duration;

const MAX_RING_BUFFER_SIZE: usize = 1000;

pub struct ContextBuilder {
    brain: Arc<Brain>,
    recent_events: Mutex<VecDeque<ActivityEvent>>,
    live_state: RwLock<LiveState>,
}

impl ContextBuilder {
    pub fn new(brain: Arc<Brain>) -> Self {
        Self {
            brain,
            recent_events: Mutex::new(VecDeque::with_capacity(MAX_RING_BUFFER_SIZE)),
            live_state: RwLock::new(LiveState::default()),
        }
    }

    /// Process an incoming activity event: update live state and add to ring buffer.
    pub fn handle_event(&self, event: &ActivityEvent) {
        // Update live state based on event type
        if let Ok(mut state) = self.live_state.write() {
            match event.event_type {
                ActivityEventType::BrowserNavigated => {
                    state.last_browser_url = event
                        .payload
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                ActivityEventType::TerminalCommandStarted => {
                    state.last_terminal_command = event
                        .payload
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    state.last_terminal_cwd = event
                        .payload
                        .get("working_directory")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                ActivityEventType::ProjectSelected => {
                    state.active_project_id = event.project_id.clone().or_else(|| {
                        event
                            .payload
                            .get("project_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    });
                }
                ActivityEventType::ChatTurnStarted => {
                    state.active_session_id = event.session_id.clone();
                }
                _ => {}
            }
        }

        // Add to ring buffer
        if let Ok(mut buf) = self.recent_events.lock() {
            buf.push_back(event.clone());
            while buf.len() > MAX_RING_BUFFER_SIZE {
                buf.pop_front();
            }
        }
    }

    /// Produce a digest of current activity context.
    pub fn current_digest(&self, opts: DigestOpts) -> anyhow::Result<Digest> {
        let cutoff = chrono::Utc::now() - chrono::Duration::from_std(opts.event_window)?;

        let recent_events: Vec<ActivityEvent> = self
            .recent_events
            .lock()
            .map(|buf| {
                buf.iter()
                    .filter(|e| e.timestamp >= cutoff)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Cap at max_recent_events, keeping most recent
        let recent_events = if recent_events.len() > opts.max_recent_events {
            recent_events[recent_events.len() - opts.max_recent_events..].to_vec()
        } else {
            recent_events
        };

        let live_state = self
            .live_state
            .read()
            .map(|s| {
                let five_min_ago =
                    chrono::Utc::now() - chrono::Duration::from_std(Duration::from_secs(300)).unwrap();
                let events_in_last_5 = self
                    .recent_events
                    .lock()
                    .map(|buf| buf.iter().filter(|e| e.timestamp >= five_min_ago).count())
                    .unwrap_or(0);
                LiveState {
                    events_in_last_5_minutes: events_in_last_5,
                    ..s.clone()
                }
            })
            .unwrap_or_default();

        // Phase 3b TODO: Wire Brain::probe(timeline_context) here.
        // Spectral exposes both probe(text, opts) and probe_recent(window, opts).
        // Phase 3b should use probe_recent with ProbeWindow::Duration(Duration::minutes(10)).
        let probed_memories: Vec<serde_json::Value> = Vec::new();

        // Recall if query provided
        let recalled_memories = if let Some(ref query) = opts.include_recall_query {
            let brain = self.brain.clone();
            let q = query.clone();
            match brain.recall(&q, Visibility::Private) {
                Ok(result) => result
                    .memory_hits
                    .into_iter()
                    .map(|hit| RecalledMemory {
                        content: hit.content,
                        signal_score: hit.signal_score,
                        source: hit.source,
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!(
                        target: "permagent::activity::context_builder",
                        "Recall failed for query '{}': {}",
                        query,
                        e
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        Ok(Digest {
            live_state,
            recent_events,
            probed_memories,
            recalled_memories,
        })
    }

    /// Number of events currently in the ring buffer.
    pub fn buffered_count(&self) -> usize {
        self.recent_events
            .lock()
            .map(|buf| buf.len())
            .unwrap_or(0)
    }

    /// Snapshot of current live state.
    pub fn live_state_snapshot(&self) -> LiveState {
        self.live_state
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveState {
    pub active_project_id: Option<String>,
    pub active_session_id: Option<String>,
    pub last_browser_url: Option<String>,
    pub last_terminal_command: Option<String>,
    pub last_terminal_cwd: Option<String>,
    pub events_in_last_5_minutes: usize,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            active_project_id: None,
            active_session_id: None,
            last_browser_url: None,
            last_terminal_command: None,
            last_terminal_cwd: None,
            events_in_last_5_minutes: 0,
        }
    }
}

pub struct DigestOpts {
    pub event_window: Duration,
    pub max_recent_events: usize,
    /// Phase 3b TODO: Wire Brain::probe(timeline_context) here.
    /// Until then, this field is unused.
    pub include_probe: bool,
    pub include_recall_query: Option<String>,
}

impl Default for DigestOpts {
    fn default() -> Self {
        Self {
            event_window: Duration::from_secs(600), // 10 minutes
            max_recent_events: 50,
            include_probe: false,
            include_recall_query: None,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Digest {
    pub live_state: LiveState,
    pub recent_events: Vec<ActivityEvent>,
    /// Phase 3b TODO: probed_memories will be populated by Brain::probe_recent().
    pub probed_memories: Vec<serde_json::Value>,
    pub recalled_memories: Vec<RecalledMemory>,
}

#[derive(Debug, serde::Serialize)]
pub struct RecalledMemory {
    pub content: String,
    pub signal_score: f64,
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::activity::{ActivityEvent, ActivityEventType, EventTier, SourceSurface};

    fn make_event(
        event_type: ActivityEventType,
        surface: SourceSurface,
        tier: EventTier,
        payload: serde_json::Value,
    ) -> ActivityEvent {
        ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type,
            source_surface: surface,
            timestamp: chrono::Utc::now(),
            session_id: Some("test-session".into()),
            project_id: None,
            payload,
            tier,
        }
    }

    #[test]
    fn live_state_tracks_browser_url() {
        let brain = build_test_brain();
        let cb = ContextBuilder::new(brain);

        let event = make_event(
            ActivityEventType::BrowserNavigated,
            SourceSurface::Browser,
            EventTier::Aggregated,
            serde_json::json!({"url": "https://example.com", "title": "Example", "tab_id": "t1"}),
        );
        cb.handle_event(&event);

        let state = cb.live_state_snapshot();
        assert_eq!(state.last_browser_url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn live_state_tracks_terminal_command() {
        let brain = build_test_brain();
        let cb = ContextBuilder::new(brain);

        let event = make_event(
            ActivityEventType::TerminalCommandStarted,
            SourceSurface::Terminal,
            EventTier::Ephemeral,
            serde_json::json!({"command": "ls -la", "working_directory": "/home"}),
        );
        cb.handle_event(&event);

        let state = cb.live_state_snapshot();
        assert_eq!(state.last_terminal_command.as_deref(), Some("ls -la"));
        assert_eq!(state.last_terminal_cwd.as_deref(), Some("/home"));
    }

    #[test]
    fn live_state_tracks_project_selection() {
        let brain = build_test_brain();
        let cb = ContextBuilder::new(brain);

        let mut event = make_event(
            ActivityEventType::ProjectSelected,
            SourceSurface::ProjectPicker,
            EventTier::Always,
            serde_json::json!({"project_id": "project:permagent", "project_name": "Permagent"}),
        );
        event.project_id = Some("project:permagent".into());
        cb.handle_event(&event);

        let state = cb.live_state_snapshot();
        assert_eq!(state.active_project_id.as_deref(), Some("project:permagent"));
    }

    #[test]
    fn ring_buffer_bounded() {
        let brain = build_test_brain();
        let cb = ContextBuilder::new(brain);

        for _ in 0..1100 {
            let event = make_event(
                ActivityEventType::ChatTurnStarted,
                SourceSurface::Chat,
                EventTier::Ephemeral,
                serde_json::json!({}),
            );
            cb.handle_event(&event);
        }

        assert_eq!(cb.buffered_count(), MAX_RING_BUFFER_SIZE);
    }

    #[test]
    fn current_digest_returns_recent_events() {
        let brain = build_test_brain();
        let cb = ContextBuilder::new(brain);

        for i in 0..5 {
            let mut event = make_event(
                ActivityEventType::ChatTurnCompleted,
                SourceSurface::Chat,
                EventTier::Always,
                serde_json::json!({"duration_ms": i * 100, "input_tokens": 10, "output_tokens": 5}),
            );
            event.session_id = Some(format!("s-{}", i));
            cb.handle_event(&event);
        }

        let digest = cb.current_digest(DigestOpts::default()).unwrap();
        assert_eq!(digest.recent_events.len(), 5);
        assert!(digest.probed_memories.is_empty());
        assert!(digest.recalled_memories.is_empty());
    }

    fn build_test_brain() -> Arc<Brain> {
        use spectral::DeviceId;
        let temp = tempfile::tempdir().expect("tempdir");
        let brain_path = temp.path().join("brain");
        let ontology_path = temp.path().join("ontology.toml");
        std::fs::write(&ontology_path, include_str!("../../assets/ontology.toml")).unwrap();

        // Leak the tempdir so it persists for the test duration
        let _ = Box::leak(Box::new(temp));

        Arc::new(
            Brain::builder()
                .data_dir(&brain_path)
                .ontology_path(&ontology_path)
                .device_id(DeviceId::from_descriptor("test"))
                .build()
                .expect("test brain"),
        )
    }
}
