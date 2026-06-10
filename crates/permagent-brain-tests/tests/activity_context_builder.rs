//! Brain-dependent tests for activity::context_builder.
//! Extracted from crates/goose/src/activity/context_builder.rs to avoid
//! V8 libc++abi symbol collision on Linux (issue #190).

mod common;

// Sanctioned raw spectral::Brain usage — test crate owns its runtime.
use permagent::activity::context_builder::{ContextBuilder, DigestOpts};
use permagent::brain_handle::SafeBrain;
use permagent::events::activity::{ActivityEvent, ActivityEventType, EventTier, SourceSurface};

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
    let brain = common::shared_context_builder_brain();
    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

    let event = make_event(
        ActivityEventType::BrowserNavigated,
        SourceSurface::Browser,
        EventTier::Aggregated,
        serde_json::json!({"url": "https://example.com", "title": "Example", "tab_id": "t1"}),
    );
    cb.handle_event(&event);

    let state = cb.live_state_snapshot();
    assert_eq!(
        state.last_browser_url.as_deref(),
        Some("https://example.com")
    );
}

#[test]
fn live_state_tracks_terminal_command() {
    let brain = common::shared_context_builder_brain();
    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

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
    let brain = common::shared_context_builder_brain();
    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

    let mut event = make_event(
        ActivityEventType::ProjectSelected,
        SourceSurface::ProjectPicker,
        EventTier::Always,
        serde_json::json!({"project_id": "project:permagent", "project_name": "Permagent"}),
    );
    event.project_id = Some("project:permagent".into());
    cb.handle_event(&event);

    let state = cb.live_state_snapshot();
    assert_eq!(
        state.active_project_id.as_deref(),
        Some("project:permagent")
    );
}

#[test]
fn ring_buffer_bounded() {
    let brain = common::shared_context_builder_brain();
    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

    for _ in 0..1100 {
        let event = make_event(
            ActivityEventType::ChatTurnStarted,
            SourceSurface::Chat,
            EventTier::Ephemeral,
            serde_json::json!({}),
        );
        cb.handle_event(&event);
    }

    // MAX_RING_BUFFER_SIZE = 1000 (crate-private constant)
    assert_eq!(cb.buffered_count(), 1000);
}

#[test]
fn current_digest_blocking_returns_recent_events() {
    let brain = common::shared_context_builder_brain();
    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

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

    let digest = cb.current_digest_blocking(DigestOpts::default()).unwrap();
    assert_eq!(digest.recent_events.len(), 5);
    assert!(digest.probed_memories.is_empty());
    assert!(digest.recalled_memories.is_empty());
}

#[test]
fn probe_results_sorted_by_relevance_descending() {
    let brain = common::shared_context_builder_brain();

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

    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

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
        .current_digest_blocking(DigestOpts {
            include_probe: true,
            min_probe_relevance: Some(0.0),
            ..Default::default()
        })
        .unwrap();

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
    let brain = common::shared_context_builder_brain();

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

    let cb = ContextBuilder::new(SafeBrain::from_arc(brain));

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
        .current_digest_blocking(DigestOpts {
            include_probe: true,
            focus_wing: Some("permagent".into()),
            min_probe_relevance: Some(0.0),
            ..Default::default()
        })
        .unwrap();

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
