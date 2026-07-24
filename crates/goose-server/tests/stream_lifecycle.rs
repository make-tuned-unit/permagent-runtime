//! Streaming-lifecycle truth signals through the HTTP router (C1/C4/P1).
//!
//! Substitutes for a curl demonstration against a live daemon (the sandbox
//! blocks long-running daemon processes — see `decisions_lifecycle.rs`): this
//! drives the REAL `routes::session_events::routes` router with tower oneshot
//! requests against an AppState rooted at a throwaway PERMAGENT_PATH_ROOT, and
//! reads the actual SSE frames off the response body.
//!
//! Contract under test — "the chat UI must never lie about a turn's state":
//! 1. Subscribing ALWAYS yields an ActiveRequests frame first — including an
//!    EMPTY one, which is the client's only "nothing is running" signal after
//!    a daemon restart swallowed a turn without a terminal frame.
//! 2. POST /cancel answers honestly: `{cancelled:true}` only when a live
//!    request's token was actually cancelled; `{cancelled:false}` for a
//!    stale/unknown id or a session with no bus.
//! 3. `?last_event_id=N` resumes the replay after seq N (the query-param
//!    mirror of Last-Event-ID for EventSource clients, which cannot set
//!    headers on a manual reconnect).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use std::time::Duration;
use tower::ServiceExt;

use permagent::config::GooseMode;
use permagent::session::session_manager::SessionType;
use permagent_daemon::routes::reply::MessageEvent;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Read parsed SSE frames `(sse_id, json_payload)` off a live (never-ending)
/// SSE response body until `stop` is satisfied or a deadline passes. Heartbeat
/// comment frames (`: ping N`) are skipped; frames without an `id:` line (the
/// ActiveRequests preamble) surface with `None`.
async fn read_sse_frames(
    resp: axum::response::Response,
    stop: impl Fn(&[(Option<u64>, serde_json::Value)]) -> bool,
) -> Vec<(Option<u64>, serde_json::Value)> {
    let mut stream = resp.into_body().into_data_stream();
    let mut buf = String::new();
    let mut frames: Vec<(Option<u64>, serde_json::Value)> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    while tokio::time::Instant::now() < deadline && !stop(&frames) {
        match tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                // Byte-safe split on the ASCII frame separator — no string
                // indexing, so no char-boundary panic class (the repo denies
                // clippy::string_slice). The map ends the borrow of `buf`
                // before it is reassigned.
                while let Some((block, rest)) = buf
                    .split_once("\n\n")
                    .map(|(b, r)| (b.to_string(), r.to_string()))
                {
                    buf = rest;
                    if block.trim().is_empty() || block.starts_with(':') {
                        continue; // heartbeat / SSE comment
                    }
                    let mut id: Option<u64> = None;
                    let mut data: Option<serde_json::Value> = None;
                    for line in block.lines() {
                        if let Some(v) = line.strip_prefix("id: ") {
                            id = v.trim().parse().ok();
                        } else if let Some(v) = line.strip_prefix("data: ") {
                            data = serde_json::from_str(v).ok();
                        }
                    }
                    if let Some(d) = data {
                        frames.push((id, d));
                    }
                }
            }
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {} // idle read; keep waiting until stop/deadline
        }
    }
    frames
}

