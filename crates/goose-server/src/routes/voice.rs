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
    // Cancellation flag: set when the socket closes to abort in-flight TTS work.
    // Prevents a stale handler from holding the TTS mutex while a new handler
    // starts on a reconnected socket.
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

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
                tracing::debug!(target: "permagentd::voice", "Received text: {}", truncate_str(text_str, 100));
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

                        // Skip STT for empty or too-short buffers (< 0.3s).
                        // Prevents wasting 25s running STT on silence from
                        // quick press-release or capture failures.
                        let min_samples = (client_sample_rate as f32 * 0.3) as usize;
                        if audio_buffer.len() < min_samples {
                            tracing::info!(
                                target: "permagentd::voice",
                                "Skipping STT: buffer too short ({} samples, {:.2}s < 0.3s minimum)",
                                audio_buffer.len(), audio_duration_s
                            );
                            audio_buffer.clear();
                            // Tell the client the exchange is done so it returns to 'ready'
                            // (without this, the client hangs in 'processing' forever).
                            let _ = socket
                                .send(send_json(&ServerMessage::Error {
                                    message: "Recording too short — hold longer to speak".into(),
                                }))
                                .await;
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
                            stt_ms, truncate_str(&transcript, 80)
                        );

                        if transcript.is_empty() {
                            // Tell the client so it returns to 'ready'
                            let _ = socket
                                .send(send_json(&ServerMessage::Error {
                                    message: "No speech detected — try again".into(),
                                }))
                                .await;
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

                        let reply_ctx = VoiceReplyCtx {
                            state: &state,
                            transcript: &transcript,
                            session_id: session_id.as_deref(),
                            tts: &tts,
                            pipeline_start,
                            stt_ms,
                            cancelled: cancelled.clone(),
                        };
                        let stream_result = stream_reply_with_tts(&reply_ctx, &mut socket).await;

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
    // Signal cancellation so any in-flight TTS spawn_blocking tasks skip the mutex
    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!(target: "permagentd::voice", "Voice WebSocket handler exiting (cancelled=true)");
}

/// Truncate a string at a char boundary for safe logging.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => s.get(..i).unwrap_or(s),
        None => s,
    }
}

/// Context for a voice reply exchange (reduces arg count for stream_reply_with_tts).
struct VoiceReplyCtx<'a> {
    state: &'a AppState,
    transcript: &'a str,
    session_id: Option<&'a str>,
    tts: &'a Arc<dyn crate::voice::TextToSpeech>,
    pipeline_start: std::time::Instant,
    stt_ms: u128,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Stream the LLM reply, synthesize each sentence as it completes, and send
