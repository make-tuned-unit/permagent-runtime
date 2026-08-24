//! Dedicated `/voice` WebSocket for the push-to-talk voice loop.
//!
//! Protocol (Phase 1, non-streaming):
//!   Client → Server:
//!     Text: {"type":"start","sample_rate":16000}
//!     Binary: [pcm_f32le audio chunks while push-to-talk held]
//!     Text: {"type":"stop"}
//!     Text: {"type":"wake_start","sample_rate":16000}   (hands-free: begin keyword spotting)
//!     Text: {"type":"wake_stop"}
//!   Server → Client:
//!     Text: {"type":"transcript","text":"..."}
//!     Text: {"type":"reply_start"}
//!     Binary: [tts pcm_f32le audio]
//!     Text: {"type":"clipboard","text":"..."}  (as soon as copy_to_clipboard
//!            runs — not after TTS — so the phone can write the pasteboard
//!            before the user switches to Notes)
//!     Text: {"type":"reply_text","text":"..."}
//!     Text: {"type":"navigate",...}            (after narration; desktop only)
//!     Text: {"type":"reply_end","sample_rate":24000}
//!     Text: {"type":"error","message":"..."}
//!     Text: {"type":"wake_status","active":true,"phrase":"Hey Henry"}
//!     Text: {"type":"wake","kind":"wake"|"stop"}        (keyword detected)
//!     Text: {"type":"stopped"}                          (spoken stop cancelled the in-flight turn)
//!
//! Wake mode: while no recording is active, binary frames are mic MONITOR
//! audio fed to the on-device keyword spotter (voice::kws) — never to STT,
//! never off-machine. Detections come back as `wake` events; a stop phrase
//! that lands while a reply is still being generated cancels the turn
//! server-side and is announced with `stopped`.

use crate::routes::errors::ErrorResponse;
use crate::state::{build_kokoro_tts, AppState, SharedTts};
use crate::voice::provider::{AudioOutput, SttConfig, TtsConfig};
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

// ── User pronunciation lexicon (#516 follow-through: the never-spell rule) ──

#[derive(serde::Deserialize)]
struct SavePronunciationRequest {
    word: String,
    /// Phonemes, OPTIONAL. Omit it: the daemon derives them from `sounds_like`
    /// using the very G2P that speaks. Supplying IPA by hand is an escape hatch
    /// for someone who genuinely knows Kokoro's alphabet, not the normal path —
    /// see `phonemize_text` for why hand-authored IPA cannot be trusted.
    #[serde(default)]
    ipa: Option<String>,
    sounds_like: String,
}

/// GET /voice/pronunciations — every saved pronunciation.
async fn list_pronunciations(
) -> Json<std::collections::HashMap<String, crate::voice::user_lexicon::PronunciationEntry>> {
    Json(crate::voice::user_lexicon::all())
}

/// GET /voice/pronunciations/unresolved — words synthesis had to spell out.
///
/// The review queue: every word that missed the dictionary, the lexicon AND the
/// compound splitter, so it was spelled letter by letter. Ordered most-frequent
/// first, which is the order worth teaching them in.
async fn unresolved_pronunciations() -> Json<serde_json::Value> {
    let items: Vec<serde_json::Value> = crate::voice::oov_log::snapshot()
        .into_iter()
        .map(|(word, count)| serde_json::json!({ "word": word, "spelled_out_times": count }))
        .collect();
    Json(serde_json::json!({ "unresolved": items }))
}

/// PUT /voice/pronunciations — upsert one; effective on the very next
/// synthesized sentence (the lexicon is re-read per call).
///
/// The respelling is the source of truth. When `ipa` is absent it is DERIVED by
/// running `sounds_like` through the live TTS backend's G2P, which makes the
/// stored phonemes exactly what the engine will say. That closes the loop
/// hand-written IPA left open: the only entry ever saved that way stored
/// "permagent" as "pʌmˈeɪdʒənt" / "PUM-ay-jent" — internally consistent and
/// confidently wrong (it is "PER-ma-jent"), with nothing able to detect it.
async fn save_pronunciation_route(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SavePronunciationRequest>,
) -> Result<Json<serde_json::Value>, ErrorResponse> {
    let supplied = req.ipa.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let (ipa, derived) = match supplied {
        Some(ipa) => (ipa.to_string(), false),
        None => {
            if req.sounds_like.trim().is_empty() {
                return Err(ErrorResponse::bad_request(
                    "sounds_like is required — respell the word using ordinary English words or \
                     syllables, e.g. 'prop tech'",
                ));
            }
            let tts = state.voice_tts.read().await.clone().ok_or_else(|| {
                ErrorResponse::service_unavailable(
                    "Voice models not loaded — cannot derive pronunciation from a respelling yet",
                )
            })?;
            let respelling = req.sounds_like.clone();
            // G2P holds a std Mutex and loads no I/O; still, keep it off the
            // async runtime so a long lock wait cannot stall the reactor.
            let phonemes = tokio::task::spawn_blocking(move || tts.phonemize_text(&respelling))
                .await
                .map_err(|e| ErrorResponse::internal(format!("phonemize task panicked: {e}")))?
                .map_err(|e| ErrorResponse::bad_request(e.to_string()))?;
            (phonemes, true)
        }
    };

    let count = crate::voice::user_lexicon::save(
        &req.word,
        crate::voice::user_lexicon::PronunciationEntry {
            ipa: ipa.clone(),
            sounds_like: req.sounds_like,
        },
    )
    .map_err(ErrorResponse::bad_request)?;
    // Taught — drop it from the outstanding queue.
    crate::voice::oov_log::forget(&req.word);
    tracing::info!(
        target: "permagentd::voice",
        word = %req.word, %ipa, derived, "pronunciation saved"
    );
    Ok(Json(serde_json::json!({
        "saved": true, "total": count, "ipa": ipa, "derived_from_respelling": derived
    })))
}

