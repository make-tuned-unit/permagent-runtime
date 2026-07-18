//! Server-side replay marker on /events frames (#770 follow-up).
//!
//! Drives the REAL `routes::events::routes` WebSocket over a real TCP socket
//! (the `events_wire.rs` harness pattern) and proves the wire contract:
//!
//! - frames re-delivered from the replay buffer — BOTH the full-buffer
//!   backfill on a plain connect and the `resume_from` replay — carry
//!   `"replayed": true`;
//! - live frames omit the field entirely (byte-identical to the pre-marker
//!   wire, so older clients are unaffected);
//! - what gets buffered/replayed is unchanged — frames are only marked.
//!
//! One test per integration binary (own process): the event bus and
//! PERMAGENT_PATH_ROOT are per-process, so this buffer is isolated from the
//! sibling `events_wire` binary's traffic.

use futures::{SinkExt, Stream, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Read text frames off `ws` until one parses AND matches `pred` (returning
/// it), or `deadline` passes (returning None).
async fn capture_until<S>(
    ws: &mut S,
    deadline: tokio::time::Instant,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value>
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    if pred(&v) {
                        return Some(v);
                    }
                }
            }
            Ok(Some(Ok(_))) => {} // ping/binary — ignore
            Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => {} // idle read; keep waiting until the deadline
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_frames_are_marked_and_live_frames_are_not() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    // Real AppState + the REAL events router on a real loopback TCP socket.
    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::events::routes(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });

    use permagent::events::{self, emit};

    // ── Sentinel A lands in the buffer BEFORE any client connects: a fresh
    //    connection can only ever receive it as a buffer replay. ──
    let a = events::task_created("task-REPLAY-A", "buffered before connect", None);
    let a_id = a.id.clone();
    emit(a);

    // ── ws1: plain connect (no resume) → the full-buffer backfill delivers A,
    //    and it must carry the marker. ──
    let url = format!("ws://{addr}/events");
    let (mut ws1, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let frame_a = capture_until(&mut ws1, deadline, |v| v["id"] == a_id.as_str())
        .await
        .expect("buffered sentinel A must be replayed to a fresh connection");
    eprintln!("replayed-on-connect frame: {frame_a}");
    // The canonical envelope is intact — the marker is additive.
    assert_eq!(frame_a["type"], "task_created");
    assert!(frame_a["timestamp"].is_string());
    assert_eq!(frame_a["payload"]["task_id"], "task-REPLAY-A");
    assert_eq!(
        frame_a["replayed"],
        serde_json::Value::Bool(true),
        "a buffer-replayed frame must be stamped replayed:true: {frame_a}"
    );

    // ── Sentinel B live on ws1. The handler only subscribes to the live bus
    //    after the resume-wait + backfill, so re-emit each round until a copy
    //    is captured (the events_wire race-defeat pattern); every copy this
    //    connection sees is a live send and must be unmarked. ──
    let mut b_ids: Vec<String> = Vec::new();
    let overall = tokio::time::Instant::now() + Duration::from_secs(10);
    let frame_b = loop {
        assert!(
            tokio::time::Instant::now() < overall,
            "live sentinel B never captured on ws1"
        );
        let b = events::task_started("task-REPLAY-B", "sess-replay-marker");
        b_ids.push(b.id.clone());
        emit(b);
        let round = tokio::time::Instant::now() + Duration::from_millis(400);
        if let Some(f) = capture_until(&mut ws1, round, |v| {
            v["payload"]["task_id"] == "task-REPLAY-B"
        })
        .await
        {
            break f;
        }
    };
    eprintln!("live frame: {frame_b}");
    assert!(
        frame_b.get("replayed").is_none(),
        "a live frame must omit the marker entirely (pre-marker wire shape): {frame_b}"
    );

    // ── ws2: resume_from=A → the resume replay re-delivers the B emit(s),
    //    marked. Every B copy on ws2 comes from the resume backfill (nothing
    //    emits B after this point). ──
    let (mut ws2, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws2.send(Message::Text(
        format!("{{\"resume_from\":\"{a_id}\"}}").into(),
    ))
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let resumed_b = capture_until(&mut ws2, deadline, |v| {
        v["payload"]["task_id"] == "task-REPLAY-B"
    })
    .await
    .expect("resume_from replay must re-deliver the buffered B frame(s)");
    eprintln!("resume-replayed frame: {resumed_b}");
    assert!(
        b_ids.iter().any(|id| resumed_b["id"] == id.as_str()),
        "the resumed frame must be one of the emitted B envelopes"
    );
    assert_eq!(
        resumed_b["replayed"],
        serde_json::Value::Bool(true),
        "a resume-replayed frame must be stamped replayed:true: {resumed_b}"
    );

    eprintln!("✓ replay marker: backfill + resume replays marked, live frames unmarked.");
}
