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

use crate::routes::errors::ErrorResponse;
use crate::state::{build_kokoro_tts, AppState, SharedTts};
use crate::voice::provider::{SttConfig, TtsConfig};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use permagent::download_manager::{get_download_manager, DownloadProgress, DownloadStatus};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use subtle;

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new().route("/voice", get(voice_ws_handler).with_state(state))
}

// ── On-demand Kokoro voice-model downloader ────────────────────────────────
//
// The ~353MB Kokoro assets are NOT git-tracked or DMG-bundled, so a fresh
// install has voice TTS silently disabled (state::init_voice_providers finds
// no models). These authenticated endpoints fetch the assets on demand using
// the same DownloadManager that backs local-inference model downloads, then
// hot-swap a live TTS provider into the shared slot — no daemon restart.

/// Canonical kokoro-onnx model release (Apache-2.0). Filenames match the paths
/// `OrtKokoroModelPaths::default_paths()` expects under the voice models dir.
const KOKORO_MODEL_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";
const KOKORO_VOICES_URL: &str =
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";
/// kokoro-v1.0.onnx (325_532_387) + voices-v1.0.bin (28_214_398).
const KOKORO_TOTAL_BYTES: u64 = 353_746_785;
/// DownloadManager key for the Kokoro asset bundle.
const KOKORO_DOWNLOAD_ID: &str = "kokoro-voice";

/// Bearer-protected voice HTTP routes (the on-demand model downloader + the
/// standalone synth primitive), merged into the protected router group —
/// unlike the public `/voice` WS which does its own query-param auth.
pub fn http_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/voice/models", get(voice_models_status))
        .route(
            "/voice/models/download",
            post(download_voice_models)
                .get(voice_models_progress)
                .delete(cancel_voice_models_download),
        )
        .route("/voice/synthesize", post(synthesize_voice))
        .route("/api/voices", get(get_voices))
        .with_state(state)
}

/// Snapshot of voice-asset availability for the picker's graceful-absence path.
#[derive(Serialize)]
pub struct VoiceModelStatus {
    /// Both Kokoro asset files are present on disk.
    pub models_present: bool,
    /// A live TTS provider is loaded — synthesis/preview works right now.
    pub tts_loaded: bool,
    /// A download is currently in progress.
    pub downloading: bool,
}

async fn current_status(state: &Arc<AppState>) -> VoiceModelStatus {
    let models_present =
        crate::voice::ort_kokoro_backend::OrtKokoroModelPaths::default_paths().models_exist();
    let tts_loaded = state.voice_tts.read().await.is_some();
    let downloading = get_download_manager()
        .get_progress(KOKORO_DOWNLOAD_ID)
        .is_some_and(|p| p.status == DownloadStatus::Downloading);
    VoiceModelStatus {
        models_present,
        tts_loaded,
        downloading,
    }
}

/// GET /voice/models — is voice synthesis available, and is a download running?
async fn voice_models_status(State(state): State<Arc<AppState>>) -> Json<VoiceModelStatus> {
    Json(current_status(&state).await)
}

/// POST /voice/models/download — fetch the Kokoro assets on demand, then
/// hot-swap a live TTS provider into the shared slot when complete.
async fn download_voice_models(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<VoiceModelStatus>), ErrorResponse> {
    let paths = crate::voice::ort_kokoro_backend::OrtKokoroModelPaths::default_paths();

    // Already downloaded: ensure the live provider is loaded, then no-op.
    if paths.models_exist() {
        if state.voice_tts.read().await.is_none() {
            if let Some(tts) = build_kokoro_tts() {
                *state.voice_tts.write().await = Some(tts);
            }
        }
        return Ok((StatusCode::OK, Json(current_status(&state).await)));
    }

    let files = vec![
        (KOKORO_MODEL_URL.to_string(), paths.model_path.clone()),
        (KOKORO_VOICES_URL.to_string(), paths.voices_path.clone()),
    ];

    // Build the provider off the blocking pool on completion and swap it in so
    // /voice + synth work immediately, without a restart.
    let tts_slot: SharedTts = state.voice_tts.clone();
    let on_complete: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        tokio::spawn(async move {
            match tokio::task::spawn_blocking(build_kokoro_tts).await {
                Ok(Some(tts)) => {
                    *tts_slot.write().await = Some(tts);
                    tracing::info!(
                        target: "permagentd::voice",
                        "Kokoro TTS hot-loaded after on-demand download — voice enabled"
                    );
                }
                _ => tracing::error!(
                    target: "permagentd::voice",
                    "Kokoro download finished but TTS failed to load"
                ),
            }
        });
    });

    get_download_manager()
        .download_model_sharded(
            KOKORO_DOWNLOAD_ID.to_string(),
            files,
            KOKORO_TOTAL_BYTES,
            Some(on_complete),
        )
        .await
        .map_err(|e| {
            ErrorResponse::internal(format!("Voice model download failed to start: {e}"))
        })?;

    Ok((StatusCode::ACCEPTED, Json(current_status(&state).await)))
}

