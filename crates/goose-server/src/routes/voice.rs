//! Dedicated `/voice` WebSocket for the push-to-talk voice loop.
//!
//! Protocol (Phase 1, non-streaming):
//!   Client → Server:
//!     Text: {"type":"start","sample_rate":16000}
//!     Binary: [pcm_f32le audio chunks while push-to-talk held]
//!     Text: {"type":"stop"}
//!   Server → Client:
//!     Text: {"type":"transcript","text":"..."}
//!     Text: {"type":"reply_start"}
//!     Binary: [tts pcm_f32le audio]
//!     Text: {"type":"reply_end","sample_rate":24000}
//!     Text: {"type":"error","message":"..."}

use crate::state::AppState;
use crate::voice::provider::{SttConfig, TtsConfig};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use subtle;

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new().route("/voice", get(voice_ws_handler).with_state(state))
}

#[derive(Deserialize)]
struct VoiceQuery {
    session_id: Option<String>,
    token: Option<String>,
}

async fn voice_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<VoiceQuery>,
) -> Result<impl IntoResponse, axum::http::StatusCode> {
    // Manual token validation — WebSocket upgrade can't use Bearer middleware.
    if let Some(ref expected) = state.daemon_token {
        match &query.token {
            Some(t) if subtle::ConstantTimeEq::ct_eq(t.as_bytes(), expected.as_bytes()).into() => {}
            _ => return Err(axum::http::StatusCode::UNAUTHORIZED),
        }
    }
    Ok(ws.on_upgrade(move |socket| handle_voice_socket(socket, state, query.session_id)))
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "start")]
    Start { sample_rate: Option<u32> },
    #[serde(rename = "stop")]
    Stop,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "transcript")]
    Transcript { text: String },
    #[serde(rename = "reply_start")]
    ReplyStart,
    #[serde(rename = "reply_end")]
    ReplyEnd { sample_rate: u32 },
    #[serde(rename = "reply_text")]
    ReplyText { text: String },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "ready")]
    Ready,
}

