//! Live emit→WebSocket evidence for the daemon-emit lanes (#313/#324/#325/#302/#288).
//!
//! Substitutes for a wscat/curl demonstration against a live daemon — the sandbox
//! blocks long-running daemon processes (see `decisions_lifecycle.rs`). This drives
//! the REAL `routes::events::routes` WebSocket over a real TCP socket: it emits each
//! new event through its PRODUCTION constructor and captures the exact JSON frame a
//! World View consumer receives off the wire. It asserts the snake_case `type`
//! discriminator, the payload shape, and payload discipline (no raw memory/entity
//! bodies on the broadcast bus).
//!
//! Run with `--nocapture` to print every captured frame as evidence.

use futures::StreamExt;
use serial_test::serial;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Find the first captured frame of `event_type` matching `pred`.
fn find<'a>(
    frames: &'a [serde_json::Value],
    event_type: &str,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> Option<&'a serde_json::Value> {
    frames
        .iter()
        .find(|f| f["type"] == event_type && pred(&f["payload"]))
}

/// Assert the canonical envelope: id, snake_case type, timestamp, payload.
fn assert_envelope(frame: &serde_json::Value, expected_type: &str) {
    assert!(frame["id"].is_string(), "envelope missing id: {frame}");
    assert_eq!(frame["type"], expected_type, "wrong/non-snake_case type");
    assert!(frame["timestamp"].is_string(), "envelope missing timestamp");
    assert!(
        frame["payload"].is_object(),
        "envelope missing payload object"
    );
    eprintln!(
        "  ✓ {expected_type}: {}",
        serde_json::to_string(frame).unwrap()
    );
}

// #[serial]: mutates the process-global PERMAGENT_PATH_ROOT + subscribes to the
// global event bus — must not run concurrently with other AppState tests.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn all_lanes_emit_to_real_websocket() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    // Real AppState (also spawns the #288 agent-state tick) + the REAL events router.
    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::events::routes(state.clone());

    // Serve the router on a real loopback TCP socket.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    // Connect a real WebSocket client.
    let url = format!("ws://{addr}/events");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // The handler waits up to 500ms for an optional resume_from, replays the buffer,
    // then subscribes. Wait that out so our LIVE emits reach a live subscriber.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // ── Emit one event per lane via its PRODUCTION constructor (sentinel ids) ──
    use permagent::events::{self, emit};
    eprintln!("\n── emitting 7 events through production constructors ──");
    emit(events::memory_added(
        "mem-EVID-1",
        "activity:evid:browser_navigated:abcd1234",
        "browser_navigated",
        Some("personal"),
    )); // lane 1
    emit(events::memory_recalled(
        "when did I switch editors",
        3,
        "search_memory",
    )); // lane 3 (#324)
    emit(events::entity_added("term:evid-rust", "term")); // lane 4 (#325)
    emit(events::entity_updated("cat:evid-tools", "cat")); // lane 4 (#325)
    emit(events::decision_created("dec-EVID-1", "unblock", 2)); // lane 5 (#302)
    emit(events::decision_resolved(
        "dec-EVID-1",
        "unblock",
        "approve",
        "jesse",
        2,
    )); // lane 5 (#302)
    emit(events::agent_state_changed("henry", "Henry", "working")); // lane 6 (#288)

    // ── Capture frames off the wire until all sentinels seen (or timeout) ──
    let mut frames: Vec<serde_json::Value> = Vec::new();
    let overall = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < overall && frames.len() < 200 {
        match tokio::time::timeout(Duration::from_millis(300), ws.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    frames.push(v);
                }
            }
            Ok(Some(Ok(_))) => {} // ping/binary/close — ignore
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {} // idle read; keep waiting until the deadline
        }
    }
    eprintln!(
        "\n── captured {} frame(s) off the WebSocket ──",
        frames.len()
    );

    // ── Assert each lane's event arrived with the right wire shape ──
    eprintln!("\n── verified emit→WebSocket per lane ──");

    let f = find(&frames, "memory_added", |p| p["memory_id"] == "mem-EVID-1")
        .expect("lane 1: memory_added not delivered over WS");
    assert_envelope(f, "memory_added");
    assert_eq!(f["payload"]["category"], "browser_navigated");
    assert_eq!(f["payload"]["wing"], "personal");
    assert!(f["payload"]["key"].is_string());
    assert!(
        f["payload"]["content"].is_null(),
        "discipline breach: raw content on the bus"
    );

    let f = find(&frames, "memory_recalled", |p| {
        p["source"] == "search_memory"
    })
    .expect("lane 3 (#324): memory_recalled not delivered over WS");
    assert_envelope(f, "memory_recalled");
    assert_eq!(f["payload"]["hit_count"], 3);
    assert_eq!(f["payload"]["query"], "when did I switch editors");

    let f = find(&frames, "entity_added", |p| {
        p["entity_id"] == "term:evid-rust"
    })
    .expect("lane 4 (#325): entity_added not delivered over WS");
    assert_envelope(f, "entity_added");
    assert_eq!(f["payload"]["entity_type"], "term");

    let f = find(&frames, "entity_updated", |p| {
        p["entity_id"] == "cat:evid-tools"
    })
    .expect("lane 4 (#325): entity_updated not delivered over WS");
    assert_envelope(f, "entity_updated");
    assert_eq!(f["payload"]["entity_type"], "cat");

    let f = find(&frames, "decision_created", |p| {
        p["decision_id"] == "dec-EVID-1"
    })
    .expect("lane 5 (#302): decision_created not delivered over WS");
    assert_envelope(f, "decision_created");
    assert_eq!(f["payload"]["kind"], "unblock");
    assert_eq!(f["payload"]["tier"], 2);
    assert!(
        f["payload"]["headline"].is_null() && f["payload"]["detail"].is_null(),
        "discipline breach: decision headline/detail on the bus"
    );

    let f = find(&frames, "decision_resolved", |p| p["answer"] == "approve")
        .expect("lane 5 (#302): decision_resolved not delivered over WS");
    assert_envelope(f, "decision_resolved");
    assert_eq!(f["payload"]["decision_id"], "dec-EVID-1");
    assert_eq!(f["payload"]["acted_by"], "jesse");

    let f = find(&frames, "agent_state_changed", |p| p["state"] == "working")
        .expect("lane 6 (#288): agent_state_changed not delivered over WS");
    assert_envelope(f, "agent_state_changed");
    assert_eq!(f["payload"]["agent_id"], "henry");
    assert_eq!(f["payload"]["name"], "Henry");

    eprintln!("\n✓ all 7 events traversed emit→/events WebSocket with correct wire shape.\n");
}
