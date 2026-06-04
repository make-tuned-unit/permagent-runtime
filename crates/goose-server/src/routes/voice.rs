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
    // Check voice providers are available
    let (stt, tts) = match (&state.voice_stt, &state.voice_tts) {
        (Some(stt), Some(tts)) => (stt.clone(), tts.clone()),
        _ => {
            let _ = socket
                .send(send_json(&ServerMessage::Error {
                    message: "Voice providers not available — models not loaded".into(),
                }))
                .await;
            return;
        }
    };

    // Signal ready
    if socket.send(send_json(&ServerMessage::Ready)).await.is_err() {
        return;
    }

    let mut audio_buffer: Vec<f32> = Vec::new();
    let mut recording = false;
    let mut client_sample_rate: u32 = 16000;

    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                let text_str: &str = &text;
                match serde_json::from_str::<ClientMessage>(text_str) {
                    Ok(ClientMessage::Start { sample_rate }) => {
                        audio_buffer.clear();
                        recording = true;
                        client_sample_rate = sample_rate.unwrap_or(16000);
                        tracing::debug!(target: "permagentd::voice", "Recording started, sample_rate={}", client_sample_rate);
                    }
                    Ok(ClientMessage::Stop) if recording => {
                        recording = false;
                        tracing::debug!(
                            target: "permagentd::voice",
                            "Recording stopped, {} samples captured",
                            audio_buffer.len()
                        );

                        if audio_buffer.is_empty() {
                            continue;
                        }

                        // --- STT ---
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

                        if transcript.is_empty() {
                            continue;
                        }

                        // Send transcript
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
                        let reply_text =
                            feed_transcript_to_chat(&state, &transcript, session_id.as_deref())
                                .await;

                        let reply_text = match reply_text {
                            Ok(t) => t,
                            Err(e) => {
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("Chat reply failed: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                        };

                        // Send reply text
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
                        let tts_ref = tts.clone();
                        let text_for_tts = reply_text;
                        let audio = tokio::task::spawn_blocking(move || {
                            tts_ref.synthesize(&text_for_tts, &TtsConfig::default())
                        })
                        .await;

                        match audio {
                            Ok(Ok(audio)) => {
                                let _ = socket.send(send_json(&ServerMessage::ReplyStart)).await;

                                // Send audio as binary (f32le PCM)
                                let bytes: Vec<u8> =
                                    audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                                let _ = socket.send(Message::Binary(bytes.into())).await;

                                let _ = socket
                                    .send(send_json(&ServerMessage::ReplyEnd {
                                        sample_rate: audio.sample_rate,
                                    }))
                                    .await;
                            }
                            Ok(Err(e)) => {
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
                // PCM f32le audio chunk
                let chunk: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                audio_buffer.extend_from_slice(&chunk);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
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

    // Create user message from transcript
    let user_msg = ChatMessage::user().with_text(transcript);

    // Get agent for this session
    let agent = state
        .get_agent(sid.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get agent for session {}: {}", sid, e))?;

    // Inject recall if Brain is available (already uses spawn_blocking internally)
    if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        crate::brain_ops::inject_recall(brain, &agent, transcript, recognition_ctx).await;
    }

    let session_config = SessionConfig {
        id: sid.clone(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    };

    // Stream the reply and collect the assistant text
    let mut stream = agent.reply(user_msg, session_config, None).await?;

    let mut reply_text = String::new();
    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::Message(msg)) = event {
            if msg.role == rmcp::model::Role::Assistant {
                for content in &msg.content {
                    if let MessageContent::Text(text) = content {
                        reply_text.push_str(&text.text);
                    }
                }
            }
        }
    }

    Ok(reply_text)
}