fn send_json(msg: &ServerMessage) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn handle_voice_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    session_id: Option<String>,
) {
    tracing::info!(
        target: "permagentd::voice",
        "Voice WebSocket connected (session_id={:?}, stt={}, tts={})",
        session_id,
        state.voice_stt.is_some(),
        state.voice_tts.is_some()
    );

    // Check voice providers are available
    let (stt, tts) = match (&state.voice_stt, &state.voice_tts) {
        (Some(stt), Some(tts)) => (stt.clone(), tts.clone()),
        _ => {
            tracing::warn!(
                target: "permagentd::voice",
                "Voice providers not loaded — closing WebSocket"
            );
            let _ = socket
                .send(send_json(&ServerMessage::Error {
                    message: "Voice providers not available — models not loaded".into(),
                }))
                .await;
            return;
        }
    };

    // Signal ready
    tracing::info!(target: "permagentd::voice", "Sending ready signal to client");
    if socket.send(send_json(&ServerMessage::Ready)).await.is_err() {
        tracing::warn!(target: "permagentd::voice", "Failed to send ready — client disconnected");
        return;
    }

    let mut audio_buffer: Vec<f32> = Vec::new();
    let mut recording = false;
    let mut client_sample_rate: u32 = 16000;

    tracing::info!(target: "permagentd::voice", "Entering message loop");
    while let Some(result) = socket.recv().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(target: "permagentd::voice", "WebSocket recv error: {}", e);
                break;
            }
        };
        match msg {
            Message::Text(text) => {
                let text_str: &str = &text;
                tracing::debug!(target: "permagentd::voice", "Received text: {}", &text_str[..text_str.len().min(100)]);
                match serde_json::from_str::<ClientMessage>(text_str) {
                    Ok(ClientMessage::Start { sample_rate }) => {
                        audio_buffer.clear();
                        recording = true;
                        client_sample_rate = sample_rate.unwrap_or(16000);
                        tracing::info!(target: "permagentd::voice", "Recording started, sample_rate={}", client_sample_rate);
                    }
                    Ok(ClientMessage::Stop) if recording => {
                        recording = false;
                        let pipeline_start = std::time::Instant::now();
                        let audio_duration_s =
                            audio_buffer.len() as f32 / client_sample_rate as f32;
                        tracing::info!(
                            target: "permagentd::voice",
                            "Recording stopped, {} samples ({:.1}s audio)",
                            audio_buffer.len(), audio_duration_s
                        );

                        if audio_buffer.is_empty() {
                            continue;
                        }

                        // --- STT ---
                        let stt_start = std::time::Instant::now();
                        let samples = std::mem::take(&mut audio_buffer);
                        let sr = client_sample_rate;
                        let stt_ref = stt.clone();
                        let transcript = tokio::task::spawn_blocking(move || {
                            stt_ref.transcribe(&samples, sr, &SttConfig::default())
                        })
                        .await;

                        let transcript = match transcript {
                            Ok(Ok(t)) => t,
                            Ok(Err(e)) => {
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("STT failed: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                            Err(e) => {
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("STT task panicked: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                        };

                        let stt_ms = stt_start.elapsed().as_millis();
                        tracing::info!(
                            target: "permagentd::voice",
                            "TIMING STT: {}ms | transcript: \"{}\"",
                            stt_ms, &transcript[..transcript.len().min(80)]
                        );

                        if transcript.is_empty() {
                            continue;
                        }

                        if socket
                            .send(send_json(&ServerMessage::Transcript {
                                text: transcript.clone(),
                            }))
                            .await
                            .is_err()
                        {
                            return;
                        }

                        // --- Feed transcript into chat reply path ---
                        let reply_start = std::time::Instant::now();
                        let reply_text =
                            feed_transcript_to_chat(&state, &transcript, session_id.as_deref())
                                .await;

                        let reply_ms = reply_start.elapsed().as_millis();

                        let reply_text = match reply_text {
                            Ok(t) => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "TIMING Reply: {}ms | {} chars | \"{}...\"",
                                    reply_ms, t.len(), &t[..t.len().min(60)]
                                );
                                t
                            }
                            Err(e) => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "TIMING Reply: {}ms | FAILED: {}",
                                    reply_ms, e
                                );
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("Chat reply failed: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                        };

                        if socket
                            .send(send_json(&ServerMessage::ReplyText {
                                text: reply_text.clone(),
                            }))
                            .await
                            .is_err()
                        {
                            return;
                        }

                        // --- TTS ---
                        let tts_start = std::time::Instant::now();
                        let tts_ref = tts.clone();
                        let text_for_tts = reply_text;
                        let audio = tokio::task::spawn_blocking(move || {
                            tts_ref.synthesize(&text_for_tts, &TtsConfig::default())
                        })
                        .await;
                        let tts_ms = tts_start.elapsed().as_millis();

                        match audio {
                            Ok(Ok(audio)) => {
                                let audio_dur =
                                    audio.samples.len() as f32 / audio.sample_rate as f32;
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "TIMING TTS: {}ms | {:.1}s audio | {:.2}x realtime",
                                    tts_ms, audio_dur, tts_ms as f32 / 1000.0 / audio_dur
                                );

                                let _ = socket.send(send_json(&ServerMessage::ReplyStart)).await;
                                let bytes: Vec<u8> =
                                    audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                                let _ = socket.send(Message::Binary(bytes.into())).await;
                                let _ = socket
                                    .send(send_json(&ServerMessage::ReplyEnd {
                                        sample_rate: audio.sample_rate,
                                    }))
                                    .await;

                                let total_ms = pipeline_start.elapsed().as_millis();
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "TIMING Total: {}ms (STT={}ms Reply={}ms TTS={}ms)",
                                    total_ms, stt_ms, reply_ms, tts_ms
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "TIMING TTS: {}ms | FAILED: {}",
                                    tts_ms, e
                                );
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("TTS failed: {}", e),
                                    }))
                                    .await;
                            }
                            Err(e) => {
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("TTS task panicked: {}", e),
                                    }))
                                    .await;
                            }
                        }
                    }
                    Ok(ClientMessage::Stop) => {} // Not recording, ignore
                    Err(e) => {
                        tracing::warn!(target: "permagentd::voice", "Invalid voice message: {}", e);
                    }
                }
            }
            Message::Binary(data) if recording => {
                let chunk: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                audio_buffer.extend_from_slice(&chunk);
                if audio_buffer.len() % 16000 < chunk.len() {
                    tracing::debug!(
                        target: "permagentd::voice",
                        "Audio buffer: {} samples ({:.1}s)",
                        audio_buffer.len(),
                        audio_buffer.len() as f32 / client_sample_rate as f32
                    );
                }
            }
            Message::Close(frame) => {
                tracing::info!(target: "permagentd::voice", "Client sent Close frame: {:?}", frame);
                break;
            }
            Message::Ping(_) => {
                let _ = socket.send(Message::Pong(vec![].into())).await;
            }
            _ => {
                tracing::debug!(target: "permagentd::voice", "Received other message type");
            }
        }
    }
    tracing::info!(target: "permagentd::voice", "Voice WebSocket handler exiting");
}

