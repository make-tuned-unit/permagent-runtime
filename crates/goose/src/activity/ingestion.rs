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

use crate::brain_handle::SafeBrain;
use crate::events::activity::{ActivityEvent, ActivityEventType, EventTier};
use spectral::ingest::CompactionTier;
use spectral::{DeviceId, RememberOpts, Visibility};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use tracing::{debug, error, warn};

/// Tracks the user's currently-active project for wing classification.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ActiveProject {
    pub project_id: String,
    pub project_name: String,
    pub wing: String,
}

/// Hard-block patterns for ad/tracking domains in content.
const AD_TRACKING_PATTERNS: &[&str] = &[
    "doubleclick",
    "crwdcntrl",
    "recaptcha",
    "ogs.google.com",
    "googleads",
    "adtrafficquality",
    "/ads/",
    "/tracking/",
];

/// Determine whether an activity event should be ingested into Brain.
///
/// Returns `false` for:
/// - content containing "about:blank"
/// - content matching ad/tracking patterns
/// - chat_turn_completed events (token-count noise)
/// - very short browser_navigated content (< 20 chars)
fn should_ingest_activity(content: &str, activity_type: &str) -> bool {
    // about:blank
    if content.contains("about:blank") {
        debug!(
            target: "permagent::activity::filter",
            activity_type = %activity_type,
            "Rejected: about:blank"
        );
        return false;
    }

    // Ad/tracking patterns
    let content_lower = content.to_lowercase();
    for pattern in AD_TRACKING_PATTERNS {
        if content_lower.contains(pattern) {
            debug!(
                target: "permagent::activity::filter",
                activity_type = %activity_type,
                pattern = %pattern,
                "Rejected: ad/tracking pattern"
            );
            return false;
        }
    }

    // chat_turn_completed is pure noise (token counts)
    if activity_type == "chat_turn_completed" {
        debug!(
            target: "permagent::activity::filter",
            "Rejected: chat_turn_completed"
        );
        return false;
    }

    // Very short browser_navigated content
    if activity_type == "browser_navigated" && content.len() < 20 {
        debug!(
            target: "permagent::activity::filter",
            content_len = content.len(),
            "Rejected: browser_navigated too short"
        );
        return false;
    }

    true
}

/// Extract domain from rendered browser_navigated content.
///
/// Content format: "Navigated to <title> (<url>) in tab <id>."
/// We extract the domain from the URL between parentheses.
fn extract_domain_from_content(content: &str) -> Option<String> {
    // Find URL in parentheses: "(<url>)"
    let start = content.find("(http")?;
    let url_start = start + 1;
    let rest = content.get(url_start..)?;
    let end = rest.find(')')?;
    let url = rest.get(..end)?;

    // Extract domain: skip scheme, take until '/' or end
    let after_scheme = url.find("://").and_then(|pos| url.get(pos + 3..))?;

    let domain_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let domain = after_scheme.get(..domain_end)?;

    if domain.is_empty() {
        None
    } else {
        Some(domain.to_lowercase())
    }
}

/// Check if a memory with the same domain was ingested within the last 24 hours.
/// Uses direct SQLite query against the Brain's memory.db.
fn domain_seen_recently(domain: &str) -> bool {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let conn = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let pattern = format!("%{}%", domain);
    let count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM memories \
             WHERE key LIKE 'activity:%browser_navigated%' \
               AND content LIKE ?1 \
               AND created_at > datetime('now', '-24 hours')",
            rusqlite::params![pattern],
            |r| r.get(0),
        )
        .unwrap_or(0);

    count > 0
}

pub struct ActivityIngester {
    brain: SafeBrain,
    device_id: DeviceId,
    failure_count: AtomicU64,
    always_count: AtomicU64,
    aggregated_count: AtomicU64,
    ephemeral_count: AtomicU64,
    /// Events rejected by the ingest filter (noise, ad/tracking, etc.)
    filtered_count: AtomicU64,
    /// Events deduplicated by domain within the 24h window.
    deduped_count: AtomicU64,
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
    pub fn new(brain: SafeBrain, device_id: String) -> Self {
        Self {
            brain,
            device_id: DeviceId::from_descriptor(&device_id),
            failure_count: AtomicU64::new(0),
            always_count: AtomicU64::new(0),
            aggregated_count: AtomicU64::new(0),
            ephemeral_count: AtomicU64::new(0),
            filtered_count: AtomicU64::new(0),
            deduped_count: AtomicU64::new(0),
            last_ingested_at: Mutex::new(None),
            aggregation_queue: Mutex::new(Vec::new()),
            active_project: RwLock::new(None),
            paused: AtomicBool::new(false),
        }
    }