// One test per integration binary (own process): PERMAGENT_PATH_ROOT and the
// startup singletons are per-process, so #[serial] had nothing to serialize
// against here — it was a no-op (superseding #695).
#[tokio::test(flavor = "multi_thread")]
async fn streaming_lifecycle_truth_signals() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    // The router was split (scoped stream tokens must not reach reply/cancel):
    // this test drives both the SSE stream and POST /cancel with no auth layer,
    // so merge the event + control routers to mount all three handlers.
    let app = permagent_daemon::routes::session_events::event_routes(state.clone()).merge(
        permagent_daemon::routes::session_events::control_routes(state.clone()),
    );

    // A real session — /sessions/{id}/events 404s for unknown sessions.
    let session = state
        .session_manager()
        .create_session(
            std::path::PathBuf::from("/tmp"),
            "Stream lifecycle".to_string(),
            SessionType::User,
            GooseMode::default(),
        )
        .await
        .unwrap();
    let sid = session.id;

    // ── 1. Idle subscribe: the FIRST frame is an EMPTY ActiveRequests ──
    // The "nothing is running" reconciliation signal (C1's wedge-breaker):
    // must arrive even when no request is in flight.
    let resp = app
        .clone()
        .oneshot(get(&format!("/sessions/{sid}/events")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let frames = read_sse_frames(resp, |f| !f.is_empty()).await;
    assert!(
        !frames.is_empty(),
        "idle subscribe must emit at least the ActiveRequests preamble"
    );
    let (id, first) = &frames[0];
    assert_eq!(
        first["type"], "ActiveRequests",
        "first frame must be ActiveRequests, got: {first}"
    );
    assert_eq!(
        first["request_ids"].as_array().map(Vec::len),
        Some(0),
        "idle session must report an EMPTY request list, got: {first}"
    );
    assert!(
        id.is_none(),
        "ActiveRequests must carry no SSE id (it must not regress the client's cursor)"
    );
    eprintln!("✓ idle subscribe → empty ActiveRequests preamble: {first}");

    // ── 2. Cancel with nothing running → honest {cancelled:false} ──
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/sessions/{sid}/cancel"),
            serde_json::json!({ "request_id": "00000000-0000-0000-0000-000000000000" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(
        v["cancelled"], false,
        "cancelling a request that does not exist must say so, got: {v}"
    );
    eprintln!("✓ cancel of unknown request → {v}");

    // A session with NO bus at all is equally "nothing to cancel" — not a 404
    // the client can't tell apart from a wrong URL.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/sessions/no-bus-session/cancel",
            serde_json::json!({ "request_id": "req-x" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["cancelled"], false);
    eprintln!("✓ cancel on bus-less session → {v}");

    // ── 3. Live request: subscribe lists it; cancel is honest both ways ──
    let bus = state.get_or_create_event_bus(&sid).await;
    let token = bus
        .try_register_request("req-live-1".to_string())
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(get(&format!("/sessions/{sid}/events")))
        .await
        .unwrap();
    let frames = read_sse_frames(resp, |f| !f.is_empty()).await;
    let (_, first) = &frames[0];
    assert_eq!(first["type"], "ActiveRequests");
    assert_eq!(
        first["request_ids"],
        serde_json::json!(["req-live-1"]),
        "mid-turn subscribe must list the live request, got: {first}"
    );
    eprintln!("✓ mid-turn subscribe → {first}");

    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/sessions/{sid}/cancel"),
            serde_json::json!({ "request_id": "req-live-1" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["cancelled"], true, "live request must cancel, got: {v}");
    assert!(
        token.is_cancelled(),
        "cancel=true must mean the token was actually cancelled"
    );
    eprintln!("✓ cancel of live request → {v}");

    // After cleanup (what the reply task does when the turn settles), the same
    // id is stale — cancelling it again must be honest about doing nothing.
    bus.cleanup_request("req-live-1").await;
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/sessions/{sid}/cancel"),
            serde_json::json!({ "request_id": "req-live-1" }),
        ))
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert_eq!(
        v["cancelled"], false,
        "cancel of an already-settled request must say nothing was cancelled"
    );
    eprintln!("✓ cancel of settled request → {v}");

    // ── 4. ?last_event_id=N resumes the replay after seq N ──
    bus.publish(None, MessageEvent::Ping).await; // seq 1
    bus.publish(None, MessageEvent::Ping).await; // seq 2
    bus.publish(None, MessageEvent::Ping).await; // seq 3

    let resp = app
        .clone()
        .oneshot(get(&format!("/sessions/{sid}/events?last_event_id=2")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let frames = read_sse_frames(resp, |f| f.iter().any(|(id, _)| *id == Some(3))).await;
    assert!(
        frames
            .iter()
            .any(|(id, d)| *id == Some(3) && d["type"] == "Ping"),
        "resume must replay seq 3, got: {frames:?}"
    );
    assert!(
        frames.iter().all(|(id, _)| id.is_none_or(|i| i > 2)),
        "resume from last_event_id=2 must not re-replay seq 1/2, got: {frames:?}"
    );
    eprintln!("✓ ?last_event_id=2 → replay resumed at seq 3, nothing re-replayed");

    // A malformed cursor degrades to a full replay, never a 400 on the
    // streaming endpoint.
    let resp = app
        .clone()
        .oneshot(get(&format!("/sessions/{sid}/events?last_event_id=bogus")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let frames = read_sse_frames(resp, |f| f.iter().any(|(id, _)| *id == Some(1))).await;
    assert!(
        frames.iter().any(|(id, _)| *id == Some(1)),
        "malformed cursor must fall back to full replay, got: {frames:?}"
    );
    eprintln!("✓ malformed last_event_id → 200 + full replay");

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}