/// GET /voice/models/download — progress of the in-flight Kokoro download.
async fn voice_models_progress() -> Result<Json<DownloadProgress>, ErrorResponse> {
    get_download_manager()
        .get_progress(KOKORO_DOWNLOAD_ID)
        .map(Json)
        .ok_or_else(|| ErrorResponse::not_found("No active voice model download"))
}

/// DELETE /voice/models/download — cancel an in-flight download.
async fn cancel_voice_models_download() -> Result<StatusCode, ErrorResponse> {
    get_download_manager()
        .cancel_download(KOKORO_DOWNLOAD_ID)
        .map_err(|e| ErrorResponse::bad_request(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Standalone synth primitive ─────────────────────────────────────────────
//
// The one text→audio path that doesn't exist on the `/voice` WS (which only
// synthesizes the reply stream). ONE primitive, THREE consumers: per-voice
// sound preview, the spoken opening greeting, and the picker's audition tap.
// Returns a self-contained 16-bit PCM WAV so a fresh consumer can play it with
// `new Audio(URL.createObjectURL(blob))` — no manual AudioBuffer assembly.

#[derive(Deserialize)]
pub struct SynthesizeRequest {
    /// Text to speak.
    pub text: String,
    /// Voice pack key (e.g. "bf_emma"). `None` → the backend default.
    #[serde(default)]
    pub voice_id: Option<String>,
}

/// POST /voice/synthesize — synthesize `text` in `voice_id` and return WAV.
/// 503 when the Kokoro assets aren't loaded yet (the picker handles this by
/// offering the download affordance rather than a dead audition button).
async fn synthesize_voice(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SynthesizeRequest>,
) -> Result<impl IntoResponse, ErrorResponse> {
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(ErrorResponse::bad_request("text must not be empty"));
    }

    // Snapshot the hot-swappable provider; absent → graceful 503.
    let tts = state.voice_tts.read().await.clone().ok_or_else(|| {
        ErrorResponse::service_unavailable(
            "Voice models not loaded — download the voice assets to enable synthesis",
        )
    })?;

    let voice_id = req.voice_id.clone();
    let audio = tokio::task::spawn_blocking(move || {
        tts.synthesize(
            &text,
            &TtsConfig {
                voice_id,
                speed: 1.0,
                lexicon: None,
            },
        )
    })
    .await
    .map_err(|e| ErrorResponse::internal(format!("synthesis task panicked: {e}")))?
    .map_err(|e| ErrorResponse::internal(format!("synthesis failed: {e}")))?;

    let wav = encode_wav_pcm16(&audio)
        .map_err(|e| ErrorResponse::internal(format!("wav encode failed: {e}")))?;

    Ok(([(axum::http::header::CONTENT_TYPE, "audio/wav")], wav))
}

// ── Voice picker roster ────────────────────────────────────────────────────
//
// The picker is data-driven over the loaded pack's keys (all ~54 voices are
// free config entries — "add voices as we go" = zero code per voice). Keys are
// `{accent}{gender}_{name}` (e.g. "bf_emma" → British Female Emma), so we
// derive a friendly label rather than show cryptic ids.

/// One selectable voice for the picker.
#[derive(Serialize, PartialEq, Debug)]
pub struct VoiceInfo {
    /// Pack key, the value persisted to `persona.voice_id` (e.g. "bf_emma").
    pub id: String,
    /// Friendly label, e.g. "British Female — Emma".
    pub label: String,
    /// Language/accent group, e.g. "British English" (for picker grouping).
    pub language: String,
    /// "Female" / "Male" / "" when unknown.
    pub gender: String,
}

/// Decode a Kokoro voice key prefix into (language, gender, display-name).
fn describe_voice_id(id: &str) -> (String, String, String) {
    let (prefix, raw_name) = id.split_once('_').unwrap_or(("", id));
    let mut chars = prefix.chars();
    let accent = match chars.next() {
        Some('a') => "American English",
        Some('b') => "British English",
        Some('e') => "Spanish",
        Some('f') => "French",
        Some('h') => "Hindi",
        Some('i') => "Italian",
        Some('j') => "Japanese",
        Some('p') => "Portuguese",
        Some('z') => "Chinese",
        _ => "",
    };
    let gender = match chars.next() {
        Some('f') => "Female",
        Some('m') => "Male",
        _ => "",
    };
    // Capitalize the first letter of the name (e.g. "emma" → "Emma").
    let name = {
        let mut c = raw_name.chars();
        match c.next() {
            Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
            None => raw_name.to_string(),
        }
    };
    (accent.to_string(), gender.to_string(), name)
}

impl VoiceInfo {
    fn from_id(id: String) -> Self {
        let (language, gender, name) = describe_voice_id(&id);
        // "British English Female — Emma", trimming any empty leading parts.
        let prefix = [language.as_str(), gender.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let label = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix} — {name}")
        };
        VoiceInfo {
            id,
            label,
            language,
            gender,
        }
    }
}

/// Pure transform (kept separate from the handler so it's unit-testable
/// without constructing an AppState): pack keys → sorted picker roster.
fn voices_from_ids(ids: Vec<String>) -> Vec<VoiceInfo> {
    ids.into_iter().map(VoiceInfo::from_id).collect()
}

/// GET /api/voices — the picker roster from the loaded pack. Empty when the
/// assets aren't downloaded yet (the picker pairs this with `/voice/models`
/// to offer the download affordance instead of a dead list).
async fn get_voices(State(state): State<Arc<AppState>>) -> Json<Vec<VoiceInfo>> {
    let ids = match state.voice_tts.read().await.clone() {
        Some(tts) => tts.list_voices(),
        None => Vec::new(),
    };
    Json(voices_from_ids(ids))
}

/// Encode mono f32 PCM `[-1, 1]` samples as a 16-bit PCM WAV (universally
/// decodable by WKWebView's `<audio>` / `decodeAudioData`).
fn encode_wav_pcm16(audio: &crate::voice::provider::AudioOutput) -> anyhow::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &s in &audio.samples {
            let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            writer.write_sample(v)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
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
    /// Deferred navigation for the speak-then-act seam. Sent AFTER all narration
    /// audio for the turn so the client fires it only once playback drains —
    /// otherwise the view switches before the agent finishes saying it will.
    #[serde(rename = "navigate")]
    Navigate {
        tab: String,
        tool_type: String,
        panel_type: String,
        section: Option<String>,
        state: Option<serde_json::Value>,
        reason: String,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "ready")]
    Ready,
}

