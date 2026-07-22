//! Live emit→WebSocket evidence for the #629 multi-client liveness lanes.
//!
//! Mirrors `events_wire.rs`: drives the REAL `routes::events::routes` WebSocket
//! over a real TCP socket, emits each new liveness event through its PRODUCTION
//! constructor, and captures the exact JSON frame a second client receives off
//! the wire. Asserts the snake_case `type` discriminator and the id+change
//! payload discipline (ids only — clients refetch; the bus never carries rows).
//!
//! Lanes covered (one per stale surface the sweep wires):
//!   workspace_changed — PUT /api/workspaces/{id}/layout
//!   project_changed   — project CRUD + tags/memories/documents/notes
//!   person_changed    — project people associate/disassociate
//!   identity_changed  — PUT /api/agent/identity
//!   session_changed   — session create/delete/rename/fork
//!
//! Run with `--nocapture` to print every captured frame as evidence.

use futures::StreamExt;
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

// One test per integration binary (own process): PERMAGENT_PATH_ROOT and the
// global event bus are per-process (see events_wire.rs for why #[serial] is a
// no-op here). Delivery robustness comes from re-emitting until captured.
#[tokio::test(flavor = "multi_thread")]
async fn liveness_lanes_emit_to_real_websocket() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::events::routes(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    // Daemon token rides the WS query, like the browser clients (C1/C2 auth).
    let token = state
        .daemon_token
        .clone()
        .expect("test AppState should have generated a daemon token");
    let url = format!("ws://{addr}/events?token={token}");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Re-emit each round until every lane's frame is observed off the wire —
    // defeats the subscribe/replay race exactly as events_wire.rs does.
    use permagent::events::{self, emit};
    let emit_all_lanes = || {
        emit(events::workspace_changed("ws-EVID-1", "layout"));
        emit(events::project_changed("proj-EVID-1", "updated"));
        emit(events::person_changed(
            "proj-EVID-1",
            "person-EVID-uuid",
            "associated",
        ));
        emit(events::identity_changed("Henry Evidence"));
        emit(events::session_changed("sess-EVID-1", "created"));
    };

    let have_all_lanes = |frames: &[serde_json::Value]| {
        find(frames, "workspace_changed", |p| {
            p["workspace_id"] == "ws-EVID-1"
        })
        .is_some()
            && find(frames, "project_changed", |p| {
                p["project_id"] == "proj-EVID-1"
            })
            .is_some()
            && find(frames, "person_changed", |p| {
                p["entity_uuid"] == "person-EVID-uuid"
            })
            .is_some()
            && find(frames, "identity_changed", |p| {
                p["display_name"] == "Henry Evidence"
            })
            .is_some()
            && find(frames, "session_changed", |p| {
                p["session_id"] == "sess-EVID-1"
            })
            .is_some()
    };

    eprintln!("\n── emitting 5 liveness events through production constructors ──");
    let mut frames: Vec<serde_json::Value> = Vec::new();
    let overall = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < overall && !have_all_lanes(&frames) && frames.len() < 500 {
        emit_all_lanes();
        let round_end = tokio::time::Instant::now() + Duration::from_millis(400);
        while tokio::time::Instant::now() < round_end && frames.len() < 500 {
            match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
                Ok(Some(Ok(Message::Text(txt)))) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                        frames.push(v);
                    }
                }
                Ok(Some(Ok(_))) => {} // ping/binary/close — ignore
                Ok(Some(Err(_))) | Ok(None) => break,
                Err(_) => {} // idle read; keep waiting until the round ends
            }
        }
    }
    eprintln!(
        "\n── captured {} frame(s) off the WebSocket ──",
        frames.len()
    );

    eprintln!("\n── verified emit→WebSocket per liveness lane ──");

    let f = find(&frames, "workspace_changed", |p| {
        p["workspace_id"] == "ws-EVID-1"
    })
    .expect("workspace_changed not delivered over WS");
    assert_envelope(f, "workspace_changed");
    assert_eq!(f["payload"]["change"], "layout");
    assert!(
        f["payload"]["layout_json"].is_null(),
        "discipline breach: layout body on the bus"
    );

    let f = find(&frames, "project_changed", |p| {
        p["project_id"] == "proj-EVID-1"
    })
    .expect("project_changed not delivered over WS");
    assert_envelope(f, "project_changed");
    assert_eq!(f["payload"]["change"], "updated");
    assert!(
        f["payload"]["name"].is_null() && f["payload"]["status"].is_null(),
        "discipline breach: project fields on the bus"
    );

    let f = find(&frames, "person_changed", |p| {
        p["entity_uuid"] == "person-EVID-uuid"
    })
    .expect("person_changed not delivered over WS");
    assert_envelope(f, "person_changed");
    assert_eq!(f["payload"]["project_id"], "proj-EVID-1");
    assert_eq!(f["payload"]["change"], "associated");

    let f = find(&frames, "identity_changed", |p| {
        p["display_name"] == "Henry Evidence"
    })
    .expect("identity_changed not delivered over WS");
    assert_envelope(f, "identity_changed");
    assert!(
        f["payload"]["traits"].is_null() && f["payload"]["tone"].is_null(),
        "discipline breach: persona body on the bus — clients re-read the API"
    );

    let f = find(&frames, "session_changed", |p| {
        p["session_id"] == "sess-EVID-1"
    })
    .expect("session_changed not delivered over WS");
    assert_envelope(f, "session_changed");
    assert_eq!(f["payload"]["change"], "created");
}