/// DELETE /voice/pronunciations/{word}
async fn delete_pronunciation(
    axum::extract::Path(word): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ErrorResponse> {
    let removed = crate::voice::user_lexicon::remove(&word).map_err(ErrorResponse::bad_request)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

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
const KOKORO_MODEL_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";
const KOKORO_VOICES_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";
/// Pinned SHA-256 digests for the Kokoro release assets. These files are fed
/// straight into onnxruntime's native parser, so their integrity is a hard
/// requirement — the DownloadManager refuses to install bytes that don't
/// match. Digests verified 2026-07 from two independent sources: hashing the
/// canonical GitHub release assets themselves, and the LFS oids of the
/// huggingface.co/leonelhs/kokoro-thewh1teagle mirror (sizes match the byte
/// counts below exactly).
const KOKORO_MODEL_SHA256: &str =
    "7d5df8ecf7d4b1878015a32686053fd0eebe2bc377234608764cc0ef3636a6c5";
const KOKORO_VOICES_SHA256: &str =
    "bca610b8308e8d99f32e6fe4197e7ec01679264efed0cac9140fe9c29f1fbf7d";
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
        .route("/voice/wake/models", get(wake_models_status))
        .route("/voice/wake/models/download", post(download_wake_models))
        .route(
            "/voice/pronunciations",
            axum::routing::get(list_pronunciations).put(save_pronunciation_route),
        )
        .route(
            "/voice/pronunciations/unresolved",
            get(unresolved_pronunciations),
        )
        .route(
            "/voice/pronunciations/{word}",
            axum::routing::delete(delete_pronunciation),
        )
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
        permagent::download_manager::DownloadFile::new(
            KOKORO_MODEL_URL,
            paths.model_path.clone(),
            Some(KOKORO_MODEL_SHA256.to_string()),
        ),
        permagent::download_manager::DownloadFile::new(
            KOKORO_VOICES_URL,
            paths.voices_path.clone(),
            Some(KOKORO_VOICES_SHA256.to_string()),
        ),
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

// ── On-demand wake-word (KWS) model downloader ─────────────────────────────
//
// Same shape as the Kokoro flow, sized down: a single ~17MB pinned release
// tarball, extracted next to the other voice models, then a live spotter is
// hot-swapped into the shared slot — no restart. Also triggered implicitly by
// a `wake_start` on the voice socket when the model isn't present, so the
// first hands-free activation provisions itself.

/// Snapshot of wake-word model availability.
#[derive(Serialize)]
pub struct WakeModelStatus {
    pub models_present: bool,
    pub spotter_loaded: bool,
    pub downloading: bool,
}

async fn current_wake_status(state: &Arc<AppState>) -> WakeModelStatus {
    let models_present = crate::voice::kws::WakeWordModelPaths::default_paths().models_exist();
    let spotter_loaded = state.wake_spotter.read().await.is_some();
    let downloading = get_download_manager()
        .get_progress(crate::voice::kws::KWS_DOWNLOAD_ID)
        .is_some_and(|p| p.status == DownloadStatus::Downloading);
    WakeModelStatus {
        models_present,
        spotter_loaded,
        downloading,
    }
}

/// GET /voice/wake/models — is wake-word detection available?
async fn wake_models_status(State(state): State<Arc<AppState>>) -> Json<WakeModelStatus> {
    Json(current_wake_status(&state).await)
}

/// Start the KWS model download (idempotent: no-ops when the model is present,
/// loading the spotter if needed; the DownloadManager dedupes an in-flight
/// fetch by id). Shared by the HTTP endpoint and the `wake_start` auto-fetch.
async fn start_wake_model_download(state: &Arc<AppState>) -> anyhow::Result<()> {
    let paths = crate::voice::kws::WakeWordModelPaths::default_paths();

    if paths.models_exist() {
        if state.wake_spotter.read().await.is_none() {
            let spotter = tokio::task::spawn_blocking(crate::state::build_wake_spotter).await?;
            if let Some(spotter) = spotter {
                *state.wake_spotter.write().await = Some(spotter);
            }
        }
        return Ok(());
    }

    let files = vec![permagent::download_manager::DownloadFile::new(
        crate::voice::kws::KWS_MODEL_URL,
        paths.tarball_path(),
        Some(crate::voice::kws::KWS_MODEL_SHA256.to_string()),
    )];

    let slot = state.wake_spotter.clone();
    let on_complete: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        tokio::spawn(async move {
            let loaded = tokio::task::spawn_blocking(move || {
                let paths = crate::voice::kws::WakeWordModelPaths::default_paths();
                crate::voice::kws::install_from_tarball(&paths)?;
                anyhow::Ok(crate::state::build_wake_spotter())
            })
            .await;
            match loaded {
                Ok(Ok(Some(spotter))) => {
                    *slot.write().await = Some(spotter);
                    tracing::info!(
                        target: "permagentd::voice",
                        "Wake-word model installed and hot-loaded — say the wake phrase to talk"
                    );
                }
                Ok(Ok(None)) => tracing::error!(
                    target: "permagentd::voice",
                    "Wake-word model installed but the spotter failed to load"
                ),
                Ok(Err(e)) => tracing::error!(
                    target: "permagentd::voice",
                    "Wake-word model install failed: {e}"
                ),
                Err(e) => tracing::error!(
                    target: "permagentd::voice",
                    "Wake-word model install task panicked: {e}"
                ),
            }
        });
    });

    get_download_manager()
        .download_model_sharded(
            crate::voice::kws::KWS_DOWNLOAD_ID.to_string(),
            files,
            crate::voice::kws::KWS_MODEL_BYTES,
            Some(on_complete),
        )
        .await?;
    Ok(())
}

/// POST /voice/wake/models/download — fetch the wake-word model on demand.
async fn download_wake_models(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<WakeModelStatus>), ErrorResponse> {
    start_wake_model_download(&state).await.map_err(|e| {
        ErrorResponse::internal(format!("Wake model download failed to start: {e}"))
    })?;
    let status = current_wake_status(&state).await;
    let code = if status.spotter_loaded {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((code, Json(status)))
}

// ── Standalone synth primitive ─────────────────────────────────────────────
//
// The one text→audio path that doesn't exist on the `/voice` WS (which only
// synthesizes the reply stream). ONE primitive, THREE consumers: per-voice
// sound preview, the spoken opening greeting, and the picker's audition tap.
// Returns a self-contained 16-bit PCM WAV. Consumers MUST decode it through the
// Web Audio API (`AudioContext.decodeAudioData`), NOT `new Audio(blobUrl)` —
// WKWebView (the shipping shell) has no HTMLMediaElement backend for blob URLs
// and rejects `.play()` with "operation not supported" (#385). See
// `useVoicePreview` in `ui/command-center/src/lib/useVoices.ts` for the path.

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
    let plan = crate::voice::prosody::plan(&text);
    let speech = crate::voice::speakable::speakable(&plan.speech)
        .ok_or_else(|| ErrorResponse::bad_request("text has nothing speakable"))?;
    let speed = plan.speed;
    let audio = tokio::task::spawn_blocking(move || -> anyhow::Result<AudioOutput> {
        let mut audio = tts.synthesize(
            &speech,
            &TtsConfig {
                voice_id,
                speed,
                lexicon: crate::voice::user_lexicon::current(),
            },
        )?;
        crate::voice::loudness::master(&mut audio.samples, audio.sample_rate, &speech);
        Ok(audio)
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
    /// `ios_voice` | `watch_voice` | `desktop_voice`. Optional — a paired
    /// device named iPhone still resolves as iOS when this is omitted.
    client: Option<String>,
}

fn voice_origin_from_query(
    state: &AppState,
    query: &VoiceQuery,
    principal: &crate::middleware::auth::StreamPrincipal,
) -> permagent::events::voice_origin::VoiceOrigin {
    use crate::middleware::auth::{AuthPrincipal, StreamPrincipal};
    let device_name = match principal {
        StreamPrincipal::Long(AuthPrincipal::Device(id)) => {
            state.device_registry.get(id).map(|v| v.name)
        }
        _ => None,
    };
    permagent::events::voice_origin::VoiceOrigin::resolve(
        query.client.as_deref(),
        device_name.as_deref(),
    )
}

async fn voice_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<VoiceQuery>,
) -> Result<impl IntoResponse, axum::http::StatusCode> {
    // Manual token validation — WebSocket upgrade can't use Bearer middleware.
    // Shared fail-closed, constant-time core (middleware::auth): a tokenless
    // daemon refuses (503) instead of serving the voice socket anonymously.
    let principal = crate::middleware::auth::authenticate_stream_token(
        &state,
        query.token.as_deref(),
        query.token.as_deref(),
    )?;
    let origin = voice_origin_from_query(&state, &query, &principal);
    let session_id = query.session_id;
    Ok(ws.on_upgrade(move |socket| handle_voice_socket(socket, state, session_id, origin)))
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "start")]
    Start { sample_rate: Option<u32> },
    #[serde(rename = "stop")]
    Stop,
    /// Enter wake-listening: subsequent binary frames (outside a recording)
    /// feed the on-device keyword spotter.
    #[serde(rename = "wake_start")]
    WakeStart { sample_rate: Option<u32> },
    #[serde(rename = "wake_stop")]
    WakeStop,
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
    /// Paste-ready body for the listening device. Sent as soon as
    /// `copy_to_clipboard` returns — not after TTS — because the user often
    /// switches to Notes the moment they hear a copy is happening, and iOS
    /// drops pasteboard writes from a backgrounded app.
    #[serde(rename = "clipboard")]
    Clipboard { text: String },
    #[serde(rename = "error")]
    Error { message: String },
    /// Empty or too-short capture — return to ready with no toast.
    /// Last night (20260821_14) every `transcript: ""` flashed
    /// "No speech detected — try again" on the orb.
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "ready")]
    Ready,
    /// Wake-listening state after a `wake_start`/`wake_stop`. `phrase` is the
    /// human-readable wake phrase (e.g. "Hey Henry") for the UI hint; `reason`
    /// explains inactivity ("downloading" while the model fetch runs).
    #[serde(rename = "wake_status")]
    WakeStatus {
        active: bool,
        phrase: Option<String>,
        reason: Option<String>,
    },
    /// A keyword fired: kind "wake" (open a turn) or "stop" (halt playback).
    #[serde(rename = "wake")]
    Wake { kind: String },
    /// A spoken stop cancelled the in-flight reply server-side — no more
    /// audio is coming for this turn.
    #[serde(rename = "stopped")]
    Stopped,
    /// Show this word on the Orb. Never spoken — the user reads it and says it.
    #[serde(rename = "teach")]
    Teach { word: String },
    /// The word was stored (or skipped). Clear the Orb placement.
    #[serde(rename = "taught")]
    Taught { word: String },
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

