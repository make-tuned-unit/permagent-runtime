//! ContextBuilder — maintains live activity state and produces per-turn digests.
//!
//! Subscribes to the activity event bus alongside the [`ActivityIngester`].
//! As events flow in, updates a live state snapshot. When `current_digest()`
//! is called, assembles a [`Digest`] containing recent events, live state,
//! probed memories from Brain recognition, and optionally recalled memories.

use crate::events::activity::{ActivityEvent, ActivityEventType};
use spectral::{Brain, ProbeOpts, ProbeWindow, RecognizedMemory, Visibility};
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

        let probed_memories = if opts.include_probe {
            let window = ProbeWindow::Duration(chrono::Duration::from_std(opts.event_window)?);
            let probe_opts = ProbeOpts {
                max_results: opts.max_probe_results.unwrap_or(10),
                min_relevance: opts.min_probe_relevance.unwrap_or(0.3),
                wing_filter: opts.focus_wing.clone(),
            };

            match self.brain.probe_recent(window, probe_opts) {
                Ok(mut memories) => {
                    // Sort by relevance descending — highest scores first
                    memories.sort_by(|a, b| {
                        b.relevance
                            .partial_cmp(&a.relevance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    memories
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagent::activity::context_builder",
                        "probe_recent failed: {}",
                        e
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

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
    pub include_probe: bool,
    pub max_probe_results: Option<usize>,
    pub min_probe_relevance: Option<f64>,
    pub focus_wing: Option<String>,
    pub include_recall_query: Option<String>,
}

impl Default for DigestOpts {
    fn default() -> Self {
        Self {
            event_window: Duration::from_secs(600), // 10 minutes
            max_recent_events: 50,
            include_probe: false,
            max_probe_results: None,
            min_probe_relevance: None,
            focus_wing: None,
            include_recall_query: None,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Digest {
    pub live_state: LiveState,
    pub recent_events: Vec<ActivityEvent>,
    pub probed_memories: Vec<RecognizedMemory>,
    pub recalled_memories: Vec<RecalledMemory>,
}

#[derive(Debug, serde::Serialize)]
pub struct RecalledMemory {
    pub content: String,
    pub signal_score: f64,
    pub source: Option<String>,
}

/// Render a Digest into the `<ambient_context>` system prompt block.
/// Sections with no content are omitted.
pub fn render_ambient_context(digest: &Digest) -> String {
    let mut parts = Vec::new();

    // <live_state>
    let ls = &digest.live_state;
    let mut live_lines = Vec::new();
    if let Some(ref pid) = ls.active_project_id {
        live_lines.push(format!("You are currently working in: {} (project:{}).",
            pid.strip_prefix("project:").unwrap_or(pid), pid.strip_prefix("project:").unwrap_or(pid)));
    }
    if let Some(ref cmd) = ls.last_terminal_command {
        live_lines.push(format!("Recent terminal command: {}.", cmd));
    }
    if let Some(ref url) = ls.last_browser_url {
        live_lines.push(format!("Last browser page: {}.", url));
    }
    if ls.events_in_last_5_minutes > 0 {
        live_lines.push(format!("Activity in last 5 minutes: {} events.", ls.events_in_last_5_minutes));
    }
    if !live_lines.is_empty() {
        parts.push(format!("<live_state>\n{}\n</live_state>", live_lines.join("\n")));
    }

    // <recent_activity>
    if !digest.recent_events.is_empty() {
        let mut lines = Vec::new();
        for event in digest.recent_events.iter().rev().take(20) {
            let time = event.timestamp.format("%H:%M");
            let summary = render_event_summary(event);
            lines.push(format!("- {} {}", time, summary));
        }
        parts.push(format!("<recent_activity>\n{}\n</recent_activity>", lines.join("\n")));
    }

    // <recognized_memories>
    if !digest.probed_memories.is_empty() {
        let mut lines = vec!["The following memories from your past activity may be relevant:".to_string()];
        for mem in &digest.probed_memories {
            let wing_str = mem.wing.as_deref().unwrap_or("general");
            lines.push(format!("- \"{}\" (relevance: {:.2}, wing: {})",
                mem.content.chars().take(200).collect::<String>(), mem.relevance, wing_str));
        }
        parts.push(format!("<recognized_memories>\n{}\n</recognized_memories>", lines.join("\n")));
    }

    // <relevant_memories>
    if !digest.recalled_memories.is_empty() {
        let mut lines = vec!["Recalled memories relevant to this query:".to_string()];
        for mem in &digest.recalled_memories {
            lines.push(format!("- \"{}\" (score: {:.2})",
                mem.content.chars().take(200).collect::<String>(), mem.signal_score));
        }
        parts.push(format!("<relevant_memories>\n{}\n</relevant_memories>", lines.join("\n")));
    }

    if parts.is_empty() {
        return String::new();
    }

    format!("<ambient_context>\n{}\n</ambient_context>", parts.join("\n\n"))
}

fn render_event_summary(event: &ActivityEvent) -> String {
    match event.event_type {
        ActivityEventType::FileEdited => {
            let path = event.payload.get("file_path").and_then(|v| v.as_str()).unwrap_or("unknown");
            let lines = event.payload.get("lines_changed").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Edited file {} ({} lines)", path, lines)
        }
        ActivityEventType::TerminalCommandStarted => {
            let cmd = event.payload.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Ran '{}'", cmd)
        }
        ActivityEventType::TerminalCommandCompleted => {
            let cmd = event.payload.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let exit = event.payload.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let dur = event.payload.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Ran '{}' — exit {}, took {}ms", cmd, exit, dur)
        }
        ActivityEventType::BrowserNavigated => {
            let url = event.payload.get("url").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Navigated to {}", url)
        }
        ActivityEventType::ProjectSelected => {
            let name = event.payload.get("project_name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Started working in project {}", name)
        }
        ActivityEventType::ChatTurnStarted => "Chat turn started".to_string(),
        ActivityEventType::ChatTurnCompleted => {
            let dur = event.payload.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Chat turn completed ({}ms)", dur)
        }
        _ => format!("{:?}", event.event_type),
    }
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

    #[test]
    fn probe_results_sorted_by_relevance_descending() {
        let brain = build_test_brain();

        // Seed a few activity memories with different content
        brain
            .remember_with(
                "activity:1:test:aaa",
                "First activity memory about wing classification",
                spectral::RememberOpts {
                    source: Some("permagent.activity".into()),
                    visibility: spectral::Visibility::Private,
                    wing: Some("permagent".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        brain
            .remember_with(
                "activity:2:test:bbb",
                "Second activity memory about terminal commands and testing",
                spectral::RememberOpts {
                    source: Some("permagent.activity".into()),
                    visibility: spectral::Visibility::Private,
                    wing: Some("permagent".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let cb = ContextBuilder::new(brain);

        // Inject events so probe_recent has something to synthesize context from
        for _ in 0..3 {
            let event = make_event(
                ActivityEventType::ChatTurnCompleted,
                SourceSurface::Chat,
                EventTier::Always,
                serde_json::json!({"duration_ms": 500, "input_tokens": 10, "output_tokens": 5}),
            );
            cb.handle_event(&event);
        }

        let digest = cb
            .current_digest(DigestOpts {
                include_probe: true,
                min_probe_relevance: Some(0.0), // accept all
                ..Default::default()
            })
            .unwrap();

        // If probe returned results, they should be sorted by relevance descending
        if digest.probed_memories.len() >= 2 {
            for w in digest.probed_memories.windows(2) {
                assert!(
                    w[0].relevance >= w[1].relevance,
                    "Expected sorted descending: {} >= {}",
                    w[0].relevance,
                    w[1].relevance
                );
            }
        }
    }

    #[test]
    fn probe_wing_filter_passes_through() {
        let brain = build_test_brain();

        // Seed memories in two different wings
        brain
            .remember_with(
                "activity:1:test:wing_a",
                "Memory in permagent wing about Rust code",
                spectral::RememberOpts {
                    source: Some("permagent.activity".into()),
                    visibility: spectral::Visibility::Private,
                    wing: Some("permagent".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        brain
            .remember_with(
                "activity:2:test:wing_b",
                "Memory in get-ladle wing about recipes",
                spectral::RememberOpts {
                    source: Some("permagent.activity".into()),
                    visibility: spectral::Visibility::Private,
                    wing: Some("get-ladle".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let cb = ContextBuilder::new(brain);

        for _ in 0..3 {
            let event = make_event(
                ActivityEventType::ChatTurnCompleted,
                SourceSurface::Chat,
                EventTier::Always,
                serde_json::json!({"duration_ms": 500, "input_tokens": 10, "output_tokens": 5}),
            );
            cb.handle_event(&event);
        }

        let digest = cb
            .current_digest(DigestOpts {
                include_probe: true,
                focus_wing: Some("permagent".into()),
                min_probe_relevance: Some(0.0),
                ..Default::default()
            })
            .unwrap();

        // All probed memories should be in the permagent wing (or empty if probe found nothing)
        for mem in &digest.probed_memories {
            assert_eq!(
                mem.wing.as_deref(),
                Some("permagent"),
                "Expected wing 'permagent', got {:?} for key {}",
                mem.wing,
                mem.key
            );
        }
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
