//! Brain-dependent tests for activity::ingestion.
//! Extracted from crates/goose/src/activity/ingestion.rs to avoid
//! V8 libc++abi symbol collision on Linux (issue #190).

mod common;

use permagent::activity::ingestion::ActivityIngester;
use permagent::events::activity::{ActivityEvent, ActivityEventType, EventTier, SourceSurface};
use spectral::Brain;
use std::sync::Arc;

fn make_terminal_event() -> ActivityEvent {
    ActivityEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        event_type: ActivityEventType::TerminalCommandCompleted,
        source_surface: SourceSurface::Terminal,
        timestamp: chrono::Utc::now(),
        session_id: Some("s1".into()),
        project_id: None,
        payload: serde_json::json!({
            "command": "cargo build",
            "working_directory": "/home/user/project",
            "exit_code": 0,
            "duration_ms": 5000,
            "stdout_summary": "Compiling...",
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

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

// ── active project tracking tests ──

#[test]
fn active_project_set_on_project_selected() {
    let brain = common::shared_ingestion_brain();
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
    let brain = common::shared_ingestion_brain();
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
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(brain, "test-device".into());

    ingester.handle_event(&make_project_selected("project:permagent", "Permagent"));
    assert_eq!(ingester.active_project().unwrap().wing, "permagent");

    let bad_event = ActivityEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        event_type: ActivityEventType::ProjectSelected,
        source_surface: SourceSurface::ProjectPicker,
        timestamp: chrono::Utc::now(),
        session_id: None,
        project_id: Some("permagent".into()),
        payload: serde_json::json!({"project_id": "permagent", "project_name": "Bad"}),
        tier: EventTier::Always,
    };
    ingester.handle_event(&bad_event);

    assert_eq!(ingester.active_project().unwrap().wing, "permagent");
}

#[test]
fn wing_override_computed_during_ingestion() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(brain, "test-device".into());

    ingester.handle_event(&make_project_selected("project:permagent", "Permagent"));
    ingester.handle_event(&make_terminal_event());

    let ap = ingester.active_project().expect("should still be set");
    assert_eq!(ap.wing, "permagent");
    assert_eq!(ingester.always_count(), 2);
}

// ── Brain write tests ──

#[test]
fn always_event_ingested_to_brain() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(brain, "test-device".into());
    ingester.handle_event(&make_terminal_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.failure_count(), 0);
    assert!(ingester.last_ingested_at().is_some());
}

#[test]
fn aggregated_event_ingested_and_queued() {
    let brain = common::shared_ingestion_brain();
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
    let brain = common::shared_ingestion_brain();
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
    let temp = tempfile::tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
    let brain = Arc::new(
        Brain::builder()
            .data_dir(&brain_path)
            .ontology_path(&ontology_path)
            .device_id(spectral::DeviceId::from_descriptor("test"))
            .build()
            .expect("test brain"),
    );
    let ingester = ActivityIngester::new(brain, "test-device".into());
    ingester.handle_event(&make_terminal_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.failure_count(), 0);
    let _ = std::fs::remove_dir_all(&brain_path);
    ingester.handle_event(&make_terminal_event());
    assert_eq!(ingester.always_count(), 2, "both events should be counted");
}

#[test]
fn chat_turn_completed_filtered_by_ingester() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(brain, "test-device".into());
    ingester.handle_event(&make_always_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.filtered_count(), 1);
    assert!(ingester.last_ingested_at().is_none());
}