/// RAII cleanup for navigation interception: guarantees the session's entry is
/// removed from the registry on every exit path (including mid-stream client
/// disconnect), so a dropped voice turn can never leave a stale interceptor that
/// would swallow a later text-turn navigation for the same session.
struct NavInterceptGuard(String);

impl Drop for NavInterceptGuard {
    fn drop(&mut self) {
        let _ = permagent::events::nav_intercept::take(&self.0);
    }
}

fn send_json(msg: &ServerMessage) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn handle_voice_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    session_id: Option<String>,
) {
    // Snapshot the hot-swappable TTS slot once for this session.
    let tts_opt = state.voice_tts.read().await.clone();

    tracing::info!(
        target: "permagentd::voice",
        "Voice WebSocket connected (session_id={:?}, stt={}, tts={})",
        session_id,
        state.voice_stt.is_some(),
        tts_opt.is_some()
    );

    // Check voice providers are available
    let (stt, tts) = match (&state.voice_stt, &tts_opt) {
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

    // Load proper-noun dictionary from Brain for post-STT correction.
    let entity_dict = if let Some(ref brain) = state.brain {
        let brain = brain.clone();
        let dict = tokio::task::spawn_blocking(move || {
            let names = crate::voice::proper_noun_corrector::load_entity_names_blocking(&brain);
            crate::voice::proper_noun_corrector::EntityDictionary::new(names)
        })
        .await
        .unwrap_or_else(|_| {
            crate::voice::proper_noun_corrector::EntityDictionary::new(
                std::collections::HashSet::new(),
            )
        });
        tracing::info!(
            target: "permagentd::voice",
            "Loaded {} entity names for STT proper-noun correction",
            dict.len()
        );
        dict
    } else {
        crate::voice::proper_noun_corrector::EntityDictionary::new(std::collections::HashSet::new())
    };

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

                        // Post-STT proper-noun correction against Brain entities
                        let transcript = crate::voice::proper_noun_corrector::correct_proper_nouns(
                            &transcript,
                            &entity_dict,
                        );

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

    // Resolve the configured voice once for this reply (persona.voice_id),
    // reused for every synthesized sentence. `None` lets the backend fall back
    // to its default voice. Read once here rather than per-sentence to avoid
    // taking the persona lock inside the streaming loop.
    let voice_id = state.persona.read().await.voice_id.clone();

    let t_setup = std::time::Instant::now();

    let sid = if let Some(id) = ctx.session_id {
        id.to_string()
    } else {
        // Lean projection — we only need the most-recent session's id.
        let sessions = state.session_manager().list_session_summaries().await?;
        sessions
            .first()
            .map(|s| s.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No session available for voice"))?
    };

    // Speak-then-act: intercept this turn's navigations so `navigate_app` hands
    // them off here (instead of emitting to the global bus for instant switch).
    // We forward them down THIS socket after the narration, and the client fires
    // them only once audio playback drains. The guard removes the interceptor on
    // every exit path, including mid-stream disconnect.
    permagent::events::nav_intercept::begin(&sid);
    let _nav_guard = NavInterceptGuard(sid.clone());

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
             questions. Speak as you would in a real conversation. \
             Verbalize outcomes, not interface mechanics: don't read out UI labels, button names, \
             menu paths, file paths, URLs, or settings keys, and don't narrate the individual \
             steps you take to do something. Say what happened or what the user should do in plain \
             spoken terms — e.g. 'I turned on web search' rather than 'I clicked the Search and \
             tools toggle in Settings'. If a literal name, path, or value is essential, give just \
             that one item, not the surrounding navigation."
                .to_string(),
        )
        .await;

    let setup_ms = t_setup.elapsed().as_millis();

    // Ambient context (probe-only) and query-driven recall are independent and
    // both hit the Brain — run them CONCURRENTLY so the pre-stream budget is the
    // slower of the two, not their sum. They inject under different system-prompt
    // keys ("ambient_context" vs "memory_recall") so there's no write conflict.
    let t_ctx = std::time::Instant::now();
    let ambient_fut = crate::brain_ops::inject_ambient_context(state, &agent);
    let recall_hits = if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        let recognition_pool = state.session_manager().pool_clone().await.ok();
        let recall_fut = crate::brain_ops::inject_recall(
            brain,
            &agent,
            transcript,
            recognition_ctx,
            recognition_pool,
        );
        let (_, n) = tokio::join!(ambient_fut, recall_fut);
        n
    } else {
        ambient_fut.await;
        0
    };
    let ctx_recall_ms = t_ctx.elapsed().as_millis();
    tracing::info!(target: "permagentd::voice", "  ctx+recall: {}ms ({} recall hits)", ctx_recall_ms, recall_hits);

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
        "  pipeline: setup={}ms ctx+recall={}ms reply_setup={}ms (total pre-stream={}ms)",
        setup_ms, ctx_recall_ms, reply_setup_ms,
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
                        let voice_id = voice_id.clone();
                        let tts_start = std::time::Instant::now();
                        let audio = tokio::task::spawn_blocking(move || {
                            // Check cancellation before acquiring the mutex
                            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                return Err(anyhow::anyhow!("cancelled"));
                            }
                            tts_ref.synthesize(
                                &sent,
                                &TtsConfig {
                                    voice_id,
                                    ..TtsConfig::default()
                                },
                            )
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
        let voice_id = voice_id.clone();
        let tts_start = std::time::Instant::now();
        let audio = tokio::task::spawn_blocking(move || {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(anyhow::anyhow!("cancelled"));
            }
            tts_ref.synthesize(
                &remainder,
                &TtsConfig {
                    voice_id,
                    ..TtsConfig::default()
                },
            )
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

    // Forward any navigations captured during this turn AFTER all narration
    // audio — they ride this ordered socket, so by the time the client sees them
    // every audio chunk is already queued. The client fires them only once the
    // audio queue drains, so the view switches when the agent stops speaking.
    for nav in permagent::events::nav_intercept::take(&sid) {
        let _ = socket
            .send(send_json(&ServerMessage::Navigate {
                tab: nav.tab,
                tool_type: nav.tool_type,
                panel_type: nav.panel_type,
                section: nav.section,
                state: nav.state,
                reason: nav.reason,
            }))
            .await;
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::download_manager::DownloadManager;

    #[test]
    fn voice_roster_labels_and_sorting() {
        let roster = voices_from_ids(vec![
            "bf_emma".to_string(),
            "am_michael".to_string(),
            "bm_lewis".to_string(),
            "af_bella".to_string(),
        ]);

        // Friendly labels derived from the `{accent}{gender}_{name}` key.
        let by_id = |id: &str| roster.iter().find(|v| v.id == id).unwrap();
        assert_eq!(by_id("bf_emma").label, "British English Female — Emma");
        assert_eq!(by_id("am_michael").label, "American English Male — Michael");
        assert_eq!(by_id("af_bella").language, "American English");
        assert_eq!(by_id("bm_lewis").gender, "Male");

        // Unknown prefixes degrade gracefully to a bare capitalized name.
        let weird = VoiceInfo::from_id("xx_zephyr".to_string());
        assert_eq!(weird.label, "Zephyr");
        assert_eq!(weird.language, "");
    }

    /// End-to-end proof that the shipping Kokoro release URL fetches via the
    /// production `DownloadManager` and lands the canonical asset. Uses the
    /// smaller voices file (28MB) and a tempdir — never the real models path.
    ///
    /// `#[ignore]`d so CI never pulls 28MB over the network; run explicitly:
    ///   cargo test -p permagent-daemon --lib voice:: -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn kokoro_voices_url_downloads_via_download_manager() {
        const VOICES_BYTES: u64 = 28_214_398;
        let dir = std::env::temp_dir().join(format!("kokoro-dl-test-{}", std::process::id()));
        let dest = dir.join("voices-v1.0.bin");
        let _ = std::fs::remove_dir_all(&dir);

        let dm = DownloadManager::new();
        dm.download_model_sharded(
            "kokoro-voices-test".to_string(),
            vec![(KOKORO_VOICES_URL.to_string(), dest.clone())],
            VOICES_BYTES,
            None,
        )
        .await
        .expect("download should start");

        loop {
            let p = dm
                .get_progress("kokoro-voices-test")
                .expect("progress should exist");
            assert_ne!(
                p.status,
                DownloadStatus::Failed,
                "download failed: {:?}",
                p.error
            );
            if p.status == DownloadStatus::Completed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        let meta = std::fs::metadata(&dest).expect("voices file should be present");
        assert_eq!(
            meta.len(),
            VOICES_BYTES,
            "downloaded voices file size mismatch"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
