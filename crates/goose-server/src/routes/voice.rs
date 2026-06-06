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

                        // --- Streaming reply + TTS: synthesize and send each sentence as it completes ---
                        if socket
                            .send(send_json(&ServerMessage::ReplyStart))
                            .await
                            .is_err()
                        {
                            return;
                        }

                        let stream_result = stream_reply_with_tts(
                            &state,
                            &transcript,
                            session_id.as_deref(),
                            &tts,
                            &mut socket,
                            pipeline_start,
                            stt_ms,
                        )
                        .await;

                        if let Err(e) = stream_result {
                            let _ = socket
                                .send(send_json(&ServerMessage::Error {
                                    message: format!("Voice reply failed: {}", e),
                                }))
                                .await;
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

/// Stream the LLM reply, synthesize each sentence as it completes, and send
/// audio chunks to the client immediately. This way the client starts playing
/// sentence 1 while sentence 2 is still generating.
async fn stream_reply_with_tts(
    state: &AppState,
    transcript: &str,
    session_id: Option<&str>,
    tts: &Arc<dyn crate::voice::TextToSpeech>,
    socket: &mut WebSocket,
    pipeline_start: std::time::Instant,
    stt_ms: u128,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use permagent::agents::{AgentEvent, SessionConfig};
    use permagent::conversation::message::{Message as ChatMessage, MessageContent};

    let sid = if let Some(id) = session_id {
        id.to_string()
    } else {
        let sessions = state.session_manager().list_sessions().await?;
        sessions
            .first()
            .map(|s| s.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No session available for voice"))?
    };

    let user_msg = ChatMessage::user().with_text(transcript);
    let agent = state
        .get_agent(sid.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get agent: {}", e))?;

    agent
        .extend_system_prompt(
            "voice_reply_style".to_string(),
            "The user is speaking to you by voice. Reply in natural conversational speech: \
             short sentences, contractions, concise and direct. No markdown, no bullet points, \
             no numbered lists, no code blocks. Keep replies brief — 1-3 sentences for simple \
             questions. Speak as you would in a real conversation."
                .to_string(),
        )
        .await;

    let t = std::time::Instant::now();
    if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        let n = crate::brain_ops::inject_recall(brain, &agent, transcript, recognition_ctx).await;
        tracing::info!(target: "permagentd::voice", "  recall: {}ms ({} hits)", t.elapsed().as_millis(), n);
    }

    let session_config = SessionConfig {
        id: sid.clone(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    };

    let reply_start = std::time::Instant::now();
    let mut stream = agent.reply(user_msg, session_config, None).await?;

    // Accumulate text, detect sentence boundaries, synthesize + send each sentence
    let mut text_buf = String::new();
    let mut full_reply = String::new();
    let mut sentence_num = 0u32;
    let mut total_tts_ms: u128 = 0;
    let mut first_audio_sent = false;

    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::Message(msg)) = event {
            if msg.role != rmcp::model::Role::Assistant {
                continue;
            }
            for content in &msg.content {
                if let MessageContent::Text(text_content) = content {
                    text_buf.push_str(&text_content.text);
                    full_reply.push_str(&text_content.text);

                    // Check for sentence boundary
                    while let Some(pos) = find_sentence_end(&text_buf) {
                        let sentence = text_buf[..=pos].trim().to_string();
                        text_buf = text_buf[pos + 1..].to_string();

                        if sentence.is_empty() {
                            continue;
                        }

                        sentence_num += 1;
                        let tts_ref = tts.clone();
                        let sent = sentence.clone();
                        let tts_start = std::time::Instant::now();
                        let audio = tokio::task::spawn_blocking(move || {
                            tts_ref.synthesize(&sent, &TtsConfig::default())
                        })
                        .await;
                        let chunk_tts_ms = tts_start.elapsed().as_millis();
                        total_tts_ms += chunk_tts_ms;

                        match audio {
                            Ok(Ok(audio)) => {
                                let dur = audio.samples.len() as f32 / audio.sample_rate as f32;
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "STREAM sentence {}: TTS {}ms ({:.1}s audio) | \"{}\"",
                                    sentence_num, chunk_tts_ms, dur,
                                    &sentence[..sentence.len().min(50)]
                                );

                                if !first_audio_sent {
                                    let first_audio_ms = pipeline_start.elapsed().as_millis();
                                    tracing::info!(
                                        target: "permagentd::voice",
                                        "TIMING first audio: {}ms after speech-end",
                                        first_audio_ms
                                    );
                                    first_audio_sent = true;
                                }

                                let bytes: Vec<u8> =
                                    audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                                if socket.send(Message::Binary(bytes.into())).await.is_err() {
                                    tracing::warn!(target: "permagentd::voice", "Client disconnected during streaming");
                                    return Ok(());
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(target: "permagentd::voice", "TTS chunk failed: {}", e);
                            }
                            Err(e) => {
                                tracing::warn!(target: "permagentd::voice", "TTS task panicked: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    // Synthesize any remaining text after the stream ends
    let remainder = text_buf.trim().to_string();
    if !remainder.is_empty() {
        sentence_num += 1;
        let tts_ref = tts.clone();
        let tts_start = std::time::Instant::now();
        let audio = tokio::task::spawn_blocking(move || {
            tts_ref.synthesize(&remainder, &TtsConfig::default())
        })
        .await;
        let chunk_tts_ms = tts_start.elapsed().as_millis();
        total_tts_ms += chunk_tts_ms;

        if let Ok(Ok(audio)) = audio {
            if !first_audio_sent {
                let first_audio_ms = pipeline_start.elapsed().as_millis();
                tracing::info!(
                    target: "permagentd::voice",
                    "TIMING first audio: {}ms after speech-end",
                    first_audio_ms
                );
            }
            let bytes: Vec<u8> = audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            let _ = socket.send(Message::Binary(bytes.into())).await;
        }
    }

    // Send reply text and end marker
    let _ = socket
        .send(send_json(&ServerMessage::ReplyText {
            text: full_reply.clone(),
        }))
        .await;

    let reply_ms = reply_start.elapsed().as_millis();
    let _ = socket
        .send(send_json(&ServerMessage::ReplyEnd {
            sample_rate: tts.sample_rate(),
        }))
        .await;

    let total_ms = pipeline_start.elapsed().as_millis();
    tracing::info!(
        target: "permagentd::voice",
        "TIMING Total: {}ms (STT={}ms Reply+TTS={}ms, TTS_total={}ms, {} sentences)",
        total_ms, stt_ms, reply_ms, total_tts_ms, sentence_num
    );

    Ok(())
}

/// Find the end of a sentence in the buffer (position of .!? followed by space or end).
fn find_sentence_end(text: &str) -> Option<usize> {
    for (i, ch) in text.char_indices() {
        if (ch == '.' || ch == '!' || ch == '?') && i > 5 {
            // Check that next char is whitespace or end of string
            let next = text[i + ch.len_utf8()..].chars().next();
            if next.is_none() || next == Some(' ') || next == Some('\n') {
                return Some(i);
            }
        }
    }
    None
}
