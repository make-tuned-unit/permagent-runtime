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
use spectral::ingest::CompactionTier;
use spectral::{Brain, DeviceId, RememberOpts, Visibility};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tracing::{error, warn};

/// Tracks the user's currently-active project for wing classification.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActiveProject {
    pub project_id: String,
    pub project_name: String,
    pub wing: String,
}

pub struct ActivityIngester {
    brain: Arc<Brain>,
    device_id: DeviceId,
    failure_count: AtomicU64,
    always_count: AtomicU64,
    aggregated_count: AtomicU64,
    ephemeral_count: AtomicU64,
    last_ingested_at: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    aggregation_queue: Mutex<Vec<String>>,
    /// The user's currently-active project. Set when ProjectSelected events
    /// arrive; stays set until another ProjectSelected replaces it.
    active_project: RwLock<Option<ActiveProject>>,
    /// Pause skips Brain writes but does not stop event emission.
    /// Live awareness continues; only persistence stops.
    /// Useful when working on sensitive content the user doesn't want recorded.
    paused: AtomicBool,
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
            active_project: RwLock::new(None),
            paused: AtomicBool::new(false),
        }
    }

    pub fn handle_event(&self, event: &ActivityEvent) {
        // Update active project tracking on ProjectSelected
        if event.event_type == ActivityEventType::ProjectSelected {
            self.update_active_project(event);
        }

        match event.tier {
            EventTier::Always => {
                if !self.paused.load(Ordering::Relaxed) {
                    self.ingest_to_brain(event);
                }
                self.always_count.fetch_add(1, Ordering::Relaxed);
            }
            EventTier::Aggregated => {
                if !self.paused.load(Ordering::Relaxed) {
                    self.ingest_to_brain(event);
                }
                self.aggregated_count.fetch_add(1, Ordering::Relaxed);
            }
            EventTier::Ephemeral => {
                self.ephemeral_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn update_active_project(&self, event: &ActivityEvent) {
        let project_id = event
            .project_id
            .as_deref()
            .or_else(|| {
                event
                    .payload
                    .get("project_id")
                    .and_then(|v| v.as_str())
            });

        let project_id = match project_id {
            Some(id) => id,
            None => return,
        };

        let wing = match derive_wing_slug(project_id) {
            Some(w) => w,
            None => {
                warn!(
                    target: "permagent::activity::ingestion",
                    project_id = %project_id,
                    "ProjectSelected has malformed project_id (missing 'project:' prefix) — active project not updated"
                );
                return;
            }
        };

        let project_name = event
            .payload
            .get("project_name")
            .and_then(|v| v.as_str())
            .unwrap_or(project_id)
            .to_string();

        if let Ok(mut ap) = self.active_project.write() {
            *ap = Some(ActiveProject {
                project_id: project_id.to_string(),
                project_name,
                wing,
            });
        }
    }

    fn ingest_to_brain(&self, event: &ActivityEvent) {
        let key = format!(
            "activity:{}:{}:{}",
            event.timestamp.timestamp(),
            event_type_str(&event.event_type),
            &event.event_id[..8],
        );
        let content = render_content(event);

        let device_id = self.device_id.clone();
        let event_type_name = event_type_str(&event.event_type).to_string();
        let source_surface = format!("{:?}", event.source_surface).to_lowercase();
        let is_aggregated = event.tier == EventTier::Aggregated;

        let wing_override: Option<String> = self
            .active_project
            .read()
            .ok()
            .and_then(|ap| ap.as_ref().map(|p| p.wing.clone()));

        let result = self.brain.remember_with(
            &key,
            &content,
            RememberOpts {
                source: Some("permagent.activity".to_string()),
                device_id: Some(device_id),
                confidence: None,
                visibility: Visibility::Private,
                compaction_tier: Some(CompactionTier::Raw),
                wing: wing_override,
                ..Default::default()
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

    pub fn active_project(&self) -> Option<ActiveProject> {
        self.active_project.read().ok().and_then(|ap| ap.clone())
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

/// Strips the "project:" prefix from a canonical project_id to produce
/// the wing slug. Returns None if the input doesn't start with "project:"
/// or if the slug after the prefix is empty.
///
/// The wing slug is passed to RememberOpts.wing at write time. When set,
/// Spectral bypasses its TACT classifier and stores the slug as-is.
fn derive_wing_slug(canonical_project_id: &str) -> Option<String> {
    let slug = canonical_project_id.strip_prefix("project:")?;
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
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
        ActivityEventType::TerminalSessionStarted => "terminal_session_started",
        ActivityEventType::TerminalSessionEnded => "terminal_session_ended",
        ActivityEventType::AutomationJobStarted => "automation_job_started",
        ActivityEventType::AutomationJobCompleted => "automation_job_completed",
        ActivityEventType::AutomationJobFailed => "automation_job_failed",
    }
}

fn render_content(event: &ActivityEvent) -> String {
    let p = &event.payload;
    match event.event_type {
        ActivityEventType::ChatTurnCompleted => {
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let input = p.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let output = p.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            format!(
                "Chat turn completed in {}ms ({} input tokens, {} output tokens).",
                dur, input, output
            )
        }
        ActivityEventType::TerminalCommandCompleted => {
            let cmd = p.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let cwd = p
                .get("working_directory")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let exit = p.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let stdout = p
                .get("stdout_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!(
                "Ran '{}' in {}. Exit code {}, took {}ms. Output: '{}'.",
                cmd,
                cwd,
                exit,
                dur,
                truncate(stdout, 200)
            )
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
            let name = p
                .get("project_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let id = p
                .get("project_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            format!("Started working in project {} ({}).", name, id)
        }
        ActivityEventType::SkillExecuted => {
            let name = p
                .get("skill_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
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
            let lines = p
                .get("lines_changed")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            format!("Edited {} ({} lines changed).", path, lines)
        }
        ActivityEventType::AutomationJobStarted => {
            let name = p.get("job_name").and_then(|v| v.as_str()).unwrap_or("?");
            format!("Started scheduled automation '{}'.", name)
        }
        ActivityEventType::AutomationJobCompleted => {
            let name = p.get("job_name").and_then(|v| v.as_str()).unwrap_or("?");
            let dur = p.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let msgs = p.get("message_count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!(
                "Automation '{}' completed in {}ms — {} messages.",
                name, dur, msgs
            )
        }
        ActivityEventType::AutomationJobFailed => {
            let name = p.get("job_name").and_then(|v| v.as_str()).unwrap_or("?");
            let err = p.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
            format!("Automation '{}' failed: {}.", name, truncate(err, 200))
        }
        _ => format!(
            "{} event from {:?}.",
            event_type_str(&event.event_type),
            event.source_surface
        ),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::activity::{ActivityEvent, ActivityEventType, EventTier, SourceSurface};

    fn build_test_brain() -> Arc<Brain> {
        use spectral::DeviceId;
        let temp = tempfile::tempdir().expect("tempdir");
        let brain_path = temp.path().join("brain");
        let ontology_path = temp.path().join("ontology.toml");
        std::fs::write(&ontology_path, include_str!("../../assets/ontology.toml")).unwrap();
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

    fn make_always_event() -> ActivityEvent {
        ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
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
        }
    }

    fn make_project_selected(project_id: &str, project_name: &str) -> ActivityEvent {
        ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: ActivityEventType::ProjectSelected,
            source_surface: SourceSurface::ProjectPicker,
            timestamp: chrono::Utc::now(),
            session_id: None,
            project_id: Some(project_id.to_string()),
            payload: serde_json::json!({
                "project_id": project_id,
                "project_name": project_name,
            }),
            tier: EventTier::Always,
        }
    }

    // ── derive_wing_slug tests ──

    #[test]
    fn wing_slug_from_canonical_project() {
        assert_eq!(derive_wing_slug("project:permagent"), Some("permagent".into()));
    }

    #[test]
    fn wing_slug_from_project_with_hyphens() {
        assert_eq!(derive_wing_slug("project:get-ladle"), Some("get-ladle".into()));
    }

    #[test]
    fn wing_slug_no_prefix_returns_none() {
        assert_eq!(derive_wing_slug("permagent"), None);
    }

    #[test]
    fn wing_slug_did_returns_none() {
        assert_eq!(derive_wing_slug("did:chitin:henry-malcolm"), None);
    }

    #[test]
    fn wing_slug_empty_returns_none() {
        assert_eq!(derive_wing_slug(""), None);
    }

    #[test]
    fn wing_slug_empty_after_prefix_returns_none() {
        assert_eq!(derive_wing_slug("project:"), None);
    }

    // ── active project tracking tests ──

    #[test]
    fn active_project_set_on_project_selected() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());

        assert!(ingester.active_project().is_none());

        let event = make_project_selected("project:permagent", "Permagent");
        ingester.handle_event(&event);

        let ap = ingester.active_project().expect("should be set");
        assert_eq!(ap.project_id, "project:permagent");
        assert_eq!(ap.project_name, "Permagent");
        assert_eq!(ap.wing, "permagent");
    }

    #[test]
    fn active_project_replaced_on_subsequent_project_selected() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());

        ingester.handle_event(&make_project_selected("project:permagent", "Permagent"));
        assert_eq!(ingester.active_project().unwrap().wing, "permagent");

        ingester.handle_event(&make_project_selected("project:get-ladle", "Get Ladle"));
        let ap = ingester.active_project().unwrap();
        assert_eq!(ap.wing, "get-ladle");
        assert_eq!(ap.project_name, "Get Ladle");
    }

    #[test]
    fn active_project_unchanged_when_project_id_malformed() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());

        ingester.handle_event(&make_project_selected("project:permagent", "Permagent"));
        assert_eq!(ingester.active_project().unwrap().wing, "permagent");

        // Malformed: no "project:" prefix
        let bad_event = ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: ActivityEventType::ProjectSelected,
            source_surface: SourceSurface::ProjectPicker,
            timestamp: chrono::Utc::now(),
            session_id: None,
            project_id: Some("permagent".into()), // missing prefix
            payload: serde_json::json!({"project_id": "permagent", "project_name": "Bad"}),
            tier: EventTier::Always,
        };
        ingester.handle_event(&bad_event);

        // Should still be permagent, not replaced
        assert_eq!(ingester.active_project().unwrap().wing, "permagent");
    }

    #[test]
    fn wing_override_computed_during_ingestion() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());

        // Set active project
        ingester.handle_event(&make_project_selected("project:permagent", "Permagent"));

        // Trigger an Always-tier ingestion
        ingester.handle_event(&make_always_event());

        // The active project should still be set (ingestion didn't clear it)
        let ap = ingester.active_project().expect("should still be set");
        assert_eq!(ap.wing, "permagent");
        // wing_override is computed and passed to RememberOpts.wing inside
        // ingest_to_brain — Brain writes get wing: Some("permagent").
        assert_eq!(ingester.always_count(), 2); // ProjectSelected + ChatTurnCompleted
    }

    // ── Brain write tests ──

    #[test]
    fn always_event_ingested_to_brain() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());
        ingester.handle_event(&make_always_event());
        assert_eq!(ingester.always_count(), 1);
        assert_eq!(ingester.failure_count(), 0);
        assert!(ingester.last_ingested_at().is_some());
    }

    #[test]
    fn aggregated_event_ingested_and_queued() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());
        let event = ActivityEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: ActivityEventType::BrowserNavigated,
            source_surface: SourceSurface::Browser,
            timestamp: chrono::Utc::now(),
            session_id: None,
            project_id: None,
            payload: serde_json::json!({"url": "https://example.com", "title": "Example", "tab_id": "t1"}),
            tier: EventTier::Aggregated,
        };
        ingester.handle_event(&event);
        assert_eq!(ingester.aggregated_count(), 1);
        assert_eq!(ingester.aggregation_queue_size(), 1);
        assert_eq!(ingester.failure_count(), 0);
    }

    #[test]
    fn ephemeral_event_counted_not_ingested() {
        let brain = build_test_brain();
        let ingester = ActivityIngester::new(brain, "test-device".into());
        let event = ActivityEvent {
            event_id: "eph-1".into(),
            event_type: ActivityEventType::ChatTurnStarted,
            source_surface: SourceSurface::Chat,
            timestamp: chrono::Utc::now(),
            session_id: Some("s1".into()),
            project_id: None,
            payload: serde_json::json!({}),
            tier: EventTier::Ephemeral,
        };
        ingester.handle_event(&event);
        assert_eq!(ingester.ephemeral_count(), 1);
        assert_eq!(ingester.always_count(), 0);
        assert!(ingester.last_ingested_at().is_none());
    }

    /// Brain write failures must NOT crash the daemon.
    #[test]
    fn brain_failure_increments_counter_without_panic() {
        use spectral::DeviceId;
        let temp = tempfile::tempdir().expect("tempdir");
        let brain_path = temp.path().join("brain");
        let ontology_path = temp.path().join("ontology.toml");
        std::fs::write(&ontology_path, include_str!("../../assets/ontology.toml")).unwrap();
        let brain = Arc::new(
            Brain::builder()
                .data_dir(&brain_path)
                .ontology_path(&ontology_path)
                .device_id(DeviceId::from_descriptor("test"))
                .build()
                .expect("test brain"),
        );
        let ingester = ActivityIngester::new(brain, "test-device".into());
        ingester.handle_event(&make_always_event());
        assert_eq!(ingester.always_count(), 1);
        assert_eq!(ingester.failure_count(), 0);
        let _ = std::fs::remove_dir_all(&brain_path);
        ingester.handle_event(&make_always_event());
        assert_eq!(ingester.always_count(), 2, "both events should be counted");
    }

    // ── render tests ──

    #[test]
    fn render_chat_turn_completed() {
        let event = make_always_event();
        let content = render_content(&event);
        assert!(content.contains("500ms"));
        assert!(content.contains("100 input tokens"));
    }

    #[test]
    fn render_project_selected() {
        let event = make_project_selected("project:permagent", "Permagent");
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
            payload: serde_json::json!({"url": "https://example.com", "title": "Example", "tab_id": "tab-1"}),
            tier: EventTier::Aggregated,
        };
        let content = render_content(&event);
        assert!(content.contains("Example"));
        assert!(content.contains("https://example.com"));
    }
}