/// audio chunks to the client immediately. This way the client starts playing
/// sentence 1 while sentence 2 is still generating.
async fn stream_reply_with_tts(
    ctx: &VoiceReplyCtx<'_>,
    socket: &mut WebSocket,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use permagent::agents::{AgentEvent, SessionConfig};
    use permagent::conversation::message::{Message as ChatMessage, MessageContent};

    let state = ctx.state;
    let transcript = ctx.transcript;
    let tts = ctx.tts;
    let pipeline_start = ctx.pipeline_start;
    let stt_ms = ctx.stt_ms;
    let cancelled = &ctx.cancelled;

    let t_setup = std::time::Instant::now();

    let sid = if let Some(id) = ctx.session_id {
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

    let setup_ms = t_setup.elapsed().as_millis();

    // Ambient context — same as text chat (project focus, probed/recalled memories)
    let t_ctx = std::time::Instant::now();
    crate::brain_ops::inject_ambient_context(state, &agent, transcript).await;
    let ctx_ms = t_ctx.elapsed().as_millis();

    let t_recall = std::time::Instant::now();
    if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        let n = crate::brain_ops::inject_recall(brain, &agent, transcript, recognition_ctx).await;
        tracing::info!(target: "permagentd::voice", "  recall: {}ms ({} hits)", t_recall.elapsed().as_millis(), n);
    }
    let recall_ms = t_recall.elapsed().as_millis();

    let session_config = SessionConfig {
        id: sid.clone(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    };

    let t_reply = std::time::Instant::now();
    let mut stream = agent.reply(user_msg, session_config, None).await?;
    let reply_setup_ms = t_reply.elapsed().as_millis();
    tracing::info!(
        target: "permagentd::voice",
        "  pipeline: setup={}ms ctx={}ms recall={}ms reply_setup={}ms (total pre-stream={}ms)",
        setup_ms, ctx_ms, recall_ms, reply_setup_ms,
        pipeline_start.elapsed().as_millis()
    );

    // Accumulate text, detect phrase/sentence boundaries, synthesize + send each chunk
    let mut text_buf = String::new();
    let mut full_reply = String::new();
    let mut sentence_num = 0u32;
    let mut total_tts_ms: u128 = 0;
    let mut first_audio_sent = false;
    let mut first_token_logged = false;
    let stream_start = std::time::Instant::now();

    while let Some(event) = stream.next().await {
        if let Ok(AgentEvent::Message(msg)) = event {
            if msg.role != rmcp::model::Role::Assistant {
                continue;
            }
            for content in &msg.content {
                if let MessageContent::Text(text_content) = content {
                    if !first_token_logged {
                        tracing::info!(
                            target: "permagentd::voice",
                            "  TTFT: {}ms after stream start",
                            stream_start.elapsed().as_millis()
                        );
                        first_token_logged = true;
                    }
                    text_buf.push_str(&text_content.text);
                    full_reply.push_str(&text_content.text);

                    // Check for speakable boundary (clause or sentence)
                    // find_speakable_boundary returns (byte_end_inclusive, byte_start_of_next)
                    while let Some((end_inclusive, next_start)) = find_speakable_boundary(&text_buf)
                    {
                        let sentence = text_buf
                            .get(..=end_inclusive)
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        text_buf = text_buf.get(next_start..).unwrap_or("").to_string();

                        if sentence.is_empty() {
                            continue;
                        }

                        sentence_num += 1;

                        // Guard: skip TTS if the socket closed (prevents
                        // stale handlers from holding the TTS mutex while
                        // a new handler starts on a reconnected socket).
                        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                            tracing::info!(target: "permagentd::voice", "TTS cancelled — skipping sentence {}", sentence_num);
                            return Ok(());
                        }

                        let tts_ref = tts.clone();
                        let sent = sentence.clone();
                        let cancel_flag = cancelled.clone();
                        let tts_start = std::time::Instant::now();
                        let audio = tokio::task::spawn_blocking(move || {
                            // Check cancellation before acquiring the mutex
                            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                return Err(anyhow::anyhow!("cancelled"));
                            }
                            tts_ref.synthesize(&sent, &TtsConfig::default())
                        })
                        .await;
                        let chunk_tts_ms = tts_start.elapsed().as_millis();
                        total_tts_ms += chunk_tts_ms;

                        // Check cancellation after TTS (socket may have closed during synthesis)
                        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                            tracing::info!(target: "permagentd::voice", "TTS cancelled after sentence {}", sentence_num);
                            return Ok(());
                        }

                        match audio {
                            Ok(Ok(audio)) => {
                                let dur = audio.samples.len() as f32 / audio.sample_rate as f32;
                                let rtf = chunk_tts_ms as f32 / 1000.0 / dur.max(0.01);
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "STREAM sentence {}: {}chars TTS={}ms audio={:.1}s RTF={:.2}x | \"{}\"",
                                    sentence_num, sentence.len(), chunk_tts_ms, dur, rtf,
                                    truncate_str(&sentence, 60)
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
                                let msg = e.to_string();
                                if msg == "cancelled" {
                                    tracing::info!(target: "permagentd::voice", "TTS cancelled (pre-mutex)");
                                    return Ok(());
                                }
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
    if !remainder.is_empty() && !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        sentence_num += 1;
        let tts_ref = tts.clone();
        let cancel_flag = cancelled.clone();
        let tts_start = std::time::Instant::now();
        let audio = tokio::task::spawn_blocking(move || {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(anyhow::anyhow!("cancelled"));
            }
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

    let reply_ms = t_reply.elapsed().as_millis();
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

    // Persist voice turn to Brain — same as text chat, so future recall
    // can surface what was discussed via voice.
    if let Some(ref brain) = state.brain {
        if !transcript.is_empty() && !full_reply.is_empty() {
            // Use a simple turn index based on timestamp (voice doesn't track conversation length)
            let turn_idx = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as usize)
                .unwrap_or(0);
            crate::brain_ops::spawn_persist_chat_turn(
                brain.clone(),
                sid.to_string(),
                turn_idx,
                transcript.to_string(),
                full_reply,
            );
        }
    }

    Ok(())
}

/// Find the earliest speakable boundary in the buffer.
///
/// Returns `(end_inclusive, next_start)` — both valid byte indices.
/// `end_inclusive` is the last byte of the boundary char (for `text[..=end]`).
/// `next_start` is the first byte after the boundary char (for `text[next..]`).
///
/// Priority: sentence boundaries (.!?) first, then clause boundaries (, ; — :)
/// once enough text has accumulated.
///
/// Minimum lengths prevent choppy fragments:
/// - Sentence boundary (.!?): 6+ chars
/// - Clause boundary (, ; — :): 25+ chars
fn find_speakable_boundary(text: &str) -> Option<(usize, usize)> {
    // First pass: sentence boundary (strongest break, lowest minimum)
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if (ch == '.' || ch == '!' || ch == '?') && i > 5 {
            let after = iter.peek().map(|(_, c)| *c);
            if after.is_none() || after == Some(' ') || after == Some('\n') {
                return Some((i, i + ch.len_utf8()));
            }
        }
    }

    // Second pass: clause boundary (weaker break, higher minimum)
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if i < 25 {
            continue;
        }
        let next_byte = i + ch.len_utf8();
        let is_clause = match ch {
            ',' | ';' => {
                let after = iter.peek().map(|(_, c)| *c);
                after == Some(' ') || after == Some('\n')
            }
            '\u{2014}' => true, // em dash
            ':' => {
                let after = iter.peek().map(|(_, c)| *c);
                after == Some(' ')
            }
            _ => false,
        };
        if is_clause {
            return Some((i, next_byte));
        }
    }

    None
}