/// Feed a voice transcript into the existing chat reply path and collect
/// the text response. Uses the same session/agent infrastructure as text chat.
async fn feed_transcript_to_chat(
    state: &AppState,
    transcript: &str,
    session_id: Option<&str>,
) -> anyhow::Result<String> {
    use futures::StreamExt;
    use permagent::agents::{AgentEvent, SessionConfig};
    use permagent::conversation::message::{Message as ChatMessage, MessageContent};

    // Resolve session ID
    let sid = if let Some(id) = session_id {
        id.to_string()
    } else {
        let sessions = state.session_manager().list_sessions().await?;
        sessions
            .first()
            .map(|s| s.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No session available for voice — create one first"))?
    };

    let user_msg = ChatMessage::user().with_text(transcript);

    let t = std::time::Instant::now();
    let agent = state
        .get_agent(sid.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get agent for session {}: {}", sid, e))?;
    tracing::info!(target: "permagentd::voice", "  reply: get_agent {}ms", t.elapsed().as_millis());

    // Inject recall (uses spawn_blocking internally)
    let t = std::time::Instant::now();
    if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        let n = crate::brain_ops::inject_recall(brain, &agent, transcript, recognition_ctx).await;
        tracing::info!(target: "permagentd::voice", "  reply: recall {}ms ({} hits)", t.elapsed().as_millis(), n);
    } else {
        tracing::info!(target: "permagentd::voice", "  reply: recall skipped (no Brain)");
    }

    let session_config = SessionConfig {
        id: sid.clone(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    };

    // Stream the reply — this is the LLM network call (likely the bottleneck)
    let t = std::time::Instant::now();
    let mut stream = agent.reply(user_msg, session_config, None).await?;
    tracing::info!(target: "permagentd::voice", "  reply: stream opened {}ms", t.elapsed().as_millis());

    let t = std::time::Instant::now();
    let mut reply_text = String::new();
    let mut first_token = true;
    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::Message(msg)) = event {
            if msg.role == rmcp::model::Role::Assistant {
                if first_token {
                    tracing::info!(target: "permagentd::voice", "  reply: first token {}ms", t.elapsed().as_millis());
                    first_token = false;
                }
                for content in &msg.content {
                    if let MessageContent::Text(text) = content {
                        reply_text.push_str(&text.text);
                    }
                }
            }
        }
    }
    tracing::info!(
        target: "permagentd::voice",
        "  reply: stream complete {}ms | {} chars",
        t.elapsed().as_millis(), reply_text.len()
    );

    Ok(reply_text)
}
