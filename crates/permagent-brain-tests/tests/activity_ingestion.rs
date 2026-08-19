//! Brain-dependent tests for activity::ingestion.
//! Extracted from crates/goose/src/activity/ingestion.rs to avoid
//! V8 libc++abi symbol collision on Linux (issue #190).

mod common;

// Sanctioned raw spectral::Brain usage — test crate owns its runtime.
use permagent::activity::ingestion::ActivityIngester;
use permagent::brain_handle::SafeBrain;
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
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());

    assert!(ingester.active_project().is_none());

    let event = make_project_selected("project:permagent", "Permagent");
    ingester.handle_event_blocking(&event);

    let ap = ingester.active_project().expect("should be set");
    assert_eq!(ap.project_id, "project:permagent");
    assert_eq!(ap.project_name, "Permagent");
    assert_eq!(ap.wing, "permagent");
}

#[test]
fn active_project_replaced_on_subsequent_project_selected() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));
    assert_eq!(ingester.active_project().unwrap().wing, "permagent");

    ingester.handle_event_blocking(&make_project_selected("project:get-ladle", "Get Ladle"));
    let ap = ingester.active_project().unwrap();
    assert_eq!(ap.wing, "get-ladle");
    assert_eq!(ap.project_name, "Get Ladle");
}

#[test]
fn active_project_unchanged_when_project_id_malformed() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));
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
    ingester.handle_event_blocking(&bad_event);

    assert_eq!(ingester.active_project().unwrap().wing, "permagent");
}

#[test]
fn wing_override_computed_during_ingestion() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));
    ingester.handle_event_blocking(&make_terminal_event());

    let ap = ingester.active_project().expect("should still be set");
    assert_eq!(ap.wing, "permagent");
    assert_eq!(ingester.always_count(), 2);
}

// ── Brain write tests ──

#[test]
fn always_event_ingested_to_brain() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
    ingester.handle_event_blocking(&make_terminal_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.failure_count(), 0);
    assert!(ingester.last_ingested_at().is_some());
}

#[test]
fn aggregated_event_ingested_and_queued() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
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
    ingester.handle_event_blocking(&event);
    assert_eq!(ingester.aggregated_count(), 1);
    assert_eq!(ingester.aggregation_queue_size(), 1);
    assert_eq!(ingester.failure_count(), 0);
}

#[test]
fn ephemeral_event_counted_not_ingested() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
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
    ingester.handle_event_blocking(&event);
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
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
    ingester.handle_event_blocking(&make_terminal_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.failure_count(), 0);
    let _ = std::fs::remove_dir_all(&brain_path);
    ingester.handle_event_blocking(&make_terminal_event());
    assert_eq!(ingester.always_count(), 2, "both events should be counted");
}

// ── recurrence: a repeated project selection must REINFORCE, not insert ──

/// Row-level state of the one memory a repeated project selection should own.
struct MemoryRow {
    rows: i64,
    signal_score: f64,
    last_reinforced_at: Option<String>,
}

fn read_memory_row(db: &std::path::Path, key: &str) -> MemoryRow {
    // A fresh runtime, created only after every blocking Brain write has
    // returned: the Brain drives its own runtime with `block_on`, so the
    // ingester cannot be exercised from inside an async context.
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{}?mode=ro", db.display()))
            .await
            .expect("open memory.db");
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE key = ?")
            .bind(key)
            .fetch_one(&pool)
            .await
            .expect("count");
        let (signal_score, last_reinforced_at): (f64, Option<String>) =
            sqlx::query_as("SELECT signal_score, last_reinforced_at FROM memories WHERE key = ?")
                .bind(key)
                .fetch_one(&pool)
                .await
                .expect("read row");
        MemoryRow {
            rows,
            signal_score,
            last_reinforced_at,
        }
    })
}

/// The second selection of the same project must land on the FIRST memory and
/// strengthen it.
///
/// Collapsing the duplicates is only half the fix, and it is the half that can
/// pass on its own for the wrong reason: "the row count stopped growing" is
/// also true when ingestion silently stops writing. So this asserts both
/// halves, and asserts them from the two places they actually show up:
///
/// - the write outcome, via `reinforced_count` — which moves only when Spectral
///   reports the write was not an `Inserted` AND the follow-up reinforce found
///   the memory;
/// - the reinforcement itself, on the stored row: `signal_score` up and
///   `last_reinforced_at` set, where it had been null after the insert.
///
/// `always_count` and `failure_count` pin that both events were in fact
/// processed and neither write failed, so a green result cannot mean "nothing
/// happened". `last_reinforced_at` is the field to watch downstream: if
/// duplicate groups collapse but that stays null on the survivors, the key
/// change shipped without its other half.
#[test]
fn second_selection_of_a_project_reinforces_rather_than_inserts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
    let brain = Arc::new(
        Brain::builder()
            .data_dir(&brain_path)
            .ontology_path(&ontology_path)
            .device_id(spectral::DeviceId::from_descriptor("test-recurrence"))
            .build()
            .expect("test brain"),
    );
    let db = brain_path.join("memory.db");
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
    let key = "activity:project_selected:project:permagent";

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));
    assert_eq!(
        ingester.reinforced_count(),
        0,
        "the first occurrence must INSERT, not reinforce"
    );
    let first = read_memory_row(&db, key);
    assert_eq!(first.rows, 1, "first selection creates the memory");
    assert!(
        first.last_reinforced_at.is_none(),
        "an insert is not a reinforcement"
    );

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));

    assert_eq!(
        ingester.always_count(),
        2,
        "both events were processed — the writes did not silently stop"
    );
    assert_eq!(ingester.failure_count(), 0, "and neither write failed");
    assert_eq!(
        ingester.reinforced_count(),
        1,
        "the second write reported a non-Inserted outcome and reinforced the existing memory"
    );

    let second = read_memory_row(&db, key);
    assert_eq!(
        second.rows, 1,
        "still exactly one memory — one project, one identity"
    );
    assert!(
        second.signal_score > first.signal_score,
        "recurrence must strengthen the memory: {} -> {}",
        first.signal_score,
        second.signal_score
    );
    assert!(
        second.last_reinforced_at.is_some(),
        "recurrence must reset the decay clock"
    );
}

/// Selecting a different project must not reinforce the first one.
#[test]
fn selecting_a_different_project_inserts_a_second_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
    let brain = Arc::new(
        Brain::builder()
            .data_dir(&brain_path)
            .ontology_path(&ontology_path)
            .device_id(spectral::DeviceId::from_descriptor("test-two-projects"))
            .build()
            .expect("test brain"),
    );
    let db = brain_path.join("memory.db");
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());

    ingester.handle_event_blocking(&make_project_selected("project:permagent", "Permagent"));
    ingester.handle_event_blocking(&make_project_selected("project:get-ladle", "Get Ladle"));

    assert_eq!(
        ingester.reinforced_count(),
        0,
        "two different projects are two different facts"
    );
    assert_eq!(
        read_memory_row(&db, "activity:project_selected:project:permagent").rows,
        1
    );
    assert_eq!(
        read_memory_row(&db, "activity:project_selected:project:get-ladle").rows,
        1
    );
}

#[test]
fn chat_turn_completed_filtered_by_ingester() {
    let brain = common::shared_ingestion_brain();
    let ingester = ActivityIngester::new(SafeBrain::from_arc(brain), "test-device".into());
    ingester.handle_event_blocking(&make_always_event());
    assert_eq!(ingester.always_count(), 1);
    assert_eq!(ingester.filtered_count(), 1);
    assert!(ingester.last_ingested_at().is_none());
}
