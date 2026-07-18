//! WebSocket endpoint at GET /events for the Permagent event bus.
//!
//! Upgrades to WebSocket, streams all [`PermagentEvent`]s as JSON.
//! Supports event replay via `{"resume_from": "<event_id>"}` on connect.
//!
//! Replay honesty (#770 follow-up): every frame re-delivered from the replay
//! buffer — the full-buffer backfill on a plain connect AND the `resume_from`
//! replay — is stamped `"replayed": true` server-side (`into_replayed`), so
//! clients can tell history from a live instruction without inferring it from
//! timestamps. Live frames omit the field entirely (byte-identical to the
//! pre-marker wire). What gets buffered/replayed is unchanged — frames are
//! only *marked*.

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use permagent::events;
use std::sync::Arc;
use tracing::{debug, warn};

pub fn routes(_state: Arc<crate::state::AppState>) -> Router {
    // Event bus is global — no AppState needed for the handler itself.
    // We accept state for consistency with other route modules.
    Router::new().route("/events", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();

    debug!("WebSocket client connected to /events");

    // Wait briefly for an optional resume_from message from client.
    // If the client sends {"resume_from": "<id>"} within 500ms, replay from that point.
    let resume_from = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        if let Some(Ok(Message::Text(text))) = receiver.next().await {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(id) = parsed.get("resume_from").and_then(|v| v.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
        None
    })
    .await
    .unwrap_or(None);

    // Handle replay
    if let Some(ref resume_id) = resume_from {
        match events::buffered_events_after(resume_id) {
            Some(replay_events) => {
                debug!(
                    "Replaying {} events after {}",
                    replay_events.len(),
                    resume_id
                );
                for event in replay_events {
                    // Buffer re-delivery → stamp the replay marker.
                    if let Ok(json) = serde_json::to_string(&event.into_replayed()) {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            return; // Client disconnected
                        }
                    }
                }
            }
            None => {
                // resume_from ID not in buffer — send gap notification
                let gap_msg = serde_json::json!({
                    "type": "resume_gap",
                    "message": "resume_from ID not found in buffer, streaming live only"
                });
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&gap_msg).unwrap().into(),
                    ))
                    .await;
            }
        }
    } else {
        // No resume requested — send all buffered events, each stamped as a
        // replay so the client never mistakes backfill for a live instruction.
        let buffered = events::buffered_events();
        for event in buffered {
            if let Ok(json) = serde_json::to_string(&event.into_replayed()) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
            }
        }
    }

    // Subscribe to live events
    let mut rx = events::subscribe();

    // Stream live events. Also listen for client disconnect via the receiver half.
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break; // Client disconnected
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket client lagged, missed {} events", n);
                        // Continue streaming — client can reconnect with resume_from
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break; // Channel closed (shouldn't happen with global singleton)
                    }
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }

    debug!("WebSocket client disconnected from /events");
}