/// RAII cleanup for clipboard interception — same contract as [`NavInterceptGuard`].
struct ClipboardInterceptGuard(String);

impl Drop for ClipboardInterceptGuard {
    fn drop(&mut self) {
        let _ = permagent::events::clipboard_intercept::take(&self.0);
    }
}

/// Drop the per-turn voice origin so a later text turn on the same session
/// is not stuck thinking the user is still on the phone.
struct VoiceOriginGuard(String);

impl Drop for VoiceOriginGuard {
    fn drop(&mut self) {
        permagent::events::voice_origin::end(&self.0);
    }
}

fn send_json(msg: &ServerMessage) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

/// Push any captured clipboard bodies down this socket NOW. The caller
/// still shows them on `reply_text` at turn end.
async fn flush_voice_clipboard(
    socket: &mut WebSocket,
    session_id: &str,
    sent: &mut Vec<permagent::events::clipboard_intercept::ClipboardIntent>,
) {
    let clips = permagent::events::clipboard_intercept::drain(session_id);
    for clip in clips {
        let chars = clip.text.chars().count();
        if socket
            .send(send_json(&ServerMessage::Clipboard {
                text: clip.text.clone(),
            }))
            .await
            .is_ok()
        {
            tracing::info!(
                target: "permagentd::voice",
                "clipboard sent ({} characters) — copy on the listening device now",
                chars
            );
        } else {
            tracing::warn!(
                target: "permagentd::voice",
                "clipboard frame failed to send ({} characters)",
                chars
            );
        }
        sent.push(clip);
    }
}

