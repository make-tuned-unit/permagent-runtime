//! ContextBuilder — maintains live activity state and produces per-turn digests.
//!
//! Subscribes to the activity event bus alongside the [`ActivityIngester`].
//! As events flow in, updates a live state snapshot. When `current_digest_blocking()`
//! is called, assembles a [`Digest`] containing recent events, live state,
//! probed memories from Brain recognition, and optionally recalled memories.

use crate::brain_handle::SafeBrain;
use crate::events::activity::{ActivityEvent, ActivityEventType};
use spectral::{ProbeOpts, ProbeWindow, RecognizedMemory};
use std::collections::VecDeque;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

const MAX_RING_BUFFER_SIZE: usize = 1000;

pub struct ContextBuilder {
    brain: SafeBrain,
    recent_events: Mutex<VecDeque<ActivityEvent>>,
    live_state: RwLock<LiveState>,
}

impl ContextBuilder {
    pub fn new(brain: SafeBrain) -> Self {
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
    pub fn current_digest_blocking(&self, opts: DigestOpts) -> anyhow::Result<Digest> {
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
                let five_min_ago = chrono::Utc::now()
                    - chrono::Duration::from_std(Duration::from_secs(300)).unwrap();
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

            // Called from spawn_blocking context — use raw handle.
            match self
                .brain
                .raw_blocking_handle()
                .probe_recent(window, probe_opts)
            {
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
            let brain = self.brain.raw_blocking_handle();
            let q = query.clone();
            let recognition_ctx = spectral::graph::RecognitionContext::empty()
                .with_persona(crate::config::agent_identity::DEFAULT_PERSONA_KEY);
            // Called from spawn_blocking context — use raw handle.
            match brain.recall_cascade(&q, &recognition_ctx, &Default::default()) {
                Ok(result) => result
                    .merged_hits
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
        self.recent_events.lock().map(|buf| buf.len()).unwrap_or(0)
    }

    /// Snapshot of current live state.
    pub fn live_state_snapshot(&self) -> LiveState {
        self.live_state
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LiveState {
    pub active_project_id: Option<String>,
    pub active_session_id: Option<String>,
    pub last_browser_url: Option<String>,
    pub last_terminal_command: Option<String>,
    pub last_terminal_cwd: Option<String>,
    pub events_in_last_5_minutes: usize,
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

    // Build live_state first so we can compute freshness
    let ls = &digest.live_state;
    let is_fresh = ls.events_in_last_5_minutes > 0;
    let has_terminal = ls.last_terminal_cwd.is_some() || ls.last_terminal_command.is_some();
    let has_browser = ls.last_browser_url.is_some();

    // Preamble with unmissable signal
    parts.push(format!(
        "LIVE_ACTIVITY_AVAILABLE: true\n\
         ACTIVITY_CURRENT: {}\n\
         TERMINAL_VISIBLE: {}\n\
         BROWSER_VISIBLE: {}\n\n\
         You HAVE real-time access to the user's terminal, browser, and project activity. \
         The live_state below updates every time they run a command or navigate a page. \
         Always use it. Never say \"I can't see your terminal\" or suggest sharing screenshots. \
         When asked what they're working on, answer directly: state the project directory and recent commands. \
         Keep it short.",
        is_fresh, has_terminal, has_browser,
    ));

    // <live_state>
    let mut live_lines = Vec::new();
    if let Some(ref pid) = ls.active_project_id {
        live_lines.push(format!(
            "PROJECT: {}",
            pid.strip_prefix("project:").unwrap_or(pid)
        ));
    }
    if let Some(ref cwd) = ls.last_terminal_cwd {
        live_lines.push(format!("TERMINAL_CWD: {}", cwd));
    }
    if let Some(ref cmd) = ls.last_terminal_command {
        live_lines.push(format!("TERMINAL_LAST_COMMAND: {}", cmd));
    }
    if let Some(ref url) = ls.last_browser_url {
        live_lines.push(format!("BROWSER_URL: {}", url));
    }
    if ls.events_in_last_5_minutes > 0 {
        live_lines.push(format!("EVENTS_LAST_5MIN: {}", ls.events_in_last_5_minutes));
    }
    if !live_lines.is_empty() {
        parts.push(format!(
            "<live_state>\n{}\n</live_state>",
            live_lines.join("\n")
        ));
    }

    // <recent_activity>
    if !digest.recent_events.is_empty() {
        let mut lines = Vec::new();
        for event in digest.recent_events.iter().rev().take(20) {
            let time = event.timestamp.format("%H:%M");
            let summary = render_event_summary(event);
            lines.push(format!("- {} {}", time, summary));
        }
        parts.push(format!(
            "<recent_activity>\n{}\n</recent_activity>",
            lines.join("\n")
        ));
    }

    // <recognized_memories>
    if !digest.probed_memories.is_empty() {
        let mut lines =
            vec!["The following memories from your past activity may be relevant:".to_string()];
        for mem in &digest.probed_memories {
            let wing_str = mem.wing.as_deref().unwrap_or("general");
            lines.push(format!(
                "- \"{}\" (relevance: {:.2}, wing: {})",
                mem.content.chars().take(200).collect::<String>(),
                mem.relevance,
                wing_str
            ));
        }
        parts.push(format!(
            "<recognized_memories>\n{}\n</recognized_memories>",
            lines.join("\n")
        ));
    }

    // <relevant_memories>
    if !digest.recalled_memories.is_empty() {
        let mut lines = vec!["Recalled memories relevant to this query:".to_string()];
        for mem in &digest.recalled_memories {
            lines.push(format!(
                "- \"{}\" (score: {:.2})",
                mem.content.chars().take(200).collect::<String>(),
                mem.signal_score
            ));
        }
        parts.push(format!(
            "<relevant_memories>\n{}\n</relevant_memories>",
            lines.join("\n")
        ));
    }

    // The first element is always the preamble — only emit if there's
    // at least one data section beyond it.
    if parts.len() <= 1 {
        return String::new();
    }

    format!(
        "<ambient_context>\n{}\n</ambient_context>",
        parts.join("\n\n")
    )
}

fn render_event_summary(event: &ActivityEvent) -> String {
    match event.event_type {
        ActivityEventType::FileEdited => {
            let path = event
                .payload
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let lines = event
                .payload
                .get("lines_changed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("Edited file {} ({} lines)", path, lines)
        }
        ActivityEventType::TerminalCommandStarted => {
            let cmd = event
                .payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Ran '{}'", cmd)
        }
        ActivityEventType::TerminalCommandCompleted => {
            let cmd = event
                .payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let exit = event
                .payload
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            let dur = event
                .payload
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("Ran '{}' — exit {}, took {}ms", cmd, exit, dur)
        }
        ActivityEventType::BrowserNavigated => {
            let url = event
                .payload
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Navigated to {}", url)
        }
        ActivityEventType::ProjectSelected => {
            let name = event
                .payload
                .get("project_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Started working in project {}", name)
        }
        ActivityEventType::ChatTurnStarted => "Chat turn started".to_string(),
        ActivityEventType::ChatTurnCompleted => {
            let dur = event
                .payload
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("Chat turn completed ({}ms)", dur)
        }
        ActivityEventType::AutomationJobStarted => {
            let name = event
                .payload
                .get("job_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Automation '{}' started", name)
        }
        ActivityEventType::AutomationJobCompleted => {
            let name = event
                .payload
                .get("job_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Automation '{}' completed", name)
        }
        ActivityEventType::AutomationJobFailed => {
            let name = event
                .payload
                .get("job_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Automation '{}' failed", name)
        }
        ActivityEventType::TerminalWaitingForInput => {
            "A terminal session appears to be waiting for input".to_string()
        }
        ActivityEventType::TerminalProcessExited => {
            match event.payload.get("exit_code").and_then(|v| v.as_i64()) {
                Some(code) => format!("A terminal process exited (code {})", code),
                None => "A terminal process exited".to_string(),
            }
        }
        _ => format!("{:?}", event.event_type),
    }
}

// Brain-dependent tests have been moved to crates/permagent-brain-tests/
// to avoid V8 libc++abi symbol collision on Linux. See issue #190.