    pub fn handle_event_blocking(&self, event: &ActivityEvent) {
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
            .or_else(|| event.payload.get("project_id").and_then(|v| v.as_str()));

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
            event.event_id.get(..8).unwrap_or(&event.event_id),
        );
        let content = render_content(event);

        let device_id = self.device_id;
        let event_type_name = event_type_str(&event.event_type).to_string();
        let source_surface = format!("{:?}", event.source_surface).to_lowercase();
        let is_aggregated = event.tier == EventTier::Aggregated;

        // ── Ingest filter: reject noise before writing to Brain ──
        if !should_ingest_activity(&content, &event_type_name) {
            self.filtered_count.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // ── Domain dedup: skip browser_navigated if same domain seen in last 24h ──
        if event.event_type == ActivityEventType::BrowserNavigated {
            if let Some(domain) = extract_domain_from_content(&content) {
                if domain_seen_recently(&domain) {
                    debug!(
                        target: "permagent::activity::filter",
                        domain = %domain,
                        "Rejected: domain seen within 24h"
                    );
                    self.deduped_count.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
        }

        let wing_override: Option<String> = self
            .active_project
            .read()
            .ok()
            .and_then(|ap| ap.as_ref().map(|p| p.wing.clone()));

        // Called from spawn_blocking context (state.rs activity event loop).
        let result = self.brain.raw_blocking_handle().remember_with(
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

    pub fn filtered_count(&self) -> u64 {
        self.filtered_count.load(Ordering::Relaxed)
    }

    pub fn deduped_count(&self) -> u64 {
        self.deduped_count.load(Ordering::Relaxed)
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
        ActivityEventType::StarterRecipeUpgraded => "starter_recipe_upgraded",
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
            let err = p
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
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
        s.get(..max).unwrap_or(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::activity::{ActivityEvent, ActivityEventType, EventTier, SourceSurface};

    // Brain-dependent tests moved to crates/permagent-brain-tests/ (issue #190).

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
        assert_eq!(
            derive_wing_slug("project:permagent"),
            Some("permagent".into())
        );
    }

    #[test]
    fn wing_slug_from_project_with_hyphens() {
        assert_eq!(
            derive_wing_slug("project:get-ladle"),
            Some("get-ladle".into())
        );
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

    // Brain-dependent tests (active_project_*, wing_override_*, Brain write tests)
    // moved to crates/permagent-brain-tests/ (issue #190).

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

    // ── ingest filter tests ──

    #[test]
    fn filter_blocks_about_blank() {
        assert!(!should_ingest_activity(
            "Navigated to about:blank",
            "browser_navigated"
        ));
    }

    #[test]
    fn filter_blocks_ad_tracking() {
        assert!(!should_ingest_activity(
            "Navigated to Ad (https://doubleclick.net/ad) in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to X (https://crwdcntrl.net/px) in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to reCAPTCHA in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to (https://ogs.google.com/u/0) in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to (https://googleads.g.doubleclick.net) in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to (https://www.google.com/ads/foo) in tab t1.",
            "browser_navigated"
        ));
        assert!(!should_ingest_activity(
            "Navigated to (https://example.com/tracking/pixel) in tab t1.",
            "browser_navigated"
        ));
    }

    #[test]
    fn filter_blocks_chat_turn_completed() {
        assert!(!should_ingest_activity(
            "Chat turn completed in 500ms (100 input tokens, 50 output tokens).",
            "chat_turn_completed"
        ));
    }

    #[test]
    fn filter_blocks_short_browser_navigated() {
        assert!(!should_ingest_activity("Nav to x.", "browser_navigated"));
    }

    #[test]
    fn filter_allows_normal_navigation() {
        assert!(should_ingest_activity(
            "Navigated to GitHub (https://github.com/permagent) in tab t1.",
            "browser_navigated"
        ));
    }

    #[test]
    fn filter_allows_terminal_command() {
        assert!(should_ingest_activity(
            "Ran 'cargo build' in /home/user/project. Exit code 0, took 5000ms. Output: ''.",
            "terminal_command_completed"
        ));
    }

    // ── domain extraction tests ──

    #[test]
    fn extract_domain_from_navigation_content() {
        let content = "Navigated to GitHub (https://github.com/permagent) in tab t1.";
        assert_eq!(
            extract_domain_from_content(content),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn extract_domain_with_path() {
        let content = "Navigated to Gmail (https://mail.google.com/mail/u/0) in tab t1.";
        assert_eq!(
            extract_domain_from_content(content),
            Some("mail.google.com".to_string())
        );
    }

    #[test]
    fn extract_domain_no_url() {
        let content = "Navigated to local page in tab t1.";
        assert_eq!(extract_domain_from_content(content), None);
    }
}