async fn handle_voice_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    session_id: Option<String>,
    origin: permagent::events::voice_origin::VoiceOrigin,
) {
    // Snapshot the hot-swappable TTS slot once for this session.
    let tts_opt = state.voice_tts.read().await.clone();

    tracing::info!(
        target: "permagentd::voice",
        "Voice WebSocket connected (session_id={:?}, client={}, device={:?}, stt={}, tts={})",
        session_id,
        origin.client.wire_name(),
        origin.device_name,
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
    // Wake-word session (spotter + per-connection stream): present after a
    // `wake_start`, fed by monitor frames whenever no recording is active.
    let mut wake: Option<(
        Arc<crate::voice::kws::WakeWordSpotter>,
        crate::voice::kws::WakeSession,
    )> = None;

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
                            // Silent return to ready — a too-short tap is not an error.
                            let _ = socket.send(send_json(&ServerMessage::Idle)).await;
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
                            // 20260821_14: empty STT after real speech (and after
                            // auto-started noise turns) flashed "No speech detected".
                            // Return to ready with no toast.
                            let _ = socket.send(send_json(&ServerMessage::Idle)).await;
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

                        // Unspoken leftover from a spoken-budget cut. "Continue."
                        // last night started a cold agent turn and lost the story.
                        let remainder_key = session_id.as_deref().unwrap_or("voice-anon");

                        // Listen-once: we asked how a name is said. This turn
                        // is the pronunciation, not a new story beat.
                        if permagent::events::voice_pronounce::peek(remainder_key).is_some() {
                            let resume = handle_pronunciation_listen(
                                &state,
                                tts.clone(),
                                &mut socket,
                                remainder_key,
                                &transcript,
                                cancelled.clone(),
                                origin.client,
                            )
                            .await;
                            if let Some(held) = resume {
                                if socket
                                    .send(send_json(&ServerMessage::ReplyStart))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                let reply_ctx = VoiceReplyCtx {
                                    state: &state,
                                    transcript: &held,
                                    session_id: session_id.as_deref(),
                                    tts: &tts,
                                    pipeline_start,
                                    stt_ms,
                                    cancelled: cancelled.clone(),
                                    wake: wake.as_ref(),
                                    sample_rate: client_sample_rate,
                                    origin: &origin,
                                };
                                if let Err(e) = stream_reply_with_tts(&reply_ctx, &mut socket).await
                                {
                                    let _ = socket
                                        .send(send_json(&ServerMessage::Error {
                                            message: format!("Voice reply failed: {}", e),
                                        }))
                                        .await;
                                }
                            }
                            continue;
                        }

                        // User just said a name speech cannot say. Stop and ask
                        // — do not start the story and spell it.
                        if let Some(word) = first_unknown_name(tts.as_ref(), &transcript) {
                            tracing::info!(
                                target: "permagentd::voice",
                                word = %word,
                                "unknown name — asking how to say it before the reply"
                            );
                            permagent::events::voice_pronounce::begin(
                                remainder_key,
                                &word,
                                Some(transcript.clone()),
                            );
                            let shown = permagent::events::voice_pronounce::display_form(
                                &transcript,
                                &word,
                            );
                            let _ = socket
                                .send(send_json(&ServerMessage::Teach { word: shown }))
                                .await;
                            let _ = speak_canned_reply(
                                &state,
                                tts.clone(),
                                &mut socket,
                                permagent::events::voice_pronounce::ASK_FIRST,
                                cancelled.clone(),
                            )
                            .await;
                            continue;
                        }

                        if permagent::events::voice_remainder::is_continue_cue(&transcript) {
                            if let Some(rest) =
                                permagent::events::voice_remainder::take(remainder_key)
                            {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "continue cue — speaking leftover ({} chars), not a new agent turn",
                                    rest.len()
                                );
                                speak_remainder(
                                    &state,
                                    tts.clone(),
                                    &mut socket,
                                    remainder_key,
                                    &rest,
                                    cancelled.clone(),
                                    origin.client,
                                )
                                .await;
                                continue;
                            }
                        }

                        // Spoken yes/no while a decision is waiting: settle it
                        // as the user (this socket is their own hand) instead of
                        // sending the transcript to Henry, who cannot answer
                        // Tier-2 / live-channel kinds.
                        if let Some(verdict) =
                            crate::voice::spoken_verdict::spoken_decision_verdict(&transcript)
                        {
                            if let Some(decision_id) =
                                pick_spoken_decision(&state, session_id.as_deref()).await
                            {
                                match crate::routes::decisions::apply_jesse_answer(
                                    &state,
                                    &decision_id,
                                    permagent::decisions::DecisionAnswer {
                                        answer: verdict.to_string(),
                                        note: None,
                                        choice_id: None,
                                        input_text: None,
                                    },
                                    "voice",
                                )
                                .await
                                {
                                    Ok(outcome) => {
                                        let spoken = if verdict == "approve" {
                                            format!("Approved: {}.", outcome.decision.headline)
                                        } else {
                                            format!("Rejected: {}.", outcome.decision.headline)
                                        };
                                        let _ = speak_canned_reply(
                                            &state,
                                            tts.clone(),
                                            &mut socket,
                                            &spoken,
                                            cancelled.clone(),
                                        )
                                        .await;
                                        continue;
                                    }
                                    Err((status, msg)) => {
                                        tracing::warn!(
                                            target: "permagentd::voice",
                                            status = %status,
                                            "spoken decision answer failed: {msg}"
                                        );
                                    }
                                }
                            }
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
                            wake: wake.as_ref(),
                            sample_rate: client_sample_rate,
                            origin: &origin,
                        };
                        // A new ask replaces any leftover — only a continue cue
                        // (handled above) is allowed to replay it.
                        permagent::events::voice_remainder::clear(remainder_key);
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
                    Ok(ClientMessage::WakeStart { sample_rate }) => {
                        if let Some(sr) = sample_rate {
                            client_sample_rate = sr;
                        }
                        let mut spotter = state.wake_spotter.read().await.clone();
                        if spotter.is_none() {
                            // Auto-provision: the first hands-free activation
                            // kicks off the (idempotent, pinned) model fetch.
                            match start_wake_model_download(&state).await {
                                // The model may have been present already, in
                                // which case the spotter is loaded now.
                                Ok(()) => spotter = state.wake_spotter.read().await.clone(),
                                Err(e) => tracing::warn!(
                                    target: "permagentd::voice",
                                    "wake model auto-download failed to start: {e}"
                                ),
                            }
                        }
                        let response = match spotter {
                            Some(sp) => {
                                // The wake phrase follows the persona name.
                                let name = {
                                    let p = state.persona.read().await;
                                    p.nickname
                                        .clone()
                                        .filter(|n| !n.trim().is_empty())
                                        .unwrap_or_else(|| p.first_name.clone())
                                };
                                let phrases = vec![format!("hey {name}"), format!("okay {name}")];
                                match sp.create_session(&phrases) {
                                    Ok(sess) => {
                                        tracing::info!(
                                            target: "permagentd::voice",
                                            "wake listening started (phrase: \"hey {name}\")"
                                        );
                                        wake = Some((sp, sess));
                                        ServerMessage::WakeStatus {
                                            active: true,
                                            phrase: Some(format!("Hey {name}")),
                                            reason: None,
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            target: "permagentd::voice",
                                            "wake session failed: {e}"
                                        );
                                        ServerMessage::WakeStatus {
                                            active: false,
                                            phrase: None,
                                            reason: Some("wake phrase not encodable".into()),
                                        }
                                    }
                                }
                            }
                            None => ServerMessage::WakeStatus {
                                active: false,
                                phrase: None,
                                reason: Some("downloading".into()),
                            },
                        };
                        if socket.send(send_json(&response)).await.is_err() {
                            break;
                        }
                    }
                    Ok(ClientMessage::WakeStop) => {
                        wake = None;
                    }
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
            Message::Binary(data) => {
                // Monitor audio while idle: feed the keyword spotter. The int8
                // 3.3M zipformer decodes a ≤128ms frame in low single-digit
                // milliseconds — not worth a spawn_blocking round-trip.
                if let Some((sp, sess)) = &wake {
                    let chunk: Vec<f32> = data
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    let detection = sp.accept(sess, client_sample_rate, &chunk);
                    let kind = match detection {
                        Some(crate::voice::kws::Detection::Wake) => "wake",
                        Some(crate::voice::kws::Detection::Stop) => "stop",
                        None => continue,
                    };
                    tracing::info!(target: "permagentd::voice", "keyword detected: {kind}");
                    if socket
                        .send(send_json(&ServerMessage::Wake { kind: kind.into() }))
                        .await
                        .is_err()
                    {
                        break;
                    }
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

/// Default cap on how many sentences are SPOKEN in one turn.
///
/// The full reply always reaches the client as `ReplyText`. Eight sentences
/// is roughly 40 seconds of Kokoro speech — long for a spoken answer, and
/// well past the "1-3 sentences" the system prompt asks for. The cue when
/// this cap hits is origin-aware ([`permagent::events::voice_origin::budget_notice`])
/// so a phone listener is not told the rest is on a Mac screen.
const DEFAULT_MAX_SPOKEN_SENTENCES: u32 = 8;

/// Config key (`~/.permagent/config.yaml`) overriding the spoken-length budget.
/// Clamped to [1, 100]; 0 would mute replies entirely, which is never intended.
const MAX_SPOKEN_SENTENCES_KEY: &str = "voice_max_spoken_sentences";

fn max_spoken_sentences() -> u32 {
    permagent::config::Config::global()
        .get_param::<u32>(MAX_SPOKEN_SENTENCES_KEY)
        .unwrap_or(DEFAULT_MAX_SPOKEN_SENTENCES)
        .clamp(1, 100)
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
    /// Wake session for mid-reply spoken-stop detection (monitor frames keep
    /// arriving while the reply streams; they queue on the socket and are
    /// drained between events).
    wake: Option<&'a (
        Arc<crate::voice::kws::WakeWordSpotter>,
        crate::voice::kws::WakeSession,
    )>,
    sample_rate: u32,
    origin: &'a permagent::events::voice_origin::VoiceOrigin,
}

/// Outcome of a non-blocking drain of queued client messages mid-reply.
enum DrainOutcome {
    /// Nothing actionable — keep streaming.
    Continue,
    /// A spoken stop phrase landed — end the turn now.
    SpokenStop,
    /// The socket is gone — abandon the turn (cancelled flag already set).
    Disconnected,
}

/// Drain any client messages already queued on the socket WITHOUT blocking,
/// feeding monitor audio to the keyword spotter. This is what makes a spoken
/// "stop" land while the reply is still being generated: the handler is deep
/// in `stream_reply_with_tts` and its recv loop isn't running, so queued
/// frames would otherwise sit unread until the turn finished.
fn drain_client_messages(socket: &mut WebSocket, ctx: &VoiceReplyCtx<'_>) -> DrainOutcome {
    use futures::FutureExt;
    loop {
        match socket.recv().now_or_never() {
            // Nothing queued right now.
            None => return DrainOutcome::Continue,
            Some(None) | Some(Some(Err(_))) | Some(Some(Ok(Message::Close(_)))) => {
                ctx.cancelled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                return DrainOutcome::Disconnected;
            }
            Some(Some(Ok(Message::Binary(data)))) => {
                if let Some((sp, sess)) = ctx.wake {
                    let chunk: Vec<f32> = data
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    // A mid-reply "wake" has nothing to open; only stop acts.
                    if sp.accept(sess, ctx.sample_rate, &chunk)
                        == Some(crate::voice::kws::Detection::Stop)
                    {
                        return DrainOutcome::SpokenStop;
                    }
                }
            }
            // Text (a stray stop/start) and pings are ignored mid-turn, as
            // they always have been while the handler streams a reply.
            Some(Some(Ok(_))) => {}
        }
    }
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
    permagent::events::clipboard_intercept::begin(&sid);
    let _clip_guard = ClipboardInterceptGuard(sid.clone());
    permagent::events::voice_origin::begin(&sid, ctx.origin.clone());
    let _origin_guard = VoiceOriginGuard(sid.clone());

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
             questions. For a story or a longer ask, keep going — do not stop at three \
             sentences to ask if they want more. Never say 'do you want me to continue', \
             'shall I go on', or any mid-reply continue offer; if there is more, say it. \
             Speak as you would in a real conversation — with feeling, not a flat \
             reading: let a reaction through, vary the length of sentences, take a breath. \
             USE CONTRACTIONS the way people do: I'm, don't, should've, would've, it's, I'll, haven't. \
             Never expand those into 'I am' / 'should have' — the voice can say them. \
             Finish the sentence you are on. Do not trail off mid-clause. \
             Never write '=' or letter-spell a name (no 'EL-speth', no 'E-L-S-P-E-T-H'). Say the name. \
             The voice takes its rhythm from punctuation, not from stage directions. \
             Write the way people talk: a comma for a breath, an em dash for a turn, \
             an ellipsis (...) when you are thinking, a question mark when you actually \
             want an answer, an exclamation only when you mean the energy. Prefer two \
             short sentences over one long one — long lines flatten. \
             You may prefix a sentence with ONE delivery tag: [warm] [excited] [calm] \
             [gentle] [serious] [thoughtful] [playful]. Insert [pause] for a beat. Never \
             say the tag names or the brackets aloud; most sentences need no tag. \
             NEVER spell a word letter by letter. If you do not already know how a name \
             will sound, STOP. Say you will place it on the Orb and listen. Do not say \
             the word. Then call save_pronunciation with the word and what they said — \
             one time, then it is saved forever. Do not guess a respelling and keep going. \
             Verbalize outcomes, not interface mechanics: don't read out UI labels, button names, \
             menu paths, file paths, URLs, or settings keys, and don't narrate the individual \
             steps you take to do something. Say what happened or what the user should do in plain \
             spoken terms — e.g. 'I turned on web search' rather than 'I clicked the Search and \
             tools toggle in Settings'. If a literal name, path, or value is essential, give just \
             that one item, not the surrounding navigation. \
             When they ask for copyable text — a post, caption, speech, blurb, 'give me the \
             text', 'copy that', 'so I can paste into Notes' — call copy_to_clipboard with \
             the exact paste-ready body and speak one short confirmation such as 'It's on \
             your clipboard.' Do not read the body aloud and do not skip the tool: saying \
             the words is not the same as putting them on the clipboard."
                .to_string(),
        )
        .await;
    agent
        .extend_system_prompt("voice_origin".to_string(), ctx.origin.prompt_block())
        .await;

    let setup_ms = t_setup.elapsed().as_millis();

    // Ambient, recall, and a G2P scan of the user's words are independent —
    // run them together so pronunciation coaching does not add a serial wait.
    let t_ctx = std::time::Instant::now();
    let tts_for_scan = tts.clone();
    let transcript_for_scan = transcript.to_string();
    let oov_fut =
        tokio::task::spawn_blocking(move || tts_for_scan.unresolved_words(&transcript_for_scan));
    let ambient_fut = crate::brain_ops::inject_ambient_context(state, &agent);
    let (recall_trace, transcript_oov) = if let Some(ref brain) = state.brain {
        let recognition_ctx = state.build_recognition_context(Some(&sid));
        let recognition_pool = state.session_manager().pool_clone().await.ok();
        let recall_fut = crate::brain_ops::inject_recall(
            brain,
            &agent,
            transcript,
            recognition_ctx,
            recognition_pool,
        );
        let (_, trace, oov) = tokio::join!(ambient_fut, recall_fut, oov_fut);
        (trace, oov.unwrap_or_default())
    } else {
        let (_, oov) = tokio::join!(ambient_fut, oov_fut);
        (
            crate::brain_ops::RecallInjection::default(),
            oov.unwrap_or_default(),
        )
    };
    let ctx_recall_ms = t_ctx.elapsed().as_millis();
    tracing::info!(target: "permagentd::voice", "  ctx+recall: {}ms ({} recall hits)", ctx_recall_ms, recall_trace.count);

    if let Some(coaching) = pronunciation_coaching(&transcript_oov) {
        agent
            .extend_system_prompt("voice_pronunciation".to_string(), coaching)
            .await;
    }

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

    // Accumulate text, detect phrase/sentence boundaries. TTS of sentence N
    // runs while the LLM is still emitting sentence N+1 — awaiting synthesis
    // inside the token loop was holding first-audio on every subsequent chunk.
    let mut text_buf = String::new();
    let mut full_reply = String::new();
    let mut sentence_num = 0u32;
    let max_spoken = max_spoken_sentences();
    let spoken_cue = permagent::events::voice_origin::budget_notice(ctx.origin.client);
    let mut budget_notice_spoken = false;
    let mut leftover = String::new();
    let mut pronounce_hold = false;
    let mut total_tts_ms: u128 = 0;
    let mut first_audio_sent = false;
    let mut first_token_logged = false;
    let mut spoken_stop = false;
    let stream_start = std::time::Instant::now();
    let mut queue: std::collections::VecDeque<(String, f32)> = std::collections::VecDeque::new();
    let mut inflight: Option<(
        tokio::task::JoinHandle<anyhow::Result<AudioOutput>>,
        std::time::Instant,
        String,
    )> = None;
    let mut stream_ended = false;
    let mut sent_clips: Vec<permagent::events::clipboard_intercept::ClipboardIntent> = Vec::new();
    let mut clip_tick = tokio::time::interval(std::time::Duration::from_millis(50));
    clip_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Interval fires immediately on first poll — consume that so the select
    // arm cannot spin the loop before the 50ms cadence starts.
    clip_tick.tick().await;

    loop {
        // Copy as soon as the tool captures — do not wait for TTS. The user
        // often leaves for Notes during "let me write you something."
        flush_voice_clipboard(socket, &sid, &mut sent_clips).await;
        match drain_client_messages(socket, ctx) {
            DrainOutcome::Continue => {}
            DrainOutcome::SpokenStop => {
                spoken_stop = true;
                break;
            }
            DrainOutcome::Disconnected => return Ok(()),
        }
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        if inflight.is_none() {
            if let Some((speech, speed)) = queue.pop_front() {
                // Never spell an unknown name. Stop this reply, ask, listen.
                if let Some(word) = first_unknown_name(tts.as_ref(), &speech) {
                    tracing::info!(
                        target: "permagentd::voice",
                        word = %word,
                        "unknown name in reply — stopping to ask"
                    );
                    permagent::events::voice_remainder::append_sentence(&mut leftover, &speech);
                    while let Some((rest, _)) = queue.pop_front() {
                        permagent::events::voice_remainder::append_sentence(&mut leftover, &rest);
                    }
                    permagent::events::voice_pronounce::begin(&sid, &word, None);
                    pronounce_hold = true;
                    let shown = permagent::events::voice_pronounce::display_form(&speech, &word);
                    let _ = socket
                        .send(send_json(&ServerMessage::Teach { word: shown }))
                        .await;
                    let ask = permagent::events::voice_pronounce::ASK_FIRST.to_string();
                    inflight = Some((
                        spawn_synth(
                            tts.clone(),
                            ask.clone(),
                            voice_id.clone(),
                            1.0,
                            cancelled.clone(),
                        ),
                        std::time::Instant::now(),
                        ask,
                    ));
                } else {
                    let preview = speech.clone();
                    inflight = Some((
                        spawn_synth(
                            tts.clone(),
                            speech,
                            voice_id.clone(),
                            speed,
                            cancelled.clone(),
                        ),
                        std::time::Instant::now(),
                        preview,
                    ));
                }
            } else if stream_ended {
                break;
            }
        }

        let first_chunk = !first_audio_sent && queue.is_empty() && inflight.is_none();

        tokio::select! {
            biased;

            result = std::future::poll_fn(|cx| {
                use std::future::Future;
                let handle = &mut inflight.as_mut().expect("guarded by if").0;
                std::pin::Pin::new(handle).poll(cx)
            }), if inflight.is_some() => {
                let (_, started, preview) = inflight.take().expect("guarded");
                let chunk_tts_ms = started.elapsed().as_millis();
                total_tts_ms += chunk_tts_ms;
                match drain_client_messages(socket, ctx) {
                    DrainOutcome::Continue => {}
                    DrainOutcome::SpokenStop => {
                        spoken_stop = true;
                        break;
                    }
                    DrainOutcome::Disconnected => return Ok(()),
                }
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(());
                }
                match result {
                    Ok(Ok(audio)) => {
                        let dur = audio.samples.len() as f32 / audio.sample_rate as f32;
                        let rtf = chunk_tts_ms as f32 / 1000.0 / dur.max(0.01);
                        tracing::info!(
                            target: "permagentd::voice",
                            "STREAM sentence {}: {}chars TTS={}ms audio={:.1}s RTF={:.2}x | \"{}\"",
                            sentence_num, preview.len(), chunk_tts_ms, dur, rtf,
                            truncate_str(&preview, 60)
                        );
                        if !first_audio_sent {
                            tracing::info!(
                                target: "permagentd::voice",
                                "TIMING first audio: {}ms after speech-end",
                                pipeline_start.elapsed().as_millis()
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
                    Ok(Err(e)) if e.to_string() == "cancelled" => {
                        tracing::info!(target: "permagentd::voice", "TTS cancelled (pre-mutex)");
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(target: "permagentd::voice", "TTS chunk failed: {}", e);
                    }
                    Err(e) => {
                        tracing::warn!(target: "permagentd::voice", "TTS task panicked: {}", e);
                    }
                }
            }

            event = stream.next(), if !stream_ended => {
                match event {
                    None => {
                        stream_ended = true;
                        if pronounce_hold {
                            permagent::events::voice_remainder::append_sentence(
                                &mut leftover,
                                &text_buf,
                            );
                            text_buf.clear();
                        } else {
                            enqueue_remainder(
                                &mut text_buf,
                                &mut queue,
                                &mut sentence_num,
                                max_spoken,
                                &mut budget_notice_spoken,
                                spoken_cue,
                                &mut leftover,
                            );
                        }
                    }
                    Some(Ok(AgentEvent::Message(msg)))
                        if msg.role == rmcp::model::Role::Assistant =>
                    {
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
                                if pronounce_hold {
                                    permagent::events::voice_remainder::append_sentence(
                                        &mut leftover,
                                        &text_buf,
                                    );
                                    text_buf.clear();
                                } else {
                                    enqueue_ready_sentences(
                                        &mut text_buf,
                                        &mut queue,
                                        &mut sentence_num,
                                        max_spoken,
                                        &mut budget_notice_spoken,
                                        spoken_cue,
                                        first_chunk,
                                        &mut leftover,
                                    );
                                }
                            }
                        }
                    }
                    Some(_) => {
                        // Tool results land here. Drain on the next loop turn
                        // (and via clip_tick) so we don't wait for confirmation TTS.
                    }
                }
            }

            _ = clip_tick.tick() => {}
        }

        if spoken_stop {
            break;
        }
    }

    if spoken_stop {
        tracing::info!(
            target: "permagentd::voice",
            "spoken stop ended the turn after {} sentences",
            sentence_num
        );
        permagent::events::voice_remainder::clear(&sid);
        let _ = socket.send(send_json(&ServerMessage::Stopped)).await;
    } else if leftover.trim().is_empty() {
        permagent::events::voice_remainder::clear(&sid);
    } else {
        tracing::info!(
            target: "permagentd::voice",
            "stashing unspoken leftover ({} chars) for continue",
            leftover.len()
        );
        permagent::events::voice_remainder::stash(&sid, leftover);
    }

    // Any capture that raced the last loop turn. Then unregister so the
    // guard's Drop is a no-op rather than swallowing a late write.
    flush_voice_clipboard(socket, &sid, &mut sent_clips).await;
    for clip in permagent::events::clipboard_intercept::take(&sid) {
        let _ = socket
            .send(send_json(&ServerMessage::Clipboard {
                text: clip.text.clone(),
            }))
            .await;
        sent_clips.push(clip);
    }

    // Show the paste-ready body on screen even when the spoken reply was
    // only a confirmation — iOS VoiceView (and desktop lastReply) render this.
    let shown = if let Some(clip) = sent_clips.last() {
        if full_reply.contains(&clip.text) {
            full_reply.clone()
        } else {
            format!("{}\n\n{}", full_reply.trim(), clip.text)
        }
    } else {
        full_reply.clone()
    };
    let _ = socket
        .send(send_json(&ServerMessage::ReplyText { text: shown }))
        .await;

    // Forward any navigations captured during this turn AFTER all narration
    // audio — they ride this ordered socket, so by the time the client sees them
    // every audio chunk is already queued. The client fires them only once the
    // audio queue drains, so the view switches when the agent stops speaking.
    // A spoken stop drops them: the user ended the turn, so yanking the view
    // somewhere afterwards is exactly what they asked not to happen. (The
    // guard's Drop clears the interceptor registry either way.)
    let navs = if spoken_stop || !ctx.origin.client.can_drive_desktop_ui() {
        // Phone/watch: drop captured navs so Command Center on the Mac does
        // not switch behind the user's back. The tool also no-ops (N4).
        let _ = permagent::events::nav_intercept::take(&sid);
        Vec::new()
    } else {
        permagent::events::nav_intercept::take(&sid)
    };
    for nav in navs {
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

    recall_trace.finish(full_reply.clone());

    // Persist voice turn to Brain — same as text chat, so future recall
    // can surface what was discussed via voice.
    if let Some(ref brain) = state.brain {
        if !transcript.is_empty() && !full_reply.is_empty() {
            // Use a simple turn index based on timestamp (voice doesn't track conversation length)
            let turn_idx = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as usize)
                .unwrap_or(0);
            // Voice has no tool-call transcript in hand at this seam, so the
            // corroboration check sees prose only. That is honest: a voice
            // turn is winged when it names its project and left `general`
            // otherwise, exactly like a typed turn that names nothing.
            let pool = state.session_manager().pool_clone().await.ok();
            crate::brain_ops::spawn_persist_chat_turn(
                brain.clone(),
                pool,
                sid.to_string(),
                turn_idx,
                transcript.to_string(),
                full_reply,
                String::new(),
            );
        }
    }

    Ok(())
}

/// Strip delivery tags, drop unspeakable junk, pick Kokoro speed.
fn prepare_spoken(sentence: &str) -> Option<(String, f32)> {
    let plan = crate::voice::prosody::plan(sentence);
    crate::voice::speakable::speakable(&plan.speech).map(|speech| (speech, plan.speed))
}

fn pronunciation_coaching(transcript_oov: &[String]) -> Option<String> {
    if !transcript_oov.is_empty() {
        crate::voice::oov_log::record(transcript_oov);
    }
    crate::voice::oov_log::coaching_prompt()
}

async fn pick_spoken_decision(state: &Arc<AppState>, session_id: Option<&str>) -> Option<String> {
    let pool = state.session_manager().pool_clone().await.ok()?;
    let items = permagent::decisions::list_open_decisions(&pool)
        .await
        .ok()?;
    let binary: Vec<_> = items
        .into_iter()
        .filter(|i| crate::voice::spoken_verdict::is_binary_kind(&i.decision.kind))
        .collect();
    if let Some(sid) = session_id {
        if let Some(hit) = binary.iter().find(|i| {
            i.decision.kind == "tool_approval"
                && i.decision
                    .payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    == Some(sid)
        }) {
            return Some(hit.decision.id.clone());
        }
    }
    if binary.len() == 1 {
        return Some(binary[0].decision.id.clone());
    }
    None
}

fn first_unknown_name(tts: &dyn crate::voice::TextToSpeech, text: &str) -> Option<String> {
    let oov = tts.unresolved_words(text);
    let word = permagent::events::voice_pronounce::first_teachable(&oov)?;
    if crate::voice::user_lexicon::known(&word) {
        return None;
    }
    Some(word)
}

fn try_save_heard(
    tts: &dyn crate::voice::TextToSpeech,
    word: &str,
    transcript: &str,
) -> Option<String> {
    for sounds_like in permagent::events::voice_pronounce::save_candidates(word, transcript) {
        let Ok(ipa) = tts.phonemize_text(&sounds_like) else {
            continue;
        };
        if crate::voice::user_lexicon::save(
            word,
            crate::voice::user_lexicon::PronunciationEntry {
                ipa,
                sounds_like: sounds_like.clone(),
            },
        )
        .is_err()
        {
            continue;
        }
        crate::voice::oov_log::forget(word);
        tracing::info!(
            target: "permagentd::voice",
            word = %word,
            %sounds_like,
            "pronunciation saved from listen"
        );
        return Some(sounds_like);
    }
    None
}

/// Consume a listen turn. Returns a held user request to resume, if any.
async fn handle_pronunciation_listen(
    state: &Arc<AppState>,
    tts: Arc<dyn crate::voice::TextToSpeech>,
    socket: &mut WebSocket,
    session_id: &str,
    transcript: &str,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    client: permagent::events::voice_origin::VoiceClient,
) -> Option<String> {
    let pending = permagent::events::voice_pronounce::take(session_id)?;

    if permagent::events::voice_pronounce::is_skip_cue(transcript) {
        let _ = socket
            .send(send_json(&ServerMessage::Taught {
                word: permagent::events::voice_pronounce::display_word(&pending.word),
            }))
            .await;
        let _ = speak_canned_reply(
            state,
            tts.clone(),
            socket,
            permagent::events::voice_pronounce::SKIPPED,
            cancelled.clone(),
        )
        .await;
        if let Some(rest) = permagent::events::voice_remainder::take(session_id) {
            speak_remainder(state, tts, socket, session_id, &rest, cancelled, client).await;
            return None;
        }
        return pending.held_transcript;
    }

    if try_save_heard(tts.as_ref(), &pending.word, transcript).is_some() {
        let shown = permagent::events::voice_pronounce::display_word(&pending.word);
        let _ = socket
            .send(send_json(&ServerMessage::Taught { word: shown }))
            .await;
        let said = permagent::events::voice_pronounce::saved_confirmation(&pending.word);
        let _ = speak_canned_reply(state, tts.clone(), socket, &said, cancelled.clone()).await;
        if let Some(rest) = permagent::events::voice_remainder::take(session_id) {
            speak_remainder(state, tts, socket, session_id, &rest, cancelled, client).await;
            return None;
        }
        if let Some(held) = pending.held_transcript {
            if let Some(next) = first_unknown_name(tts.as_ref(), &held) {
                permagent::events::voice_pronounce::begin(session_id, &next, Some(held.clone()));
                let shown = permagent::events::voice_pronounce::display_form(&held, &next);
                let _ = socket
                    .send(send_json(&ServerMessage::Teach { word: shown }))
                    .await;
                let _ = speak_canned_reply(
                    state,
                    tts,
                    socket,
                    permagent::events::voice_pronounce::ASK_FIRST,
                    cancelled,
                )
                .await;
                return None;
            }
            return Some(held);
        }
        return None;
    }

    let shown = permagent::events::voice_pronounce::display_word(&pending.word);
    permagent::events::voice_pronounce::begin(session_id, &pending.word, pending.held_transcript);
    let _ = socket
        .send(send_json(&ServerMessage::Teach { word: shown }))
        .await;
    let _ = speak_canned_reply(
        state,
        tts,
        socket,
        permagent::events::voice_pronounce::ASK_AGAIN,
        cancelled,
    )
    .await;
    None
}

async fn speak_canned_reply(
    state: &Arc<AppState>,
    tts: Arc<dyn crate::voice::TextToSpeech>,
    socket: &mut WebSocket,
    text: &str,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if socket
        .send(send_json(&ServerMessage::ReplyStart))
        .await
        .is_err()
    {
        return;
    }
    let voice_id = state.persona.read().await.voice_id.clone();
    let plan = crate::voice::prosody::plan(text);
    let speech =
        crate::voice::speakable::speakable(&plan.speech).unwrap_or_else(|| text.to_string());
    match spawn_synth(tts.clone(), speech, voice_id, plan.speed, cancelled).await {
        Ok(Ok(audio)) => {
            let bytes: Vec<u8> = audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            let _ = socket.send(Message::Binary(bytes.into())).await;
        }
        Ok(Err(e)) => tracing::warn!(target: "permagentd::voice", "canned TTS failed: {e}"),
        Err(e) => tracing::warn!(target: "permagentd::voice", "canned TTS panicked: {e}"),
    }
    let _ = socket
        .send(send_json(&ServerMessage::ReplyText {
            text: text.to_string(),
        }))
        .await;
    let _ = socket
        .send(send_json(&ServerMessage::ReplyEnd {
            sample_rate: tts.sample_rate(),
        }))
        .await;
}

fn spawn_synth(
    tts: Arc<dyn crate::voice::TextToSpeech>,
    text: String,
    voice_id: Option<String>,
    speed: f32,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<anyhow::Result<AudioOutput>> {
    tokio::task::spawn_blocking(move || {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(anyhow::anyhow!("cancelled"));
        }
        let mut audio = tts.synthesize(
            &text,
            &TtsConfig {
                voice_id,
                speed,
                lexicon: crate::voice::user_lexicon::current(),
            },
        )?;
        crate::voice::loudness::master(&mut audio.samples, audio.sample_rate, &text);
        Ok(audio)
    })
}

fn push_spoken(
    sentence: &str,
    queue: &mut std::collections::VecDeque<(String, f32)>,
    sentence_num: &mut u32,
    max_spoken: u32,
    budget_notice_spoken: &mut bool,
    spoken_cue: &'static str,
    leftover: &mut String,
) {
    let Some((speech, speed)) = prepare_spoken(sentence) else {
        tracing::debug!(
            target: "permagentd::voice",
            "skipping unspeakable fragment: \"{}\"",
            truncate_str(sentence, 60)
        );
        return;
    };
    if *sentence_num >= max_spoken {
        if !*budget_notice_spoken {
            *budget_notice_spoken = true;
            tracing::info!(
                target: "permagentd::voice",
                "spoken budget reached ({} sentences) — remaining reply is leftover for continue",
                max_spoken
            );
            queue.push_back((spoken_cue.to_string(), 1.0));
        }
        permagent::events::voice_remainder::append_sentence(leftover, &speech);
        return;
    }
    // Clause chunks (comma, dash) stream for first-audio, but they are not
    // sentences. Counting them as the budget cut him mid-thought last night.
    // Only a .!? close consumes a spoken-sentence slot — he finishes the
    // sentence unless the user barges in.
    if closes_a_sentence(&speech) {
        *sentence_num += 1;
    }
    queue.push_back((speech, speed));
}

fn closes_a_sentence(text: &str) -> bool {
    matches!(
        text.trim().chars().rev().find(|c| !c.is_whitespace()),
        Some('.' | '!' | '?')
    )
}

// Eight parameters, all of them distinct state this streaming step mutates in
// place: the buffer, the outbound queue, the sentence counter, the spoken
// budget and its one-shot notice, the cue text, whether this is the first
// chunk, and the leftover tail. Bundling them into a struct would only move
// the same eight fields behind one name. Same call the rest of this crate makes
// (see verification/mod.rs, routes/analytics_verify.rs).
#[allow(clippy::too_many_arguments)]
fn enqueue_ready_sentences(
    text_buf: &mut String,
    queue: &mut std::collections::VecDeque<(String, f32)>,
    sentence_num: &mut u32,
    max_spoken: u32,
    budget_notice_spoken: &mut bool,
    spoken_cue: &'static str,
    first_chunk: bool,
    leftover: &mut String,
) {
    let mut aggressive = first_chunk;
    while let Some((end_inclusive, next_start)) = find_speakable_boundary(text_buf, aggressive) {
        let sentence = text_buf
            .get(..=end_inclusive)
            .unwrap_or("")
            .trim()
            .to_string();
        *text_buf = text_buf.get(next_start..).unwrap_or("").to_string();
        if sentence.is_empty() {
            continue;
        }
        push_spoken(
            &sentence,
            queue,
            sentence_num,
            max_spoken,
            budget_notice_spoken,
            spoken_cue,
            leftover,
        );
        aggressive = false;
    }
}

fn enqueue_remainder(
    text_buf: &mut String,
    queue: &mut std::collections::VecDeque<(String, f32)>,
    sentence_num: &mut u32,
    max_spoken: u32,
    budget_notice_spoken: &mut bool,
    spoken_cue: &'static str,
    leftover: &mut String,
) {
    let remainder = text_buf.trim().to_string();
    text_buf.clear();
    if remainder.is_empty() {
        return;
    }
    push_spoken(
        &remainder,
        queue,
        sentence_num,
        max_spoken,
        budget_notice_spoken,
        spoken_cue,
        leftover,
    );
}

/// Replay leftover prose on a continue cue — no new agent turn.
async fn speak_remainder(
    state: &Arc<AppState>,
    tts: Arc<dyn crate::voice::TextToSpeech>,
    socket: &mut WebSocket,
    session_id: &str,
    leftover: &str,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    client: permagent::events::voice_origin::VoiceClient,
) {
    if socket
        .send(send_json(&ServerMessage::ReplyStart))
        .await
        .is_err()
    {
        permagent::events::voice_remainder::stash(session_id, leftover.to_string());
        return;
    }

    let voice_id = state.persona.read().await.voice_id.clone();
    let spoken_cue = permagent::events::voice_origin::budget_notice(client);
    let max_spoken = max_spoken_sentences();
    let mut queue: std::collections::VecDeque<(String, f32)> = std::collections::VecDeque::new();
    let mut sentence_num = 0u32;
    let mut budget_notice_spoken = false;
    let mut still_left = String::new();
    let mut text_buf = leftover.to_string();
    enqueue_ready_sentences(
        &mut text_buf,
        &mut queue,
        &mut sentence_num,
        max_spoken,
        &mut budget_notice_spoken,
        spoken_cue,
        true,
        &mut still_left,
    );
    enqueue_remainder(
        &mut text_buf,
        &mut queue,
        &mut sentence_num,
        max_spoken,
        &mut budget_notice_spoken,
        spoken_cue,
        &mut still_left,
    );

    let mut spoken_text = String::new();
    while let Some((speech, speed)) = queue.pop_front() {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            permagent::events::voice_remainder::stash(session_id, leftover.to_string());
            return;
        }
        if speech != spoken_cue {
            if !spoken_text.is_empty() {
                spoken_text.push(' ');
            }
            spoken_text.push_str(&speech);
        }
        match spawn_synth(
            tts.clone(),
            speech,
            voice_id.clone(),
            speed,
            cancelled.clone(),
        )
        .await
        {
            Ok(Ok(audio)) => {
                let bytes: Vec<u8> = audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                if socket.send(Message::Binary(bytes.into())).await.is_err() {
                    permagent::events::voice_remainder::stash(session_id, leftover.to_string());
                    return;
                }
            }
            Ok(Err(e)) => tracing::warn!(target: "permagentd::voice", "remainder TTS failed: {e}"),
            Err(e) => tracing::warn!(target: "permagentd::voice", "remainder TTS panicked: {e}"),
        }
    }

    if still_left.trim().is_empty() {
        permagent::events::voice_remainder::clear(session_id);
    } else {
        permagent::events::voice_remainder::stash(session_id, still_left);
    }

    let _ = socket
        .send(send_json(&ServerMessage::ReplyText { text: spoken_text }))
        .await;
    let _ = socket
        .send(send_json(&ServerMessage::ReplyEnd {
            sample_rate: tts.sample_rate(),
        }))
        .await;
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
/// `first_chunk` lowers the floors so the first audio can leave before a
/// full 25-character clause has landed.
fn find_speakable_boundary(text: &str, first_chunk: bool) -> Option<(usize, usize)> {
    let sentence_min = if first_chunk { 3 } else { 5 };
    let clause_min = if first_chunk { 12 } else { 25 };

    // First pass: sentence boundary (strongest break, lowest minimum)
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if (ch == '.' || ch == '!' || ch == '?') && i >= sentence_min {
            let after = iter.peek().map(|(_, c)| *c);
            if after.is_none() || after == Some(' ') || after == Some('\n') {
                return Some((i, i + ch.len_utf8()));
            }
        }
    }

    // Second pass: clause boundary (weaker break, higher minimum)
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if i < clause_min {
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

    #[test]
    fn first_chunk_speaks_a_short_yes_immediately() {
        let hit = find_speakable_boundary("Yes.", true);
        assert!(
            hit.is_some(),
            "first audio should fire on a 4-char sentence"
        );
        let (end, _) = hit.unwrap();
        assert_eq!("Yes.".get(..=end), Some("Yes."));
    }

    #[test]
    fn later_chunks_still_wait_for_a_real_clause() {
        assert!(
            find_speakable_boundary("Hello there, ", false).is_none(),
            "a 13-char clause must not chop later sentences"
        );
        assert!(find_speakable_boundary("Hello there, friend of mine, ", false).is_some());
    }

    #[test]
    fn prepare_spoken_strips_delivery_tags() {
        let (speech, speed) = prepare_spoken("[excited] We shipped it!").unwrap();
        assert_eq!(speech, "We shipped it!");
        assert_eq!(speed, 1.12);
    }

    #[test]
    fn prepare_spoken_questions_run_slower_not_faster() {
        let (_, speed) = prepare_spoken("Ready?").unwrap();
        assert_eq!(speed, 0.95);
    }

    #[test]
    fn teach_frame_places_the_word_and_never_spells() {
        let teach = serde_json::to_value(ServerMessage::Teach {
            word: "Elspeth".into(),
        })
        .unwrap();
        assert_eq!(teach["type"], "teach");
        assert_eq!(teach["word"], "Elspeth");
        let taught = serde_json::to_value(ServerMessage::Taught {
            word: "Elspeth".into(),
        })
        .unwrap();
        assert_eq!(taught["type"], "taught");
    }

    /// 20260821_14: empty STT must serialize as `idle`, never the toast string.
    #[test]
    fn empty_turn_is_idle_not_an_error_toast() {
        let idle = serde_json::to_value(ServerMessage::Idle).unwrap();
        assert_eq!(idle["type"], "idle");
        assert!(idle.get("message").is_none());
        let err = serde_json::to_value(ServerMessage::Error {
            message: "No speech detected — try again".into(),
        })
        .unwrap();
        assert_ne!(
            idle["type"], err["type"],
            "empty STT must not reuse the error frame"
        );
    }

    /// Last night: eight spoken sentences, leftover dropped, Continue lost.
    #[test]
    fn spoken_budget_stashes_unspoken_sentences() {
        let mut queue = std::collections::VecDeque::new();
        let mut n = 0u32;
        let mut cue = false;
        let mut leftover = String::new();
        for i in 1..=12 {
            push_spoken(
                &format!("Sentence number {i} of the Rowan story."),
                &mut queue,
                &mut n,
                8,
                &mut cue,
                "There's more when you want it.",
                &mut leftover,
            );
        }
        assert_eq!(n, 8);
        assert!(cue);
        assert!(
            leftover.contains("Sentence number 9"),
            "leftover lost sentence 9: {leftover}"
        );
        assert!(leftover.contains("Sentence number 12"));
        assert!(
            !leftover.contains("Sentence number 8"),
            "spoken sentence leaked into leftover: {leftover}"
        );
        let spoken: Vec<_> = queue.iter().map(|(s, _)| s.as_str()).collect();
        assert!(spoken.iter().any(|s| s.contains("There's more")));
        assert!(!spoken.iter().any(|s| s.contains("Sentence number 9")));
    }

    #[test]
    fn clauses_do_not_consume_the_spoken_budget() {
        let mut queue = std::collections::VecDeque::new();
        let mut n = 0u32;
        let mut cue = false;
        let mut leftover = String::new();
        push_spoken(
            "Once upon a time, ",
            &mut queue,
            &mut n,
            1,
            &mut cue,
            "There's more when you want it.",
            &mut leftover,
        );
        push_spoken(
            "there was a girl called Elspeth.",
            &mut queue,
            &mut n,
            1,
            &mut cue,
            "There's more when you want it.",
            &mut leftover,
        );
        assert_eq!(n, 1, "only the period should consume the budget");
        assert!(!cue, "finishing one sentence must not cut him mid-thought");
        assert!(queue.iter().any(|(s, _)| s.contains("Once upon a time")));
        assert!(queue.iter().any(|(s, _)| s.contains("Elspeth")));
    }

    #[test]
    fn closes_a_sentence_is_period_question_or_bang() {
        assert!(closes_a_sentence("Hello."));
        assert!(closes_a_sentence("Ready?"));
        assert!(closes_a_sentence("Go!"));
        assert!(!closes_a_sentence("Once upon a time,"));
        assert!(!closes_a_sentence("a long clause —"));
    }

    #[test]
    fn continue_cue_takes_the_stashed_leftover() {
        let sid = "voice-budget-continue-1";
        permagent::events::voice_remainder::clear(sid);
        permagent::events::voice_remainder::stash(
            sid,
            "He pulled the cloak tighter and kept walking toward the ridge.".into(),
        );
        assert!(permagent::events::voice_remainder::is_continue_cue(
            "Continue."
        ));
        let rest = permagent::events::voice_remainder::take(sid).expect("leftover");
        assert!(rest.contains("cloak"));
        assert!(
            permagent::events::voice_remainder::take(sid).is_none(),
            "a second Continue must not invent a new leftover"
        );
        permagent::events::voice_remainder::clear(sid);
    }

    #[test]
    fn last_night_stt_fragments_are_not_a_teach_ask() {
        // 23:44: "Princess L Spith" produced spith/spyth — not a name to teach.
        assert!(permagent::events::voice_pronounce::first_teachable(&[
            "spith".into(),
            "spyth".into(),
            "peth".into(),
        ])
        .is_none());
        assert_eq!(
            permagent::events::voice_pronounce::first_teachable(&[
                "elspeth".into(),
                "speth".into(),
            ])
            .as_deref(),
            Some("elspeth")
        );
    }

    /// The pinned Kokoro asset URLs must satisfy the DownloadManager's strict
    /// policy (HTTPS + allowlisted release path), and the pinned digests must
    /// be well-formed — otherwise the voice-model download endpoint is dead on
    /// arrival.
    #[test]
    fn kokoro_urls_and_digests_pass_download_policy() {
        permagent::download_manager::validate_download_url(KOKORO_MODEL_URL)
            .expect("model URL must be allowlisted");
        permagent::download_manager::validate_download_url(KOKORO_VOICES_URL)
            .expect("voices URL must be allowlisted");
        for digest in [KOKORO_MODEL_SHA256, KOKORO_VOICES_SHA256] {
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    /// End-to-end proof that the shipping Kokoro release URL fetches via the
    /// production `DownloadManager`, passes SHA-256 verification against the
    /// pinned digest, and lands the canonical asset. Uses the smaller voices
    /// file (28MB) and a tempdir — never the real models path.
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
            vec![permagent::download_manager::DownloadFile::new(
                KOKORO_VOICES_URL,
                dest.clone(),
                Some(KOKORO_VOICES_SHA256.to_string()),
            )],
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
