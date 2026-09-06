//! Dedicated `/voice` WebSocket for the push-to-talk voice loop.
//!
//! Protocol (Phase 1, non-streaming):
//!   Client → Server:
//!     Text: {"type":"start","sample_rate":16000}
//!     Binary: [pcm_f32le audio chunks while push-to-talk held]
//!     Text: {"type":"stop"}
//!     Text: {"type":"wake_start","sample_rate":16000}   (hands-free: begin keyword spotting)
//!     Text: {"type":"wake_stop"}
//!     Text: {"type":"enroll_start"}
//!     Text: {"type":"enroll_done"}
//!     Text: {"type":"enroll_skip"}
//!     Text: {"type":"enroll_clear"}
//!   Server → Client:
//!     Text: {"type":"voice_print","enrolled":true|false}  (after ready)
//!     Text: {"type":"enroll_status","have":1,"need":3,"prompt":"..."}
//!     Text: {"type":"enrolled"}
//!     Text: {"type":"enroll_retry","reason":"..."}
//!     Text: {"type":"enroll_cleared"}
//!     Text: {"type":"transcript_partial","text":"..."} (optional,
//!       provisional online-STT hypothesis)
//!     Text: {"type":"transcript","text":"..."} (authoritative final)
//!     Text: {"type":"reply_start"}
//!     Text: {"type":"audio_segment", ...} (immediately before each binary frame)
//!     Binary: [tts pcm_f32le audio]
//!     Text: {"type":"clipboard","text":"..."}  (as soon as copy_to_clipboard
//!            runs — not after TTS — so the phone can write the pasteboard
//!            before the user switches to Notes)
//!     Text: {"type":"reply_text","text":"..."}
//!     Text: {"type":"navigate",...}            (after narration; desktop only)
//!     Text: {"type":"reply_end","sample_rate":24000}
//!     Text: {"type":"turn_outcome","outcome":"empty_stt","reason":"near_silent_pcm"}
//!     Text: {"type":"idle"}                    (legacy ready/reset signal)
//!
//! `audio_segment` is additive framing for clients that want synchronized
//! transcript presentation. Its `text` is exactly the text sent to TTS,
//! `segment_id` starts at zero for each reply and increases monotonically,
//! and `word_timings` use UTF-16 offsets (the native indexing unit for iOS)
//! plus millisecond offsets relative to that segment. Current Kokoro output
//! has no alignments, so the timings are explicitly marked
//! `estimated_proportional`; they are deterministic estimates, not phoneme
//! timestamps. Older clients can ignore the metadata text frame and continue
//! consuming the following binary frame, `reply_text`, and `reply_end`.
//!     Text: {"type":"error","message":"..."}
//!     Text: {"type":"wake_status","active":true,"phrase":"Hey <agent>"}
//!     Text: {"type":"wake","kind":"wake"|"stop"}        (keyword detected)
//!     Text: {"type":"stopped"}                          (spoken stop cancelled the in-flight turn)
//!
//! Wake mode: while no recording is active, binary frames are mic MONITOR
//! audio fed to the on-device keyword spotter (voice::kws) — never to STT,
//! never off-machine. Detections come back as `wake` events; a stop phrase
//! that lands while a reply is still being generated cancels the turn
//! server-side and is announced with `stopped`.
//!
//! Speaker print (N3): after `ready` the hub sends `voice_print`. iOS is the
//! enrollment UI (three orb sentences). Desktop/watch share the same print
//! and fail OPEN when none exists. On `stop`, if a print exists and cosine
//! is below threshold, the hub sends `idle` and skips STT — same as empty
//! speech. Score is logged; audio is not.

use crate::routes::errors::ErrorResponse;
use crate::state::{build_kokoro_tts, AppState, SharedSpeakerVerifier, SharedTts};
use crate::voice::provider::{
    AudioOutput, StreamingSttEvent, StreamingSttGate, StreamingSttSession, SttConfig, TtsConfig,
};
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
use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;

/// The style contract for a SPOKEN reply, appended to the agent's system prompt on
/// the voice path only. The chat path never sees it — a chat reply is read, and
/// most of what follows (no markdown, contractions, delivery tags, "never open in
/// silence") would be wrong advice there.
///
/// The silence rule at the end came out of the voice-model bench
/// (`docs/research/VOICE_MODEL_BENCH_2026-08-25.md`): every candidate opened some
/// turns with a bare tool call and no spoken text, up to 7 turns in 20, and the
/// user then hears nothing for a whole tool round trip. It is model-independent,
/// so it belongs in the prompt rather than in the model choice.
pub const VOICE_REPLY_STYLE: &str =
    "The user is speaking to you by voice. Reply in natural conversational speech: \
     short sentences, contractions, concise and direct. No markdown, no bullet points, \
     no numbered lists, no code blocks. Keep replies brief — 1-3 sentences for simple \
     questions. For a story or a longer ask, keep going — do not stop at three \
     sentences to ask if they want more. Never say 'do you want me to continue', \
     'shall I go on', or any mid-reply continue offer; if there is more, say it. \
     Complete the answer or action they actually asked for before offering any \
     optional adjacent detail. Never replace the conclusion with a generic \
     'there is more' cue. Use standard spoken spellings, not eye dialect such \
     as 'doin'', 'gonna', or dropped final letters. \
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
     the words is not the same as putting them on the clipboard. \
     NEVER OPEN A TURN IN SILENCE. If a tool call is the right next step, say one \
     short sentence first or alongside it — 'Let me check.', 'One sec, pulling that \
     up.' — and only then call the tool. A turn that begins with a bare tool call and \
     no words is dead air in the user's ear for the whole round trip, and they cannot \
     tell it from a crash.";

// ── The voice model (#voice-latency): which model answers a spoken turn ──────

/// The provider built for the configured voice route, kept across turns.
///
/// Building a provider is cheap but not free (an HTTP client and a config read),
/// and the voice path is the one place in the daemon where a hundred milliseconds
/// is audible. Keyed on the resolved (provider, model) so a config change during
/// a session is picked up on the next turn rather than pinned for the life of the
/// process.
/// The voice route that is live plus the provider built for it.
type CachedVoiceProvider = (
    permagent::config::VoiceModel,
    Arc<dyn permagent::providers::base::Provider>,
);

static VOICE_PROVIDER_CACHE: tokio::sync::OnceCell<
    tokio::sync::Mutex<Option<CachedVoiceProvider>>,
> = tokio::sync::OnceCell::const_new();

async fn voice_provider_cache() -> &'static tokio::sync::Mutex<Option<CachedVoiceProvider>> {
    VOICE_PROVIDER_CACHE
        .get_or_init(|| async { tokio::sync::Mutex::new(None) })
        .await
}

/// Point this turn's agent at the configured VOICE model, if one is configured
/// and reachable.
///
/// Returns the route that is now live, or `None` when the turn should run on the
/// session model — which is the case when nothing is configured (the common one),
/// when the model id is invalid, or when the provider cannot be built (no API key,
/// no network). A voice model that cannot be reached must never turn into a failed
/// turn: the user is mid-conversation, and the session model still answers.
///
/// On "turn reasoning off for voice": there is no such switch to throw, and this
/// function deliberately does not pretend otherwise. No in-tree request builder
/// emits a disable-thinking field for a non-Claude model
/// (`formats::anthropic::thinking_type` returns `Disabled` and writes nothing),
/// MiniMax documents that it ignores `thinking: {"type": "disabled"}`, and Claude
/// only thinks when asked. So the `reasoning` flag stays whatever the canonical
/// table says the chosen model actually does — the request log then tells the
/// truth about it. The way to stop a voice turn thinking is to configure a model
/// that does not think; see `docs/research/VOICE_MODEL_BENCH_2026-08-25.md`.
/// Build the `ModelConfig` for a voice route, or `None` if the model id is not
/// usable. Split out from [`apply_voice_model`] so the fallback path — a bad id
/// must never take a spoken turn down — is testable without a live agent.
fn voice_model_config(
    route: &permagent::config::VoiceModel,
) -> Option<permagent::model::ModelConfig> {
    if route.model.trim().is_empty() || route.provider.trim().is_empty() {
        tracing::warn!(
            target: "permagentd::voice",
            "voice route is missing a provider or a model; this turn runs on the session model"
        );
        return None;
    }
    match permagent::model::ModelConfig::new(&route.model) {
        Ok(config) => Some(config.with_canonical_limits(&route.provider)),
        Err(e) => {
            tracing::warn!(
                target: "permagentd::voice",
                voice_provider = %route.provider,
                voice_model = %route.model,
                error = %e,
                "configured voice model is invalid; this turn runs on the session model"
            );
            None
        }
    }
}

async fn apply_voice_model(
    agent: &Arc<permagent::agents::Agent>,
    session_id: &str,
) -> Option<permagent::config::VoiceModel> {
    let (route, source) = permagent::config::voice_model_from_config()?;
    if source == permagent::config::VoiceModelSource::HalfConfigured {
        tracing::warn!(
            target: "permagentd::voice",
            "only one of `{}`/`{}` is set — a half-configured pair cannot route, so the \
             measured default ({}/{}) applies; set both or set one to `session` to turn the \
             voice model off",
            permagent::config::VOICE_PROVIDER_KEY,
            permagent::config::VOICE_MODEL_KEY,
            route.provider,
            route.model,
        );
    }

    let cache = voice_provider_cache().await;
    let mut cached = cache.lock().await;
    let provider = match cached.as_ref() {
        Some((cached_route, provider)) if *cached_route == route => Arc::clone(provider),
        _ => {
            let model_config = voice_model_config(&route)?;
            let extensions = agent.get_extension_configs().await;
            match permagent::providers::create(&route.provider, model_config, extensions).await {
                Ok(provider) => {
                    *cached = Some((route.clone(), Arc::clone(&provider)));
                    provider
                }
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::voice",
                        voice_provider = %route.provider,
                        voice_model = %route.model,
                        error = %e,
                        "configured voice model is unreachable; this turn runs on the session model"
                    );
                    return None;
                }
            }
        }
    };
    drop(cached);

    if let Err(e) = agent.update_provider(provider, session_id).await {
        tracing::warn!(
            target: "permagentd::voice",
            voice_provider = %route.provider,
            voice_model = %route.model,
            error = %e,
            "could not switch this session to the voice model; running on the session model"
        );
        return None;
    }
    Some(route)
}

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
        derived, "pronunciation saved"
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
        .route("/voice/speaker/models", get(speaker_models_status))
        .route(
            "/voice/speaker/models/download",
            post(download_speaker_models),
        )
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

// ── Learned speaker identity model ─────────────────────────────────────────

#[derive(Serialize)]
pub struct SpeakerModelStatus {
    pub models_present: bool,
    pub verifier_loaded: bool,
    pub enrolled: bool,
    pub downloading: bool,
}

async fn current_speaker_status(state: &Arc<AppState>) -> SpeakerModelStatus {
    let models_present =
        crate::voice::speaker_print::SpeakerModelPaths::default_paths().models_exist();
    let verifier_loaded = state.speaker_verifier.read().await.is_some();
    let downloading = get_download_manager()
        .get_progress(crate::voice::speaker_print::DOWNLOAD_ID)
        .is_some_and(|p| p.status == DownloadStatus::Downloading);
    SpeakerModelStatus {
        models_present,
        verifier_loaded,
        enrolled: crate::voice::speaker_print::load().is_some(),
        downloading,
    }
}

async fn speaker_models_status(State(state): State<Arc<AppState>>) -> Json<SpeakerModelStatus> {
    Json(current_speaker_status(&state).await)
}

async fn start_speaker_model_download(state: &Arc<AppState>) -> anyhow::Result<()> {
    let paths = crate::voice::speaker_print::SpeakerModelPaths::default_paths();
    if paths.models_exist() {
        if state.speaker_verifier.read().await.is_none() {
            let verifier =
                tokio::task::spawn_blocking(crate::state::build_speaker_verifier).await?;
            if let Some(verifier) = verifier {
                *state.speaker_verifier.write().await = Some(verifier);
            }
        }
        return Ok(());
    }

    let files = vec![permagent::download_manager::DownloadFile::new(
        crate::voice::speaker_print::MODEL_URL,
        paths.model_path,
        Some(crate::voice::speaker_print::MODEL_SHA256.to_string()),
    )];
    let slot: SharedSpeakerVerifier = state.speaker_verifier.clone();
    let on_complete: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        tokio::spawn(async move {
            match tokio::task::spawn_blocking(crate::state::build_speaker_verifier).await {
                Ok(Some(verifier)) => {
                    *slot.write().await = Some(verifier);
                    tracing::info!(
                        target: "permagentd::voice",
                        "CAM++ speaker identity model hot-loaded"
                    );
                }
                _ => tracing::error!(
                    target: "permagentd::voice",
                    "Speaker identity model downloaded but failed to load"
                ),
            }
        });
    });
    get_download_manager()
        .download_model_sharded(
            crate::voice::speaker_print::DOWNLOAD_ID.to_string(),
            files,
            crate::voice::speaker_print::MODEL_BYTES,
            Some(on_complete),
        )
        .await?;
    Ok(())
}

async fn download_speaker_models(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<SpeakerModelStatus>), ErrorResponse> {
    start_speaker_model_download(&state).await.map_err(|e| {
        ErrorResponse::internal(format!("Speaker model download failed to start: {e}"))
    })?;
    let status = current_speaker_status(&state).await;
    let code = if status.verifier_loaded {
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
    #[serde(rename = "enroll_start")]
    EnrollStart,
    #[serde(rename = "enroll_done")]
    EnrollDone,
    #[serde(rename = "enroll_skip")]
    EnrollSkip,
    #[serde(rename = "enroll_clear")]
    EnrollClear,
}

enum StreamingSttCommand {
    Audio(Vec<f32>),
    Finish,
    Cancel,
}

fn spawn_streaming_stt_worker(
    mut session: Box<dyn StreamingSttSession>,
) -> (
    SyncSender<StreamingSttCommand>,
    tokio::sync::mpsc::Receiver<StreamingSttEvent>,
    tokio::task::JoinHandle<()>,
) {
    let (command_tx, command_rx) = sync_channel(4);
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
    let worker = tokio::task::spawn_blocking(move || {
        while let Ok(command) = command_rx.recv() {
            let result = match command {
                StreamingSttCommand::Audio(samples) => session.push_audio(&samples),
                StreamingSttCommand::Finish => {
                    let result = session.finish();
                    if let Ok(events) = result {
                        for event in events {
                            if event_tx.blocking_send(event).is_err() {
                                return;
                            }
                        }
                    }
                    return;
                }
                StreamingSttCommand::Cancel => {
                    session.cancel();
                    return;
                }
            };
            let Ok(events) = result else {
                return;
            };
            for event in events {
                if event_tx.blocking_send(event).is_err() {
                    return;
                }
            }
        }
        session.cancel();
    });
    (command_tx, event_rx, worker)
}

fn cancel_streaming_stt_worker(
    command_tx: &mut Option<SyncSender<StreamingSttCommand>>,
    event_rx: &mut Option<tokio::sync::mpsc::Receiver<StreamingSttEvent>>,
    gate: &mut Option<StreamingSttGate>,
    pending_partial: &mut Option<String>,
) {
    if let Some(command_tx) = command_tx.take() {
        let _ = command_tx.try_send(StreamingSttCommand::Cancel);
    }
    *event_rx = None;
    if let Some(gate) = gate.as_mut() {
        gate.cancel();
    }
    *gate = None;
    *pending_partial = None;
}

fn streaming_worker_is_available(worker: Option<&tokio::task::JoinHandle<()>>) -> bool {
    worker.map_or(true, |worker| worker.is_finished())
}

type BatchSttTask = tokio::task::JoinHandle<anyhow::Result<String>>;

fn batch_worker_is_available(worker: Option<&BatchSttTask>) -> bool {
    worker.map_or(true, |worker| worker.is_finished())
}

fn native_stt_workers_are_available(
    streaming_worker: Option<&tokio::task::JoinHandle<()>>,
    batch_worker: Option<&BatchSttTask>,
) -> bool {
    streaming_worker_is_available(streaming_worker) && batch_worker_is_available(batch_worker)
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "transcript")]
    Transcript { text: String },
    #[serde(rename = "transcript_partial")]
    TranscriptPartial { text: String },
    #[serde(rename = "reply_start")]
    ReplyStart,
    /// Metadata for the binary TTS frame that follows immediately. This is a
    /// separate frame rather than an attribute on the audio bytes so existing
    /// clients can continue to ignore it without changing their PCM decoder.
    #[serde(rename = "audio_segment")]
    AudioSegment {
        segment_id: u64,
        text: String,
        sample_rate: u32,
        duration_ms: u64,
        timing_source: &'static str,
        word_timings: Vec<WordTiming>,
    },
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
    /// Legacy ready/reset signal after an empty or too-short capture. New
    /// clients receive a typed `turn_outcome` immediately before this frame;
    /// older clients remain silently compatible with the established behavior.
    #[serde(rename = "idle")]
    Idle,
    /// Additive terminal evidence for a capture that will not produce a reply.
    /// This always precedes the legacy `idle` frame. Clients that do not know
    /// this frame retain the old ready/reset behavior; newer clients can render
    /// a recoverable, precise explanation without inspecting daemon logs.
    #[serde(rename = "turn_outcome")]
    TurnOutcome {
        outcome: VoiceTurnOutcome,
        reason: VoiceTurnOutcomeReason,
    },
    #[serde(rename = "ready")]
    Ready,
    /// Wake-listening state after a `wake_start`/`wake_stop`. `phrase` is the
    /// human-readable wake phrase derived from the persona for the UI hint; `reason`
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
    #[serde(rename = "voice_print")]
    VoicePrint {
        enrolled: bool,
        available: bool,
        downloading: bool,
    },
    /// Learned identity rejected this talker. Unlike generic idle, clients use
    /// this to require a short quiet interval before VAD can open again.
    #[serde(rename = "speaker_rejected")]
    SpeakerRejected,
    #[serde(rename = "enroll_status")]
    EnrollStatus {
        have: usize,
        need: usize,
        prompt: Option<String>,
    },
    #[serde(rename = "enrolled")]
    Enrolled,
    #[serde(rename = "enroll_retry")]
    EnrollRetry { reason: String },
    #[serde(rename = "enroll_cleared")]
    EnrollCleared,
}

/// Closed terminal outcomes exposed to voice clients. Keep this intentionally
/// small: the wire frame communicates state, never raw PCM, thresholds, or
/// transcript content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum VoiceTurnOutcome {
    CaptureRejectedMalformed,
    CaptureRejectedShort,
    EmptyStt,
    SttBusy,
}

/// Closed, privacy-safe evidence for a no-reply terminal outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum VoiceTurnOutcomeReason {
    MalformedPcm,
    ShortCapture,
    ZeroPcm,
    NearSilentPcm,
    FiniteSignalNoWords,
    SttBusy,
}

/// A word's estimated position inside one synthesized segment.
///
/// Offsets are UTF-16 code-unit ranges because that is the indexing unit used
/// by `NSString`/`NSRange` on iOS. `end_utf16` is exclusive. The server keeps
/// this type deliberately independent of any TTS implementation: once a
/// backend exposes alignments, the same wire shape can carry measured values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WordTiming {
    word: String,
    start_ms: u64,
    end_ms: u64,
    start_utf16: u32,
    end_utf16: u32,
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

fn terminal_idle_frames(
    outcome: VoiceTurnOutcome,
    reason: VoiceTurnOutcomeReason,
) -> [ServerMessage; 2] {
    [
        ServerMessage::TurnOutcome { outcome, reason },
        // This legacy frame must remain second so established clients reset to
        // ready even if they ignore the additive outcome frame.
        ServerMessage::Idle,
    ]
}

async fn send_terminal_idle(
    socket: &mut WebSocket,
    outcome: VoiceTurnOutcome,
    reason: VoiceTurnOutcomeReason,
) {
    for frame in terminal_idle_frames(outcome, reason) {
        // Attempt Idle even if a newer-frame write fails, preserving the
        // compatibility behavior whenever the socket remains usable.
        let _ = socket.send(send_json(&frame)).await;
    }
}

enum StreamFinalResult {
    Final(String),
    Fallback,
    Disconnected,
    Deferred(Message),
}

enum SttWaitDisposition {
    KeepWaiting,
    Deferred(Message),
    Disconnected,
}

/// Controls which arrive while native STT is running must not turn into a
/// cancellation. A replacement/setup command is deferred for the outer loop;
/// duplicate stop, late PCM, pong, and unknown text are harmlessly consumed.
/// This keeps one in-flight native call per socket without dropping a valid
/// stream merely because a peer sent a late frame.
fn classify_stt_wait_message(message: Message) -> SttWaitDisposition {
    match message {
        Message::Close(_) => SttWaitDisposition::Disconnected,
        Message::Text(text) => {
            let replacement = serde_json::from_str::<ClientMessage>(&text)
                .ok()
                .is_some_and(|message| {
                    matches!(
                        message,
                        ClientMessage::Start { .. }
                            | ClientMessage::WakeStart { .. }
                            | ClientMessage::WakeStop
                            | ClientMessage::EnrollStart
                            | ClientMessage::EnrollDone
                            | ClientMessage::EnrollSkip
                            | ClientMessage::EnrollClear
                    )
                });
            if replacement {
                SttWaitDisposition::Deferred(Message::Text(text))
            } else {
                SttWaitDisposition::KeepWaiting
            }
        }
        Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => SttWaitDisposition::KeepWaiting,
    }
}

async fn handle_stream_event(
    gate: &mut StreamingSttGate,
    event: StreamingSttEvent,
    publish: bool,
    pending_partial: &mut Option<String>,
    socket: &mut WebSocket,
) -> Result<Option<String>, ()> {
    let terminal = gate.accept(event);
    if let Some(StreamingSttEvent::Final { text, .. }) = terminal {
        return Ok(Some(text));
    }
    if let Some(StreamingSttEvent::Partial { text, .. }) = gate.take_partial() {
        if publish {
            socket
                .send(send_json(&ServerMessage::TranscriptPartial { text }))
                .await
                .map_err(|_| ())?;
        } else {
            *pending_partial = Some(text);
        }
    }
    Ok(None)
}

fn speaker_gate_allows_stream_text(gate: Option<&crate::voice::speaker_print::Gate>) -> bool {
    matches!(
        gate,
        Some(
            crate::voice::speaker_print::Gate::Admit { .. }
                | crate::voice::speaker_print::Gate::Open
        )
    )
}

async fn flush_pending_stream_partial(
    pending_partial: &mut Option<String>,
    socket: &mut WebSocket,
) -> Result<(), ()> {
    if let Some(text) = pending_partial.take() {
        socket
            .send(send_json(&ServerMessage::TranscriptPartial { text }))
            .await
            .map_err(|_| ())?;
    }
    Ok(())
}

async fn finish_streaming_stt(
    command_tx: &SyncSender<StreamingSttCommand>,
    event_rx: &mut tokio::sync::mpsc::Receiver<StreamingSttEvent>,
    gate: &mut StreamingSttGate,
    pending_partial: &mut Option<String>,
    socket: &mut WebSocket,
) -> StreamFinalResult {
    if command_tx.try_send(StreamingSttCommand::Finish).is_err() {
        let _ = command_tx.try_send(StreamingSttCommand::Cancel);
        return StreamFinalResult::Fallback;
    }

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    let Some(event) = event else {
                        return StreamFinalResult::Fallback;
                    };
                    match handle_stream_event(gate, event, true, pending_partial, socket).await {
                        Ok(Some(text)) => return StreamFinalResult::Final(text),
                        Ok(None) => {}
                        Err(()) => return StreamFinalResult::Disconnected,
                    }
                }
                message = socket.recv() => {
                    match message {
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                            let _ = command_tx.try_send(StreamingSttCommand::Cancel);
                            return StreamFinalResult::Disconnected;
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            if socket.send(Message::Pong(payload)).await.is_err() {
                                return StreamFinalResult::Disconnected;
                            }
                        }
                        Some(Ok(message)) => {
                            match classify_stt_wait_message(message) {
                                SttWaitDisposition::KeepWaiting => {}
                                SttWaitDisposition::Deferred(message) => {
                                    return StreamFinalResult::Deferred(message);
                                }
                                SttWaitDisposition::Disconnected => {
                                    let _ = command_tx.try_send(StreamingSttCommand::Cancel);
                                    return StreamFinalResult::Disconnected;
                                }
                            }
                        }
                    }
                }
            }
        }
    })
    .await;

    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = command_tx.try_send(StreamingSttCommand::Cancel);
            StreamFinalResult::Fallback
        }
    }
}

enum BatchTranscribeResult {
    Completed(Result<anyhow::Result<String>, tokio::task::JoinError>),
    Disconnected,
    Deferred(Message, BatchSttTask),
}

async fn transcribe_batch_with_socket(
    stt: Arc<dyn crate::voice::SpeechToText>,
    samples: Vec<f32>,
    sample_rate: u32,
    socket: &mut WebSocket,
) -> BatchTranscribeResult {
    let mut stt_task: BatchSttTask = tokio::task::spawn_blocking(move || {
        stt.transcribe(&samples, sample_rate, &SttConfig::default())
    });
    loop {
        tokio::select! {
            result = &mut stt_task => return BatchTranscribeResult::Completed(result),
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        stt_task.abort();
                        return BatchTranscribeResult::Disconnected;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            stt_task.abort();
                            return BatchTranscribeResult::Disconnected;
                        }
                    }
                    Some(Ok(message)) => {
                        match classify_stt_wait_message(message) {
                            SttWaitDisposition::KeepWaiting => {}
                            SttWaitDisposition::Deferred(message) => {
                                // `spawn_blocking` native work cannot be hard
                                // cancelled. Retain the handle so a deferred
                                // replacement cannot start a second call on
                                // this socket while the first one unwinds.
                                return BatchTranscribeResult::Deferred(message, stt_task);
                            }
                            SttWaitDisposition::Disconnected => {
                                stt_task.abort();
                                return BatchTranscribeResult::Disconnected;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Derive stable word ranges and proportional timings when a TTS backend does
/// not expose alignments. The full segment duration is apportioned by word
/// length, making this pure and repeatable for a given text/duration pair.
/// Whitespace is intentionally omitted from ranges; punctuation attached to a
/// word remains part of that word because it is spoken by the synthesizer.
fn estimate_word_timings(text: &str, duration_ms: u64) -> Vec<WordTiming> {
    let mut words: Vec<(String, u32, u32)> = Vec::new();
    let mut current = String::new();
    let mut start_utf16 = 0u32;
    let mut cursor_utf16 = 0u32;

    for ch in text.chars() {
        let width = ch.len_utf16() as u32;
        if ch.is_whitespace() {
            if !current.is_empty() {
                words.push((std::mem::take(&mut current), start_utf16, cursor_utf16));
            }
        } else {
            if current.is_empty() {
                start_utf16 = cursor_utf16;
            }
            current.push(ch);
        }
        cursor_utf16 += width;
    }
    if !current.is_empty() {
        words.push((current, start_utf16, cursor_utf16));
    }

    let total_units: u64 = words
        .iter()
        .map(|(_, start, end)| u64::from(end.saturating_sub(*start)))
        .sum();
    if total_units == 0 {
        return Vec::new();
    }

    let mut elapsed_units = 0u64;
    words
        .into_iter()
        .map(|(word, start_utf16, end_utf16)| {
            let word_units = u64::from(end_utf16.saturating_sub(start_utf16));
            let start_ms = duration_ms.saturating_mul(elapsed_units) / total_units;
            elapsed_units = elapsed_units.saturating_add(word_units);
            let end_ms = duration_ms.saturating_mul(elapsed_units) / total_units;
            WordTiming {
                word,
                start_ms,
                end_ms,
                start_utf16,
                end_utf16,
            }
        })
        .collect()
}

/// Send the synchronization frame and its PCM frame as one ordered pair.
/// Returning `false` means cancellation or a missing socket; callers should
/// stop the turn without attempting to send another frame.
async fn send_tts_segment(
    socket: &mut WebSocket,
    audio: &AudioOutput,
    text: String,
    segment_id: u64,
    cancelled: &std::sync::atomic::AtomicBool,
) -> bool {
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let metadata = audio_segment_metadata(audio, &text, segment_id);
    if socket.send(send_json(&metadata)).await.is_err() {
        return false;
    }
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let bytes: Vec<u8> = audio.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    socket.send(Message::Binary(bytes.into())).await.is_ok()
}

fn audio_segment_metadata(audio: &AudioOutput, text: &str, segment_id: u64) -> ServerMessage {
    let duration_ms = if audio.sample_rate == 0 {
        0
    } else {
        (audio.samples.len() as u64).saturating_mul(1_000) / u64::from(audio.sample_rate)
    };
    ServerMessage::AudioSegment {
        segment_id,
        text: text.to_string(),
        sample_rate: audio.sample_rate,
        duration_ms,
        timing_source: "estimated_proportional",
        word_timings: estimate_word_timings(text, duration_ms),
    }
}

fn enroll_status_msg(have: usize) -> ServerMessage {
    ServerMessage::EnrollStatus {
        have,
        need: crate::voice::speaker_print::NEED_UTTERANCES,
        prompt: crate::voice::speaker_print::prompt_at(have).map(str::to_string),
    }
}

async fn voice_print_msg(state: &Arc<AppState>) -> ServerMessage {
    let status = current_speaker_status(state).await;
    ServerMessage::VoicePrint {
        enrolled: status.enrolled,
        available: status.verifier_loaded,
        downloading: status.downloading,
    }
}

async fn speaker_embedding(
    state: &Arc<AppState>,
    samples: &[f32],
    sample_rate: u32,
) -> anyhow::Result<Option<Vec<f32>>> {
    let verifier = state
        .speaker_verifier
        .read()
        .await
        .clone()
        .ok_or_else(|| anyhow::anyhow!("learned speaker verifier is not loaded"))?;
    let owned = samples.to_vec();
    tokio::task::spawn_blocking(move || verifier.extract(&owned, sample_rate)).await?
}

async fn learned_speaker_gate(
    state: &Arc<AppState>,
    samples: &[f32],
    sample_rate: u32,
) -> crate::voice::speaker_print::Gate {
    let Some(print) = crate::voice::speaker_print::load() else {
        return crate::voice::speaker_print::Gate::Open;
    };
    match speaker_embedding(state, samples, sample_rate).await {
        Ok(Some(embedding)) => crate::voice::speaker_print::gate_against(Some(&print), &embedding),
        Ok(None) | Err(_) => crate::voice::speaker_print::Gate::Unavailable,
    }
}

fn early_speaker_gate_ready(
    enrolling: bool,
    already_checked: bool,
    enrolled: bool,
    samples: usize,
    sample_rate: u32,
) -> bool {
    !enrolling
        && !already_checked
        && enrolled
        && sample_rate > 0
        && samples >= sample_rate as usize * 2
}

/// Re-place a parked word after reconnect. iOS barge-in closes the socket
/// (this morning: teach at 11:14:29, close 1001 at 11:14:34) and the new
/// socket used to never send `teach` again — the agent still spoke ASK_FIRST
/// on the dying socket, then the orb came back empty.
fn pending_teach_msg(session_id: &str) -> Option<ServerMessage> {
    let pending = permagent::events::voice_pronounce::peek(session_id)?;
    Some(ServerMessage::Teach {
        word: permagent::events::voice_pronounce::display_word(&pending.word),
    })
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
    let socket_epoch = next_voice_socket_epoch();
    // Snapshot the hot-swappable TTS slot once for this session.
    let tts_opt = state.voice_tts.read().await.clone();

    tracing::info!(
        target: "permagentd::voice",
        event = "voice_socket",
        stage = "connected",
        socket_epoch,
        session_id = session_id.as_deref().unwrap_or("voice-anon"),
        client = origin.client.wire_name(),
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
                event = "voice_socket",
                stage = "rejected_provider_unavailable",
                socket_epoch,
                session_id = session_id.as_deref().unwrap_or("voice-anon"),
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
        tracing::warn!(
            target: "permagentd::voice",
            event = "voice_socket",
            stage = "ready_send_disconnected",
            socket_epoch,
            session_id = session_id.as_deref().unwrap_or("voice-anon"),
            "Failed to send ready — client disconnected"
        );
        return;
    }
    tracing::info!(
        target: "permagentd::voice",
        event = "voice_socket",
        stage = "ready",
        socket_epoch,
        session_id = session_id.as_deref().unwrap_or("voice-anon"),
        "Voice WebSocket ready"
    );
    let _ = socket.send(send_json(&voice_print_msg(&state).await)).await;
    let remainder_key = session_id.as_deref().unwrap_or("voice-anon");
    if let Some(teach) = pending_teach_msg(remainder_key) {
        tracing::info!(
            target: "permagentd::voice",
            "replaying pending teach after connect"
        );
        let _ = socket.send(send_json(&teach)).await;
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
    let mut capture_health = CaptureHealth::default();
    let mut recording = false;
    let mut streaming_command_tx: Option<SyncSender<StreamingSttCommand>> = None;
    let mut streaming_event_rx: Option<tokio::sync::mpsc::Receiver<StreamingSttEvent>> = None;
    let mut streaming_gate: Option<StreamingSttGate> = None;
    let mut streaming_worker: Option<tokio::task::JoinHandle<()>> = None;
    let mut streaming_blocked = false;
    let mut pending_stream_partial: Option<String> = None;
    let mut streaming_output_closed = false;
    let mut streaming_failed = false;
    // At most one actionable control is deferred while native STT runs. A
    // queue would let a peer build an unbounded backlog and obscure which
    // replacement generation actually supersedes the turn.
    let mut deferred_message: Option<Message> = None;
    let mut batch_worker: Option<BatchSttTask> = None;
    let mut turn_generation = 0u64;
    // A supported online provider gets a single bounded worker below. Moonshine
    // remains final-only batch STT: no rolling re-transcription is attempted,
    // and the batch path remains the fallback when no online model is loaded.
    let mut recording_turn_id: Option<u64> = None;
    let mut recording_started_at: Option<std::time::Instant> = None;
    let mut capture_first_received_at: Option<std::time::Instant> = None;
    let mut capture_last_received_at: Option<std::time::Instant> = None;
    // Keep a turn clock from Start through Stop so a socket that disappears
    // during capture still gets a terminal outcome rather than only the
    // generic handler-exit line.
    let mut active_telemetry: Option<VoiceTurnTelemetry> = None;
    // Set after the early 2s learned-speaker check. An admitted turn does not
    // pay for the same embedding again at Stop.
    let mut speaker_gate: Option<crate::voice::speaker_print::Gate> = None;
    let mut client_sample_rate: u32 = 16000;
    // Per-socket enrollment buffer. `Some` means Stop collects a print
    // utterance and must not run STT or the pronunciation teach path.
    let mut enroll: Option<Vec<Vec<f32>>> = None;
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

    let mut socket_close_reason = "peer_eof";
    tracing::info!(target: "permagentd::voice", "Entering message loop");
    'voice_loop: while let Some(result) = if let Some(message) = deferred_message.take() {
        Some(Ok(message))
    } else if streaming_event_rx.is_some() && !streaming_output_closed {
        let event_rx = streaming_event_rx
            .as_mut()
            .expect("stream receiver present");
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(event) => {
                        let Some(gate) = streaming_gate.as_mut() else {
                            continue 'voice_loop;
                        };
                        let publish = speaker_gate_allows_stream_text(speaker_gate.as_ref());
                        match handle_stream_event(
                            gate,
                            event,
                            publish,
                            &mut pending_stream_partial,
                            &mut socket,
                        )
                        .await
                        {
                            Ok(Some(_)) => {
                                // A provider final before Stop violates the
                                // stream contract; fall back to batch at Stop.
                                streaming_failed = true;
                                gate.cancel();
                            }
                            Ok(None) => {}
                            Err(()) => {
                                socket_close_reason = "stream_partial_send_disconnected";
                                break 'voice_loop;
                            }
                        }
                        continue 'voice_loop;
                    }
                    None => {
                        streaming_output_closed = true;
                        socket.recv().await
                    }
                }
            }
            result = socket.recv() => result,
        }
    } else {
        socket.recv().await
    } {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                socket_close_reason = "transport_recv_error";
                tracing::warn!(
                    target: "permagentd::voice",
                    socket_epoch,
                    "WebSocket recv error: {}",
                    e
                );
                break;
            }
        };
        match msg {
            Message::Text(text) => {
                let text_str: &str = &text;
                match serde_json::from_str::<ClientMessage>(text_str) {
                    Ok(ClientMessage::Start { sample_rate }) => {
                        turn_generation = turn_generation.wrapping_add(1);
                        if let Some(mut prior) = active_telemetry.take() {
                            prior.set_stop_reason("replaced_by_start");
                            prior.record_capture_health(std::mem::take(&mut capture_health));
                            prior.log_outcome("capture_replaced");
                        }
                        audio_buffer.clear();
                        recording = true;
                        let turn_id = next_voice_turn_id();
                        recording_turn_id = Some(turn_id);
                        let started_at = std::time::Instant::now();
                        recording_started_at = Some(started_at);
                        active_telemetry = Some(VoiceTurnTelemetry::new(
                            turn_id,
                            socket_epoch,
                            session_id.as_deref(),
                            started_at,
                        ));
                        capture_first_received_at = None;
                        capture_last_received_at = None;
                        speaker_gate = None;
                        if batch_worker_is_available(batch_worker.as_ref()) {
                            // A finished native task is safe to reap before a
                            // replacement turn. If it is still running, keep
                            // the handle and fail closed at the next Stop.
                            batch_worker = None;
                        }
                        client_sample_rate = sample_rate.unwrap_or(16000).max(1);
                        streaming_blocked = !native_stt_workers_are_available(
                            streaming_worker.as_ref(),
                            batch_worker.as_ref(),
                        );
                        if streaming_worker_is_available(streaming_worker.as_ref()) {
                            streaming_worker = None;
                        }
                        cancel_streaming_stt_worker(
                            &mut streaming_command_tx,
                            &mut streaming_event_rx,
                            &mut streaming_gate,
                            &mut pending_stream_partial,
                        );
                        streaming_output_closed = false;
                        streaming_failed = false;
                        if enroll.is_none() && !streaming_blocked {
                            if let Some(capability) = stt.streaming_capability() {
                                match capability.start_stream(
                                    client_sample_rate,
                                    &SttConfig::default(),
                                    turn_id,
                                ) {
                                    Ok(stream) => {
                                        let (command_tx, event_rx, worker) =
                                            spawn_streaming_stt_worker(stream);
                                        streaming_command_tx = Some(command_tx);
                                        streaming_event_rx = Some(event_rx);
                                        streaming_gate = Some(StreamingSttGate::new(turn_id));
                                        streaming_worker = Some(worker);
                                    }
                                    Err(error) => {
                                        tracing::debug!(
                                            target: "permagentd::voice",
                                            turn_id,
                                            "optional streaming STT unavailable; retaining batch fallback: {error}"
                                        );
                                    }
                                }
                            }
                        }
                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            stage = "capture_started",
                            turn_id,
                            socket_epoch,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            sample_rate = client_sample_rate,
                            "voice capture started"
                        );
                    }
                    Ok(ClientMessage::Stop) if recording => {
                        recording = false;
                        let pipeline_start = std::time::Instant::now();
                        let turn_id = recording_turn_id.take().unwrap_or_else(next_voice_turn_id);
                        let generation_at_stop = turn_generation;
                        let turn_started_at = recording_started_at.take().unwrap_or(pipeline_start);
                        let audio_duration_s =
                            audio_buffer.len() as f32 / client_sample_rate as f32;
                        let mut telemetry = active_telemetry.take().unwrap_or_else(|| {
                            VoiceTurnTelemetry::new(
                                turn_id,
                                socket_epoch,
                                session_id.as_deref(),
                                turn_started_at,
                            )
                        });
                        telemetry.set_stop_reason("client_stop");
                        let completed_health = std::mem::take(&mut capture_health);
                        telemetry.record_capture_health(completed_health.clone());
                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            stage = "capture_stopped",
                            turn_id,
                            socket_epoch,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            stop_reason = "client_stop",
                            "voice capture stopped"
                        );
                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            turn_id,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            stage = "capture_received",
                            stage_elapsed_ms = telemetry.elapsed_ms(),
                            stage_duration_ms = capture_first_received_at
                                .map(|at| at.duration_since(turn_started_at).as_millis())
                                .unwrap_or(0),
                            capture_receive_span_ms = capture_first_received_at
                                .zip(capture_last_received_at)
                                .map(|(first, last)| last.duration_since(first).as_millis())
                                .unwrap_or(0),
                            capture_samples = audio_buffer.len(),
                            capture_duration_ms = (audio_buffer.len() as u64)
                                .saturating_mul(1_000)
                                .checked_div(u64::from(client_sample_rate.max(1)))
                                .unwrap_or(0),
                            sample_rate = client_sample_rate,
                            capture_had_frames = capture_first_received_at.is_some(),
                            "voice capture received"
                        );
                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            turn_id,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            stage = "capture_complete",
                            capture_samples = audio_buffer.len(),
                            capture_duration_ms = (audio_buffer.len() as u64)
                                .saturating_mul(1_000)
                                .checked_div(u64::from(client_sample_rate.max(1)))
                                .unwrap_or(0),
                            capture_wall_ms = turn_started_at.elapsed().as_millis(),
                            capture_receive_span_ms = capture_first_received_at
                                .zip(capture_last_received_at)
                                .map(|(first, last)| last.duration_since(first).as_millis())
                                .unwrap_or(0),
                            capture_had_frames = capture_first_received_at.is_some(),
                            "voice capture complete"
                        );

                        if completed_health.is_malformed() {
                            telemetry.set_empty_reason("malformed_pcm");
                            telemetry.log_outcome("capture_rejected_malformed");
                            audio_buffer.clear();
                            send_terminal_idle(
                                &mut socket,
                                VoiceTurnOutcome::CaptureRejectedMalformed,
                                VoiceTurnOutcomeReason::MalformedPcm,
                            )
                            .await;
                            continue;
                        }

                        // Skip STT for empty or too-short buffers (< 0.3s).
                        // Prevents wasting 25s running STT on silence from
                        // quick press-release or capture failures.
                        let min_samples = (client_sample_rate as f32 * 0.3) as usize;
                        if audio_buffer.len() < min_samples {
                            tracing::info!(
                                target: "permagentd::voice",
                                turn_id,
                                session_id = session_id.as_deref().unwrap_or("voice-anon"),
                                event = "voice_latency_stage",
                                stage = "capture_rejected_short",
                                "Skipping STT: buffer too short ({} samples, {:.2}s < 0.3s minimum)",
                                audio_buffer.len(), audio_duration_s
                            );
                            telemetry.log_outcome("capture_rejected_short");
                            audio_buffer.clear();
                            send_terminal_idle(
                                &mut socket,
                                VoiceTurnOutcome::CaptureRejectedShort,
                                VoiceTurnOutcomeReason::ShortCapture,
                            )
                            .await;
                            continue;
                        }

                        let samples = std::mem::take(&mut audio_buffer);
                        let sr = client_sample_rate;

                        // Enrollment collects a print. Never STT, never agent,
                        // never pronunciation teach — setup is a separate surface.
                        if let Some(ref mut collected) = enroll {
                            match speaker_embedding(&state, &samples, sr).await {
                                Ok(Some(emb)) => {
                                    collected.push(emb);
                                    let have = collected.len();
                                    tracing::info!(
                                        target: "permagentd::voice",
                                        "speaker_print enroll have={have} need={}",
                                        crate::voice::speaker_print::NEED_UTTERANCES
                                    );
                                    let _ = socket.send(send_json(&enroll_status_msg(have))).await;
                                    telemetry.log_outcome("enrollment_clip");
                                }
                                Ok(None) => {
                                    telemetry.log_outcome("enrollment_clip_rejected");
                                    let _ = socket
                                        .send(send_json(&ServerMessage::EnrollRetry {
                                            reason: "That was too short — say the full sentence."
                                                .into(),
                                        }))
                                        .await;
                                }
                                Err(e) => {
                                    telemetry.log_outcome("enrollment_error");
                                    tracing::warn!(
                                        target: "permagentd::voice",
                                        "speaker enrollment inference failed: {e}"
                                    );
                                    let _ = socket
                                        .send(send_json(&ServerMessage::EnrollRetry {
                                            reason: "Voice identity isn't ready yet. Try again in a moment."
                                                .into(),
                                        }))
                                        .await;
                                }
                            }
                            let _ = socket.send(send_json(&ServerMessage::Idle)).await;
                            continue;
                        }

                        // Gate other talkers before STT so a reject feels like
                        // empty speech (idle), not a 4 s think. No print → open.
                        let gate_start = std::time::Instant::now();
                        let gate = match speaker_gate.take() {
                            Some(gate) => gate,
                            None => learned_speaker_gate(&state, &samples, sr).await,
                        };
                        let gate_ms = gate_start.elapsed().as_millis();
                        let speaker_admitted = speaker_gate_allows_stream_text(Some(&gate));
                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            stage = "speaker_gate",
                            turn_id,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            stage_elapsed_ms = telemetry.elapsed_ms(),
                            stage_duration_ms = gate_ms,
                            "voice speaker gate complete"
                        );
                        match gate {
                            crate::voice::speaker_print::Gate::Reject { score } => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "speaker_print reject score={score:.3} gate_ms={gate_ms}"
                                );
                                telemetry.log_outcome("speaker_rejected");
                                cancel_streaming_stt_worker(
                                    &mut streaming_command_tx,
                                    &mut streaming_event_rx,
                                    &mut streaming_gate,
                                    &mut pending_stream_partial,
                                );
                                let _ = socket
                                    .send(send_json(&ServerMessage::SpeakerRejected))
                                    .await;
                                continue;
                            }
                            crate::voice::speaker_print::Gate::Admit { score } => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    "speaker_print admit score={score:.3} gate_ms={gate_ms}"
                                );
                            }
                            crate::voice::speaker_print::Gate::Open => {
                                tracing::debug!(
                                    target: "permagentd::voice",
                                    "speaker_print open gate_ms={gate_ms}"
                                );
                            }
                            crate::voice::speaker_print::Gate::Unavailable => {
                                tracing::warn!(
                                    target: "permagentd::voice",
                                    "speaker_print unavailable gate_ms={gate_ms}; enrolled identity fails closed"
                                );
                                telemetry.log_outcome("speaker_gate_unavailable");
                                cancel_streaming_stt_worker(
                                    &mut streaming_command_tx,
                                    &mut streaming_event_rx,
                                    &mut streaming_gate,
                                    &mut pending_stream_partial,
                                );
                                let _ = socket
                                    .send(send_json(&ServerMessage::SpeakerRejected))
                                    .await;
                                continue;
                            }
                        }

                        if speaker_admitted {
                            if flush_pending_stream_partial(
                                &mut pending_stream_partial,
                                &mut socket,
                            )
                            .await
                            .is_err()
                            {
                                socket_close_reason = "stream_partial_send_disconnected";
                                break 'voice_loop;
                            }
                        }

                        let streamed_transcript = if !streaming_failed {
                            match (
                                streaming_command_tx.as_ref(),
                                streaming_event_rx.as_mut(),
                                streaming_gate.as_mut(),
                            ) {
                                (Some(command_tx), Some(event_rx), Some(gate)) => {
                                    match finish_streaming_stt(
                                        command_tx,
                                        event_rx,
                                        gate,
                                        &mut pending_stream_partial,
                                        &mut socket,
                                    )
                                    .await
                                    {
                                        StreamFinalResult::Final(text) => Some(text),
                                        StreamFinalResult::Fallback => {
                                            streaming_failed = true;
                                            None
                                        }
                                        StreamFinalResult::Disconnected => {
                                            socket_close_reason = "stream_final_disconnected";
                                            break 'voice_loop;
                                        }
                                        StreamFinalResult::Deferred(message) => {
                                            // A replacement/setup control must
                                            // reach Start/Wake/Enroll before
                                            // this turn can fall through to
                                            // batch STT or reply. Do not let an
                                            // old stream answer after a new
                                            // generation has been requested.
                                            deferred_message = Some(message);
                                            cancel_streaming_stt_worker(
                                                &mut streaming_command_tx,
                                                &mut streaming_event_rx,
                                                &mut streaming_gate,
                                                &mut pending_stream_partial,
                                            );
                                            streaming_failed = true;
                                            telemetry.log_outcome("capture_replaced");
                                            continue 'voice_loop;
                                        }
                                    }
                                }
                                _ => None,
                            }
                        } else {
                            None
                        };
                        if streamed_transcript.is_none() {
                            cancel_streaming_stt_worker(
                                &mut streaming_command_tx,
                                &mut streaming_event_rx,
                                &mut streaming_gate,
                                &mut pending_stream_partial,
                            );
                        } else {
                            streaming_command_tx = None;
                            streaming_event_rx = None;
                            streaming_gate = None;
                        }

                        // --- STT ---
                        let stt_start = std::time::Instant::now();
                        let transcript = if let Some(transcript) = streamed_transcript {
                            Ok(Ok(transcript))
                        } else {
                            if !native_stt_workers_are_available(
                                streaming_worker.as_ref(),
                                batch_worker.as_ref(),
                            ) {
                                telemetry.log_outcome("stt_busy");
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: "Voice transcription is still finishing; please try again.".into(),
                                    }))
                                    .await;
                                send_terminal_idle(
                                    &mut socket,
                                    VoiceTurnOutcome::SttBusy,
                                    VoiceTurnOutcomeReason::SttBusy,
                                )
                                .await;
                                continue 'voice_loop;
                            }
                            match transcribe_batch_with_socket(
                                stt.clone(),
                                samples,
                                sr,
                                &mut socket,
                            )
                            .await
                            {
                                BatchTranscribeResult::Completed(result) => result,
                                BatchTranscribeResult::Disconnected => {
                                    socket_close_reason = "batch_stt_disconnected";
                                    break 'voice_loop;
                                }
                                BatchTranscribeResult::Deferred(message, task) => {
                                    deferred_message = Some(message);
                                    batch_worker = Some(task);
                                    telemetry.log_outcome("stt_interrupted_control");
                                    continue 'voice_loop;
                                }
                            }
                        };
                        let stt_ms = stt_start.elapsed().as_millis();
                        telemetry.stt_ms = Some(stt_ms);
                        telemetry.log_stage("stt", stt_start);

                        let transcript = match transcript {
                            Ok(Ok(t)) => t,
                            Ok(Err(e)) => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    event = "voice_latency_stage",
                                    stage = "stt_complete",
                                    stt_outcome = "error",
                                    turn_id,
                                    socket_epoch,
                                    session_id = session_id.as_deref().unwrap_or("voice-anon"),
                                    stt_ms,
                                    "voice transcription failed"
                                );
                                telemetry.log_outcome("stt_error");
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("STT failed: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                            Err(e) => {
                                tracing::info!(
                                    target: "permagentd::voice",
                                    event = "voice_latency_stage",
                                    stage = "stt_complete",
                                    stt_outcome = "task_panic",
                                    turn_id,
                                    socket_epoch,
                                    session_id = session_id.as_deref().unwrap_or("voice-anon"),
                                    stt_ms,
                                    "voice transcription task failed"
                                );
                                telemetry.log_outcome("stt_task_panic");
                                let _ = socket
                                    .send(send_json(&ServerMessage::Error {
                                        message: format!("STT task panicked: {}", e),
                                    }))
                                    .await;
                                continue;
                            }
                        };

                        tracing::info!(
                            target: "permagentd::voice",
                            event = "voice_latency_stage",
                            stage = "stt_complete",
                            turn_id,
                            socket_epoch,
                            session_id = session_id.as_deref().unwrap_or("voice-anon"),
                            stt_ms,
                            transcript_chars = transcript.chars().count(),
                            stt_outcome = if transcript.is_empty() { "empty" } else { "nonempty" },
                            "voice transcription timing"
                        );

                        if transcript.is_empty() {
                            // 20260821_14: empty STT after real speech (and after
                            // auto-started noise turns) flashed "No speech detected".
                            // Return to ready with no toast.
                            telemetry.set_empty_reason(empty_stt_reason(&completed_health));
                            telemetry.log_outcome("empty_stt");
                            send_terminal_idle(
                                &mut socket,
                                VoiceTurnOutcome::EmptyStt,
                                empty_stt_wire_reason(&completed_health),
                            )
                            .await;
                            continue;
                        }

                        // Post-STT proper-noun correction against Brain entities
                        let transcript = crate::voice::proper_noun_corrector::correct_proper_nouns(
                            &transcript,
                            &entity_dict,
                        );

                        // Fence the reply against a replacement generation
                        // that arrived while STT was finishing. Poll exactly
                        // once: this is deliberately non-blocking and cannot
                        // become a peer-controlled drain loop.
                        if turn_generation != generation_at_stop {
                            telemetry.log_outcome("capture_replaced");
                            continue 'voice_loop;
                        }
                        use futures::FutureExt;
                        match socket.recv().now_or_never() {
                            None => {}
                            Some(None) | Some(Some(Err(_))) | Some(Some(Ok(Message::Close(_)))) => {
                                socket_close_reason = "reply_generation_disconnected";
                                break 'voice_loop;
                            }
                            Some(Some(Ok(Message::Ping(payload)))) => {
                                if socket.send(Message::Pong(payload)).await.is_err() {
                                    socket_close_reason = "reply_generation_disconnected";
                                    break 'voice_loop;
                                }
                            }
                            Some(Some(Ok(message))) => match classify_stt_wait_message(message) {
                                SttWaitDisposition::Deferred(message) => {
                                    deferred_message = Some(message);
                                    telemetry.log_outcome("capture_replaced");
                                    continue 'voice_loop;
                                }
                                SttWaitDisposition::KeepWaiting => {}
                                SttWaitDisposition::Disconnected => {
                                    socket_close_reason = "reply_generation_disconnected";
                                    break 'voice_loop;
                                }
                            },
                        }

                        if socket
                            .send(send_json(&ServerMessage::Transcript {
                                text: transcript.clone(),
                            }))
                            .await
                            .is_err()
                        {
                            telemetry.log_outcome("transcript_send_disconnected");
                            return;
                        }

                        // Unspoken leftover from a spoken-budget cut. "Continue."
                        // last night started a cold agent turn and lost the story.

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
                                    telemetry.log_outcome("reply_start_disconnected");
                                    return;
                                }
                                let mut reply_ctx = VoiceReplyCtx {
                                    state: &state,
                                    transcript: &held,
                                    telemetry,
                                    turn_id,
                                    session_id: session_id.as_deref(),
                                    tts: &tts,
                                    pipeline_start,
                                    stt_ms,
                                    cancelled: cancelled.clone(),
                                    wake: wake.as_ref(),
                                    sample_rate: client_sample_rate,
                                    origin: &origin,
                                };
                                if let Err(e) =
                                    stream_reply_with_tts(&mut reply_ctx, &mut socket).await
                                {
                                    reply_ctx.telemetry.log_outcome("reply_error");
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

                        // Spoken yes/no while a decision is waiting: STAGE it
                        // instead of sending the transcript to the agent (which
                        // cannot answer Tier-2 / live-channel kinds anyway).
                        //
                        // It used to answer, as the user, at any tier — with the
                        // audit recording the principal as the literal string
                        // "voice": no speaker, no device, and on a machine with
                        // no voiceprint enrolled the speaker gate admits anyone.
                        // NIST SP 800-63B-4 §3.2.3.2 bars voice as a biometric
                        // comparison outright and §3.2.3 bars any biometric from
                        // standing alone, so enrolling a print would not have
                        // fixed it. Voice proposes; the tap on the confirm
                        // surface — possession of the unlocked device — commits.
                        if let Some(verdict) =
                            crate::voice::spoken_verdict::spoken_decision_verdict(&transcript)
                        {
                            if let Some(decision_id) =
                                pick_spoken_decision(&state, session_id.as_deref()).await
                            {
                                match pool_for_voice(&state).await {
                                    Some(pool) => {
                                        match crate::voice::spoken_verdict::stage_spoken_verdict(
                                            &pool,
                                            &decision_id,
                                            verdict,
                                        )
                                        .await
                                        {
                                            Ok(spoken) => {
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
                                            Err(msg) => {
                                                tracing::warn!(
                                                    target: "permagentd::voice",
                                                    "staging a spoken verdict failed: {msg}"
                                                );
                                            }
                                        }
                                    }
                                    None => {
                                        tracing::warn!(
                                            target: "permagentd::voice",
                                            "no pool available to stage a spoken verdict"
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
                            telemetry.log_outcome("reply_start_disconnected");
                            return;
                        }

                        let mut reply_ctx = VoiceReplyCtx {
                            state: &state,
                            transcript: &transcript,
                            telemetry,
                            turn_id,
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
                        let stream_result =
                            stream_reply_with_tts(&mut reply_ctx, &mut socket).await;

                        if let Err(e) = stream_result {
                            reply_ctx.telemetry.log_outcome("reply_error");
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
                    Ok(ClientMessage::EnrollStart) => {
                        audio_buffer.clear();
                        recording = false;
                        speaker_gate = None;
                        if state.speaker_verifier.read().await.is_none() {
                            if let Err(e) = start_speaker_model_download(&state).await {
                                tracing::warn!(
                                    target: "permagentd::voice",
                                    "speaker model auto-download failed to start: {e}"
                                );
                            }
                            if state.speaker_verifier.read().await.is_none() {
                                enroll = None;
                                let _ = socket
                                    .send(send_json(&ServerMessage::EnrollRetry {
                                        reason: "Preparing voice identity. This setup will continue when the model is ready."
                                            .into(),
                                    }))
                                    .await;
                                let _ =
                                    socket.send(send_json(&voice_print_msg(&state).await)).await;
                                continue;
                            }
                        }
                        enroll = Some(Vec::new());
                        tracing::info!(
                            target: "permagentd::voice",
                            "speaker_print enroll start"
                        );
                        let _ = socket.send(send_json(&enroll_status_msg(0))).await;
                    }
                    Ok(ClientMessage::EnrollDone) => match enroll.take() {
                        Some(collected)
                            if collected.len() >= crate::voice::speaker_print::NEED_UTTERANCES =>
                        {
                            match crate::voice::speaker_print::from_utterances(&collected) {
                                Some(print) => match crate::voice::speaker_print::save(&print) {
                                    Ok(()) => {
                                        tracing::info!(
                                            target: "permagentd::voice",
                                            "speaker_print enrolled n={}",
                                            print.n_utterances
                                        );
                                        let _ =
                                            socket.send(send_json(&ServerMessage::Enrolled)).await;
                                        let _ = socket
                                            .send(send_json(&voice_print_msg(&state).await))
                                            .await;
                                    }
                                    Err(e) => {
                                        enroll = Some(collected);
                                        let _ = socket
                                            .send(send_json(&ServerMessage::EnrollRetry {
                                                reason: format!(
                                                    "Couldn't save the voice print: {e}"
                                                ),
                                            }))
                                            .await;
                                    }
                                },
                                None => {
                                    enroll = Some(collected);
                                    let _ = socket
                                        .send(send_json(&ServerMessage::EnrollRetry {
                                            reason: "Those three takes didn't line up — try again."
                                                .into(),
                                        }))
                                        .await;
                                }
                            }
                        }
                        Some(collected) => {
                            let have = collected.len();
                            enroll = Some(collected);
                            let _ = socket
                                .send(send_json(&ServerMessage::EnrollRetry {
                                    reason: format!(
                                        "Say all {} sentences first ({have} so far).",
                                        crate::voice::speaker_print::NEED_UTTERANCES
                                    ),
                                }))
                                .await;
                            let _ = socket.send(send_json(&enroll_status_msg(have))).await;
                        }
                        None => {
                            let _ = socket
                                .send(send_json(&ServerMessage::EnrollRetry {
                                    reason: "Start enrollment first.".into(),
                                }))
                                .await;
                        }
                    },
                    Ok(ClientMessage::EnrollSkip) => {
                        enroll = None;
                        recording = false;
                        audio_buffer.clear();
                        tracing::info!(
                            target: "permagentd::voice",
                            "speaker_print enroll skip"
                        );
                        let _ = socket.send(send_json(&voice_print_msg(&state).await)).await;
                    }
                    Ok(ClientMessage::EnrollClear) => {
                        enroll = None;
                        recording = false;
                        audio_buffer.clear();
                        if let Err(e) = crate::voice::speaker_print::clear() {
                            tracing::warn!(
                                target: "permagentd::voice",
                                "speaker_print clear failed: {e}"
                            );
                        }
                        tracing::info!(
                            target: "permagentd::voice",
                            "speaker_print enroll cleared"
                        );
                        let _ = socket.send(send_json(&ServerMessage::EnrollCleared)).await;
                        let _ = socket.send(send_json(&voice_print_msg(&state).await)).await;
                    }
                    Err(e) => {
                        tracing::warn!(target: "permagentd::voice", "Invalid voice message: {}", e);
                    }
                }
            }
            Message::Binary(data) if recording => {
                // Enforce the capture bound on the server, before allocating
                // decoded PCM. A missing client stop must not grow memory or
                // leave an arbitrarily expensive offline STT job behind.
                if capture_exceeds_limit(audio_buffer.len(), data.len(), client_sample_rate) {
                    if let Some(mut telemetry) = active_telemetry.take() {
                        telemetry.set_stop_reason("capture_limit");
                        telemetry.record_capture_health(std::mem::take(&mut capture_health));
                        telemetry.log_outcome("capture_rejected_limit");
                    }
                    recording = false;
                    cancel_streaming_stt_worker(
                        &mut streaming_command_tx,
                        &mut streaming_event_rx,
                        &mut streaming_gate,
                        &mut pending_stream_partial,
                    );
                    audio_buffer.clear();
                    let _ = socket
                        .send(send_json(&ServerMessage::Error {
                            message: "Voice capture limit reached. Please start a shorter turn."
                                .into(),
                        }))
                        .await;
                    let _ = socket.send(send_json(&ServerMessage::Idle)).await;
                    continue;
                }
                let chunk = capture_health.observe_frame(&data);
                // A non-finite sample or partial f32 cannot be trusted by the
                // speaker/STT models. Reject it at the protocol boundary rather
                // than letting an invalid value enter an inference buffer.
                if capture_health.is_malformed() {
                    if let Some(mut telemetry) = active_telemetry.take() {
                        telemetry.set_stop_reason("malformed_capture_frame");
                        telemetry.set_empty_reason("malformed_pcm");
                        telemetry.record_capture_health(std::mem::take(&mut capture_health));
                        telemetry.log_outcome("capture_rejected_malformed");
                    }
                    recording = false;
                    cancel_streaming_stt_worker(
                        &mut streaming_command_tx,
                        &mut streaming_event_rx,
                        &mut streaming_gate,
                        &mut pending_stream_partial,
                    );
                    audio_buffer.clear();
                    send_terminal_idle(
                        &mut socket,
                        VoiceTurnOutcome::CaptureRejectedMalformed,
                        VoiceTurnOutcomeReason::MalformedPcm,
                    )
                    .await;
                    continue;
                }
                if !chunk.is_empty() {
                    let received_at = std::time::Instant::now();
                    if capture_first_received_at.is_none() {
                        capture_first_received_at = Some(received_at);
                        if let Some(turn_id) = recording_turn_id {
                            tracing::info!(
                                target: "permagentd::voice",
                                event = "voice_latency_stage",
                                stage = "capture_received",
                                turn_id,
                                socket_epoch,
                                session_id = session_id.as_deref().unwrap_or("voice-anon"),
                                stage_elapsed_ms = recording_started_at
                                    .map(|at| received_at.duration_since(at).as_millis())
                                    .unwrap_or(0),
                                audio_bytes = data.len().min(1_048_576),
                                "first voice audio frame received"
                            );
                        }
                    }
                    capture_last_received_at = Some(received_at);
                }
                audio_buffer.extend_from_slice(&chunk);
                if !streaming_failed {
                    if let Some(command_tx) = streaming_command_tx.as_ref() {
                        match command_tx.try_send(StreamingSttCommand::Audio(chunk.clone())) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                                streaming_failed = true;
                                if let Some(gate) = streaming_gate.as_mut() {
                                    gate.cancel();
                                }
                                let _ = command_tx.try_send(StreamingSttCommand::Cancel);
                            }
                        }
                    }
                }
                // Do not let a wrong speaker hold the client open until the
                // 60-second cap. Once two seconds are available, run CAM++ and
                // terminate the capture immediately on rejection. Enrollment
                // clips are intentionally exempt from their own identity gate.
                if early_speaker_gate_ready(
                    enroll.is_some(),
                    speaker_gate.is_some(),
                    crate::voice::speaker_print::load().is_some(),
                    audio_buffer.len(),
                    client_sample_rate,
                ) {
                    let gate_started = std::time::Instant::now();
                    let gate =
                        learned_speaker_gate(&state, &audio_buffer, client_sample_rate).await;
                    let gate_ms = gate_started.elapsed().as_millis();
                    match gate {
                        crate::voice::speaker_print::Gate::Admit { score } => {
                            tracing::info!(
                                target: "permagentd::voice",
                                "speaker_print early_admit score={score:.3} gate_ms={gate_ms}"
                            );
                            speaker_gate = Some(gate);
                            if flush_pending_stream_partial(
                                &mut pending_stream_partial,
                                &mut socket,
                            )
                            .await
                            .is_err()
                            {
                                socket_close_reason = "stream_partial_send_disconnected";
                                break 'voice_loop;
                            }
                        }
                        crate::voice::speaker_print::Gate::Reject { score } => {
                            tracing::info!(
                                target: "permagentd::voice",
                                "speaker_print early_reject score={score:.3} gate_ms={gate_ms}"
                            );
                            if let Some(mut telemetry) = active_telemetry.take() {
                                telemetry.set_stop_reason("speaker_gate_early");
                                telemetry
                                    .record_capture_health(std::mem::take(&mut capture_health));
                                telemetry.log_outcome("speaker_rejected_early");
                            }
                            recording = false;
                            cancel_streaming_stt_worker(
                                &mut streaming_command_tx,
                                &mut streaming_event_rx,
                                &mut streaming_gate,
                                &mut pending_stream_partial,
                            );
                            audio_buffer.clear();
                            let _ = socket
                                .send(send_json(&ServerMessage::SpeakerRejected))
                                .await;
                        }
                        crate::voice::speaker_print::Gate::Unavailable => {
                            tracing::warn!(
                                target: "permagentd::voice",
                                "speaker_print early_unavailable gate_ms={gate_ms}; failing closed"
                            );
                            if let Some(mut telemetry) = active_telemetry.take() {
                                telemetry.set_stop_reason("speaker_gate_early");
                                telemetry
                                    .record_capture_health(std::mem::take(&mut capture_health));
                                telemetry.log_outcome("speaker_gate_unavailable");
                            }
                            recording = false;
                            cancel_streaming_stt_worker(
                                &mut streaming_command_tx,
                                &mut streaming_event_rx,
                                &mut streaming_gate,
                                &mut pending_stream_partial,
                            );
                            audio_buffer.clear();
                            let _ = socket
                                .send(send_json(&ServerMessage::SpeakerRejected))
                                .await;
                        }
                        crate::voice::speaker_print::Gate::Open => {
                            speaker_gate = Some(gate);
                            if flush_pending_stream_partial(
                                &mut pending_stream_partial,
                                &mut socket,
                            )
                            .await
                            .is_err()
                            {
                                socket_close_reason = "stream_partial_send_disconnected";
                                break 'voice_loop;
                            }
                        }
                    }
                }
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
                socket_close_reason = if frame.is_some() {
                    "client_close_frame"
                } else {
                    "client_close"
                };
                tracing::info!(
                    target: "permagentd::voice",
                    socket_epoch,
                    "Client sent Close frame"
                );
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
    cancel_streaming_stt_worker(
        &mut streaming_command_tx,
        &mut streaming_event_rx,
        &mut streaming_gate,
        &mut pending_stream_partial,
    );
    if let Some(mut telemetry) = active_telemetry.take() {
        telemetry.set_stop_reason(socket_close_reason);
        telemetry.record_capture_health(capture_health);
        telemetry.log_outcome("capture_disconnected");
    }
    // Signal cancellation so any in-flight TTS spawn_blocking tasks skip the mutex
    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!(
        target: "permagentd::voice",
        event = "voice_socket",
        stage = "closed",
        socket_epoch,
        session_id = session_id.as_deref().unwrap_or("voice-anon"),
        close_reason = socket_close_reason,
        "Voice WebSocket handler exiting (cancelled=true)"
    );
}

/// Emergency cap on how many sentences are spoken in one turn.
///
/// Voice replies are prompted to be concise and the listener can barge in.
/// Cutting an otherwise complete answer at eight sentences made the runtime
/// announce "there's more" before it had delivered its conclusion. A very
/// high guard still bounds a pathological model loop without shaping ordinary
/// conversation; its cue remains origin-aware.
const DEFAULT_MAX_SPOKEN_SENTENCES: u32 = 100;

/// Config key (`~/.permagent/config.yaml`) overriding the spoken-length budget.
/// Clamped to [1, 100]; 0 would mute replies entirely, which is never intended.
const MAX_SPOKEN_SENTENCES_KEY: &str = "voice_max_spoken_sentences";

static NEXT_VOICE_TURN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static NEXT_VOICE_SOCKET_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_voice_turn_id() -> u64 {
    NEXT_VOICE_TURN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_voice_socket_epoch() -> u64 {
    NEXT_VOICE_SOCKET_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

const VOICE_STT_SLOW_MS: u128 = 2_000;
const VOICE_TTFT_SLOW_MS: u128 = 5_000;
const VOICE_FIRST_AUDIO_ELEVATED_MS: u128 = 3_000;
const VOICE_FIRST_AUDIO_STALL_MS: u128 = 8_000;
const VOICE_TURN_CRITICAL_MS: u128 = 20_000;

/// Coarse, actionable buckets for latency dashboards. These thresholds are
/// intentionally conservative: the known ~20s incident should be impossible
/// to hide inside an "average" bucket, while normal network variance remains
/// distinguishable from a real STT/LLM stall.
fn classify_voice_latency(
    stt_ms: Option<u128>,
    ttft_ms: Option<u128>,
    first_audio_ms: Option<u128>,
    total_ms: u128,
) -> &'static str {
    if total_ms >= VOICE_TURN_CRITICAL_MS
        || first_audio_ms.is_some_and(|ms| ms >= VOICE_FIRST_AUDIO_STALL_MS)
    {
        "critical_stall"
    } else if stt_ms.is_some_and(|ms| ms >= VOICE_STT_SLOW_MS) {
        "slow_stt"
    } else if ttft_ms.is_some_and(|ms| ms >= VOICE_TTFT_SLOW_MS) {
        "slow_llm_ttft"
    } else if first_audio_ms.is_some_and(|ms| ms >= VOICE_FIRST_AUDIO_ELEVATED_MS) {
        "elevated_first_audio"
    } else {
        "healthy"
    }
}

/// `f32` PCM is valid only when every sample is finite. This deliberately
/// conservative threshold is -60 dBFS: it is a diagnostic label, not a VAD
/// decision, so quiet speech still reaches STT and we do not silently tune the
/// user's microphone from the server.
const PCM_NEAR_SILENCE_RMS: f64 = 0.001;

/// Bounded, privacy-safe properties of one capture. Audio is never retained or
/// logged here: only counters and aggregate amplitudes are used to distinguish
/// a quiet microphone from malformed transport bytes or invalid float values.
#[derive(Debug, Clone, Default)]
struct CaptureHealth {
    frame_count: u64,
    payload_bytes: u64,
    trailing_bytes: u64,
    decoded_samples: u64,
    finite_samples: u64,
    nonfinite_samples: u64,
    zero_samples: u64,
    sum_squares: f64,
    peak_abs: f32,
}

impl CaptureHealth {
    fn observe_frame(&mut self, data: &[u8]) -> Vec<f32> {
        self.frame_count = self.frame_count.saturating_add(1);
        self.payload_bytes = self.payload_bytes.saturating_add(data.len() as u64);
        self.trailing_bytes = self
            .trailing_bytes
            .saturating_add((data.len() % std::mem::size_of::<f32>()) as u64);

        let mut samples = Vec::with_capacity(data.len() / std::mem::size_of::<f32>());
        for bytes in data.chunks_exact(std::mem::size_of::<f32>()) {
            let sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            self.decoded_samples = self.decoded_samples.saturating_add(1);
            if sample.is_finite() {
                self.finite_samples = self.finite_samples.saturating_add(1);
                if sample == 0.0 {
                    self.zero_samples = self.zero_samples.saturating_add(1);
                }
                let absolute = sample.abs();
                self.peak_abs = self.peak_abs.max(absolute);
                self.sum_squares += f64::from(sample) * f64::from(sample);
            } else {
                self.nonfinite_samples = self.nonfinite_samples.saturating_add(1);
            }
            samples.push(sample);
        }
        samples
    }

    fn rms(&self) -> Option<f64> {
        (self.finite_samples > 0).then(|| (self.sum_squares / self.finite_samples as f64).sqrt())
    }

    fn rms_millionths(&self) -> Option<u32> {
        self.rms().map(|value| {
            (value.clamp(0.0, 1.0) * 1_000_000.0)
                .round()
                .min(u32::MAX as f64) as u32
        })
    }

    fn peak_millionths(&self) -> Option<u32> {
        (self.finite_samples > 0).then(|| {
            (f64::from(self.peak_abs).clamp(0.0, 1.0) * 1_000_000.0)
                .round()
                .min(u32::MAX as f64) as u32
        })
    }

    fn label(&self) -> &'static str {
        if self.frame_count == 0 {
            "no_frames"
        } else if self.decoded_samples == 0 || self.trailing_bytes > 0 || self.nonfinite_samples > 0
        {
            "malformed_pcm"
        } else if self.zero_samples == self.finite_samples {
            "zero_pcm"
        } else if self
            .rms()
            .is_some_and(|value| value <= PCM_NEAR_SILENCE_RMS)
        {
            "near_silent_pcm"
        } else {
            "finite_signal_pcm"
        }
    }

    fn is_malformed(&self) -> bool {
        self.label() == "malformed_pcm"
    }
}

fn empty_stt_reason(health: &CaptureHealth) -> &'static str {
    match health.label() {
        "zero_pcm" => "zero_pcm",
        "near_silent_pcm" => "near_silent_pcm",
        "no_frames" => "no_frames",
        "malformed_pcm" => "malformed_pcm",
        _ => "finite_signal_no_words",
    }
}

fn empty_stt_wire_reason(health: &CaptureHealth) -> VoiceTurnOutcomeReason {
    match health.label() {
        "zero_pcm" => VoiceTurnOutcomeReason::ZeroPcm,
        "near_silent_pcm" => VoiceTurnOutcomeReason::NearSilentPcm,
        // `no_frames` cannot reach STT in the current route; retain a closed
        // short-capture reason if a future caller routes it through this seam.
        "no_frames" => VoiceTurnOutcomeReason::ShortCapture,
        "malformed_pcm" => VoiceTurnOutcomeReason::MalformedPcm,
        _ => VoiceTurnOutcomeReason::FiniteSignalNoWords,
    }
}

/// Server-side timing state for one spoken turn. `Instant` is monotonic and
/// never serialized; only bounded elapsed milliseconds are emitted. The
/// session ID joins events without exposing user audio or transcript content.
struct VoiceTurnTelemetry {
    turn_id: u64,
    socket_epoch: u64,
    session_id: Option<String>,
    turn_started_at: std::time::Instant,
    stt_ms: Option<u128>,
    ttft_ms: Option<u128>,
    /// First audio latency from Stop/speech end (the user-visible metric).
    first_audio_ms: Option<u128>,
    tts_total_ms: u128,
    tts_enqueued: u32,
    audio_segments_sent: u32,
    playback_estimate_ms: u64,
    capture_health: Option<CaptureHealth>,
    stop_reason: Option<&'static str>,
    empty_reason: Option<&'static str>,
    outcome_logged: bool,
}

impl VoiceTurnTelemetry {
    fn new(
        turn_id: u64,
        socket_epoch: u64,
        session_id: Option<&str>,
        turn_started_at: std::time::Instant,
    ) -> Self {
        Self {
            turn_id,
            socket_epoch,
            session_id: session_id.map(str::to_string),
            turn_started_at,
            stt_ms: None,
            ttft_ms: None,
            first_audio_ms: None,
            tts_total_ms: 0,
            tts_enqueued: 0,
            audio_segments_sent: 0,
            playback_estimate_ms: 0,
            capture_health: None,
            stop_reason: None,
            empty_reason: None,
            outcome_logged: false,
        }
    }

    fn elapsed_ms(&self) -> u128 {
        self.turn_started_at.elapsed().as_millis()
    }

    fn log_stage(&self, stage: &'static str, stage_started_at: std::time::Instant) {
        tracing::info!(
            target: "permagentd::voice",
            event = "voice_latency_stage",
            turn_id = self.turn_id,
            socket_epoch = self.socket_epoch,
            session_id = self.session_id.as_deref().unwrap_or("voice-anon"),
            stage,
            stage_elapsed_ms = self.elapsed_ms(),
            stage_duration_ms = stage_started_at.elapsed().as_millis(),
            "voice latency stage"
        );
    }

    fn record_capture_health(&mut self, health: CaptureHealth) {
        self.capture_health = Some(health);
        let health = self.capture_health.as_ref().expect("set above");
        tracing::info!(
            target: "permagentd::voice",
            event = "voice_capture_health",
            turn_id = self.turn_id,
            socket_epoch = self.socket_epoch,
            session_id = self.session_id.as_deref().unwrap_or("voice-anon"),
            capture_health = health.label(),
            capture_frames = health.frame_count,
            capture_bytes = health.payload_bytes,
            capture_trailing_bytes = health.trailing_bytes,
            capture_decoded_samples = health.decoded_samples,
            capture_finite_samples = health.finite_samples,
            capture_nonfinite_samples = health.nonfinite_samples,
            capture_zero_samples = health.zero_samples,
            capture_rms_millionths = health.rms_millionths(),
            capture_peak_millionths = health.peak_millionths(),
            "voice capture health"
        );
    }

    fn set_stop_reason(&mut self, reason: &'static str) {
        self.stop_reason = Some(reason);
    }

    fn set_empty_reason(&mut self, reason: &'static str) {
        self.empty_reason = Some(reason);
    }

    fn log_outcome(&mut self, outcome: &'static str) {
        if self.outcome_logged {
            return;
        }
        self.outcome_logged = true;
        let total_ms = self.elapsed_ms();
        let classification =
            classify_voice_latency(self.stt_ms, self.ttft_ms, self.first_audio_ms, total_ms);
        tracing::info!(
            target: "permagentd::voice",
            event = "voice_latency_summary",
            turn_id = self.turn_id,
            socket_epoch = self.socket_epoch,
            session_id = self.session_id.as_deref().unwrap_or("voice-anon"),
            total_ms,
            stt_ms = self.stt_ms,
            llm_ttft_ms = self.ttft_ms,
            first_audio_ms = self.first_audio_ms,
            tts_total_ms = self.tts_total_ms,
            tts_enqueued = self.tts_enqueued,
            audio_segments_sent = self.audio_segments_sent,
            playback_estimate_ms = self.playback_estimate_ms,
            playback_observed = false,
            capture_health = self
                .capture_health
                .as_ref()
                .map(CaptureHealth::label)
                .unwrap_or("not_recorded"),
            stop_reason = self.stop_reason.unwrap_or("not_recorded"),
            empty_reason = self.empty_reason.unwrap_or("not_applicable"),
            outcome,
            classification,
            "voice latency summary"
        );
    }
}

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
    telemetry: VoiceTurnTelemetry,
    turn_id: u64,
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
    ctx: &mut VoiceReplyCtx<'_>,
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
        let sessions = match state.session_manager().list_session_summaries().await {
            Ok(sessions) => sessions,
            Err(error) => {
                ctx.telemetry.log_outcome("session_lookup_error");
                return Err(error.into());
            }
        };
        match sessions.first().map(|s| s.id.clone()) {
            Some(id) => id,
            None => {
                ctx.telemetry.log_outcome("session_missing");
                return Err(anyhow::anyhow!("No session available for voice"));
            }
        }
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
    let agent = match state.get_agent(sid.clone()).await {
        Ok(agent) => agent,
        Err(error) => {
            ctx.telemetry.log_outcome("agent_setup_error");
            return Err(anyhow::anyhow!("Failed to get agent: {}", error));
        }
    };

    // Voice turns may answer on a different model than chat — the whole point of
    // the knob. Unconfigured or unreachable falls through to the session model.
    let routing_started_at = std::time::Instant::now();
    let voice_route = apply_voice_model(&agent, &sid).await;
    ctx.telemetry.log_stage("routing", routing_started_at);
    match voice_route {
        Some(route) => tracing::info!(
            target: "permagentd::voice",
            event = "voice_route",
            turn_id = ctx.turn_id,
            session_id = ctx.session_id.unwrap_or("voice-anon"),
            route_configured = true,
            route_applied = true,
            provider = route.provider.as_str(),
            model = route.model.as_str(),
            "voice route applied"
        ),
        None => tracing::info!(
            target: "permagentd::voice",
            event = "voice_route",
            turn_id = ctx.turn_id,
            session_id = ctx.session_id.unwrap_or("voice-anon"),
            route_configured = false,
            route_applied = false,
            "session route retained"
        ),
    }

    agent
        .extend_system_prompt(
            "voice_reply_style".to_string(),
            VOICE_REPLY_STYLE.to_string(),
        )
        .await;
    agent
        .extend_system_prompt("voice_origin".to_string(), ctx.origin.prompt_block())
        .await;

    let setup_ms = t_setup.elapsed().as_millis();
    ctx.telemetry.log_stage("agent_setup", t_setup);

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
    ctx.telemetry.log_stage("context_recall", t_ctx);
    tracing::info!(
        target: "permagentd::voice",
        event = "voice_context",
        turn_id = ctx.turn_id,
        session_id = ctx.session_id.unwrap_or("voice-anon"),
        context_recall_ms = ctx_recall_ms,
        recall_hits = recall_trace.count,
        "voice context prepared"
    );

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
    ctx.telemetry.log_stage("provider_started", t_reply);
    let stream_result = agent.reply(user_msg, session_config, None).await;
    let reply_setup_ms = t_reply.elapsed().as_millis();
    ctx.telemetry.log_stage("llm_stream_setup", t_reply);
    let mut stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => {
            ctx.telemetry.log_outcome("llm_provider_error");
            return Err(error.into());
        }
    };
    tracing::info!(
        target: "permagentd::voice",
        turn_id = ctx.turn_id,
        session_id = ctx.session_id.unwrap_or("voice-anon"),
        setup_ms,
        context_recall_ms = ctx_recall_ms,
        reply_setup_ms,
        speech_end_to_stream_ms = pipeline_start.elapsed().as_millis(),
        "voice reply pipeline"
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
    let mut next_segment_id = 0u64;
    let mut first_token_logged = false;
    let mut spoken_stop = false;
    let stream_start = std::time::Instant::now();
    let mut queue: std::collections::VecDeque<(String, f32)> = std::collections::VecDeque::new();
    let mut inflight: Option<(
        tokio::task::JoinHandle<anyhow::Result<AudioOutput>>,
        std::time::Instant,
        String,
        u64,
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
                if let Some((handle, ..)) = inflight.take() {
                    handle.abort();
                }
                spoken_stop = true;
                break;
            }
            DrainOutcome::Disconnected => {
                if let Some((handle, ..)) = inflight.take() {
                    handle.abort();
                }
                ctx.telemetry.log_outcome("disconnected");
                return Ok(());
            }
        }
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some((handle, ..)) = inflight.take() {
                handle.abort();
            }
            ctx.telemetry.log_outcome("cancelled");
            return Ok(());
        }

        if inflight.is_none() {
            if let Some((speech, speed)) = queue.pop_front() {
                let segment_id = next_segment_id;
                next_segment_id = next_segment_id.saturating_add(1);
                ctx.telemetry.tts_enqueued = ctx.telemetry.tts_enqueued.saturating_add(1);
                let tts_enqueue_started_at = std::time::Instant::now();
                ctx.telemetry
                    .log_stage("tts_enqueue", tts_enqueue_started_at);
                // Never spell an unknown name. Stop this reply, ask, listen.
                if let Some(word) = first_unknown_name(tts.as_ref(), &speech) {
                    tracing::info!(
                        target: "permagentd::voice",
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
                        segment_id,
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
                        segment_id,
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
                let (_, started, preview, segment_id) = inflight.take().expect("guarded");
                let chunk_tts_ms = started.elapsed().as_millis();
                total_tts_ms += chunk_tts_ms;
                ctx.telemetry.tts_total_ms = ctx.telemetry.tts_total_ms.saturating_add(chunk_tts_ms);
                ctx.telemetry.log_stage("tts_synthesis", started);
                match drain_client_messages(socket, ctx) {
                    DrainOutcome::Continue => {}
                    DrainOutcome::SpokenStop => {
                        spoken_stop = true;
                        break;
                    }
                    DrainOutcome::Disconnected => {
                        ctx.telemetry.log_outcome("disconnected");
                        return Ok(());
                    }
                }
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    ctx.telemetry.log_outcome("cancelled");
                    return Ok(());
                }
                match result {
                    Ok(Ok(audio)) => {
                        let dur = audio.samples.len() as f32 / audio.sample_rate as f32;
                        let rtf = chunk_tts_ms as f32 / 1000.0 / dur.max(0.01);
                        tracing::info!(
                            target: "permagentd::voice",
                            turn_id = ctx.turn_id,
                            session_id = ctx.session_id.unwrap_or("voice-anon"),
                            sentence = sentence_num,
                            chars = preview.len(),
                            tts_ms = chunk_tts_ms,
                            audio_seconds = dur,
                            realtime_factor = rtf,
                            segment_id,
                            "voice audio segment synthesized"
                        );
                        let duration_ms = if audio.sample_rate == 0 {
                            0
                        } else {
                            (audio.samples.len() as u64).saturating_mul(1_000)
                                / u64::from(audio.sample_rate)
                        };
                        if !send_tts_segment(
                            socket,
                            &audio,
                            preview,
                            segment_id,
                            ctx.cancelled.as_ref(),
                        )
                        .await
                        {
                            tracing::warn!(target: "permagentd::voice", "Voice audio send stopped");
                            ctx.telemetry.log_outcome(if cancelled.load(
                                std::sync::atomic::Ordering::Relaxed,
                            ) {
                                "cancelled"
                            } else {
                                "audio_send_disconnected"
                            });
                            return Ok(());
                        }
                        if !first_audio_sent {
                            tracing::info!(
                                target: "permagentd::voice",
                                turn_id = ctx.turn_id,
                                socket_epoch = ctx.telemetry.socket_epoch,
                                session_id = ctx.session_id.unwrap_or("voice-anon"),
                                first_audio_ms = ctx.telemetry.elapsed_ms(),
                                speech_end_to_first_audio_ms = pipeline_start.elapsed().as_millis(),
                                "voice first audio queued"
                            );
                            // This is the first complete metadata+PCM pair the
                            // server queued. It remains distinct from a client
                            // playback receipt, which is not yet on the wire.
                            ctx.telemetry.first_audio_ms =
                                Some(pipeline_start.elapsed().as_millis());
                            first_audio_sent = true;
                        }
                        ctx.telemetry.playback_estimate_ms = ctx
                            .telemetry
                            .playback_estimate_ms
                            .saturating_add(duration_ms);
                        ctx.telemetry.audio_segments_sent = ctx
                            .telemetry
                            .audio_segments_sent
                            .saturating_add(1);
                    }
                    Ok(Err(e)) if e.to_string() == "cancelled" => {
                        tracing::info!(target: "permagentd::voice", "TTS cancelled (pre-mutex)");
                        ctx.telemetry.log_outcome("tts_cancelled");
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        ctx.telemetry.log_outcome("tts_error");
                        return Err(e);
                    }
                    Err(e) => {
                        ctx.telemetry.log_outcome("tts_task_panic");
                        return Err(anyhow::anyhow!("TTS task panicked: {}", e));
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
                                    let ttft_ms = stream_start.elapsed().as_millis();
                                    ctx.telemetry.ttft_ms = Some(ttft_ms);
                                    tracing::info!(
                                        target: "permagentd::voice",
                                        event = "voice_latency_stage",
                                        stage = "llm_ttft",
                                        turn_id = ctx.turn_id,
                                        session_id = ctx.session_id.unwrap_or("voice-anon"),
                                        stage_elapsed_ms = ctx.telemetry.elapsed_ms(),
                                        stage_duration_ms = ttft_ms,
                                        "voice first LLM token"
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
                    Some(Err(error)) => {
                        ctx.telemetry.log_outcome("llm_stream_error");
                        return Err(error.into());
                    }
                    Some(Ok(_)) => {
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
        if socket
            .send(send_json(&ServerMessage::Stopped))
            .await
            .is_err()
        {
            ctx.telemetry.log_outcome("stopped_send_disconnected");
            return Ok(());
        }
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
        if socket
            .send(send_json(&ServerMessage::Clipboard {
                text: clip.text.clone(),
            }))
            .await
            .is_err()
        {
            ctx.telemetry.log_outcome("clipboard_send_disconnected");
            return Ok(());
        }
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
    if socket
        .send(send_json(&ServerMessage::ReplyText { text: shown }))
        .await
        .is_err()
    {
        ctx.telemetry.log_outcome("reply_text_send_disconnected");
        return Ok(());
    }

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
        if socket
            .send(send_json(&ServerMessage::Navigate {
                tab: nav.tab,
                tool_type: nav.tool_type,
                panel_type: nav.panel_type,
                section: nav.section,
                state: nav.state,
                reason: nav.reason,
            }))
            .await
            .is_err()
        {
            ctx.telemetry.log_outcome("navigate_send_disconnected");
            return Ok(());
        }
    }

    let reply_ms = t_reply.elapsed().as_millis();
    if socket
        .send(send_json(&ServerMessage::ReplyEnd {
            sample_rate: tts.sample_rate(),
        }))
        .await
        .is_err()
    {
        ctx.telemetry.log_outcome("reply_end_send_disconnected");
        return Ok(());
    }

    let total_ms = pipeline_start.elapsed().as_millis();
    tracing::info!(
        target: "permagentd::voice",
        turn_id = ctx.turn_id,
        session_id = ctx.session_id.unwrap_or("voice-anon"),
        total_ms,
        stt_ms,
        reply_ms,
        tts_total_ms = total_tts_ms,
        sentences = sentence_num,
        "voice turn timing"
    );
    // `reply_sent` means all frames were accepted by this socket. It is not a
    // claim that iOS drained playback: the server has no playback receipt yet,
    // so the summary remains explicit about `playback_observed=false`.
    ctx.telemetry.log_outcome(if spoken_stop {
        "spoken_stop"
    } else {
        "reply_sent"
    });

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
            if let Err(error) = crate::brain_ops::persist_chat_turn(
                brain.clone(),
                pool,
                sid.to_string(),
                turn_idx,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or_default(),
                transcript.to_string(),
                full_reply,
                String::new(),
            )
            .await
            {
                tracing::warn!(target: "permagentd::brain", "chat memory enqueue failed: {error}");
            }
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

/// The Spectral pool, or `None` when it cannot be reached. The voice socket
/// only ever needs it to STAGE a verdict — it has no answer path.
async fn pool_for_voice(state: &Arc<AppState>) -> Option<sqlx::Pool<sqlx::Sqlite>> {
    state.session_manager().pool_clone().await.ok()
}

async fn pick_spoken_decision(state: &Arc<AppState>, session_id: Option<&str>) -> Option<String> {
    let pool = pool_for_voice(state).await?;
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
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
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
    let metadata_text = speech.clone();
    match spawn_synth(tts.clone(), speech, voice_id, plan.speed, cancelled.clone()).await {
        Ok(Ok(audio)) => {
            if !send_tts_segment(socket, &audio, metadata_text, 0, cancelled.as_ref()).await {
                return;
            }
        }
        Ok(Err(_)) => tracing::warn!(target: "permagentd::voice", "canned TTS failed"),
        Err(_) => tracing::warn!(target: "permagentd::voice", "canned TTS task panicked"),
    }
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return;
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
            "skipping unspeakable reply fragment"
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
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        permagent::events::voice_remainder::stash(session_id, leftover.to_string());
        return;
    }
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
    let mut next_segment_id = 0u64;
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
        let metadata_text = speech.clone();
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
                let segment_id = next_segment_id;
                next_segment_id = next_segment_id.saturating_add(1);
                if !send_tts_segment(
                    socket,
                    &audio,
                    metadata_text,
                    segment_id,
                    cancelled.as_ref(),
                )
                .await
                {
                    permagent::events::voice_remainder::stash(session_id, leftover.to_string());
                    return;
                }
            }
            Ok(Err(_)) => tracing::warn!(target: "permagentd::voice", "remainder TTS failed"),
            Err(_) => tracing::warn!(target: "permagentd::voice", "remainder TTS task panicked"),
        }
    }

    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        permagent::events::voice_remainder::stash(session_id, leftover.to_string());
        return;
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
///
/// Public so the voice-model bench (`src/bin/voice_model_bench.rs`) can time
/// candidate models against the SAME boundary rule first audio actually fires
/// on, rather than a re-implementation that would drift from it.
pub fn find_speakable_boundary(text: &str, first_chunk: bool) -> Option<(usize, usize)> {
    let sentence_min = if first_chunk { 3 } else { 5 };
    let clause_min = if first_chunk { 12 } else { 25 };

    // First pass: sentence boundary (strongest break, lowest minimum)
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if (ch == '.' || ch == '!' || ch == '?') && i >= sentence_min {
            let after = iter.peek().map(|(_, c)| *c);
            if (after.is_none() || after == Some(' ') || after == Some('\n'))
                && !(ch == '.' && period_is_mid_sentence_initialism(text, i))
            {
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

/// `U.S. tariffs` and similar initialisms are not sentence endings. Streaming
/// at the second period produced a standalone "The U.S." TTS buffer in the
/// live 2026-08-28 session, making the following predicate sound detached.
/// Only suppress the boundary when the next non-space character is lowercase;
/// `I moved to the U.S. Canada came next.` still ends where it should.
fn period_is_mid_sentence_initialism(text: &str, period_at: usize) -> bool {
    let bytes = text.as_bytes();
    if period_at < 3
        || bytes.get(period_at) != Some(&b'.')
        || !bytes[period_at - 1].is_ascii_uppercase()
        || bytes[period_at - 2] != b'.'
        || !bytes[period_at - 3].is_ascii_uppercase()
    {
        return false;
    }
    text.get(period_at + 1..)
        .and_then(|tail| tail.chars().find(|ch| !ch.is_whitespace()))
        .is_some_and(|ch| ch.is_lowercase())
}

/// Match the client's maximum turn duration with an independent absolute PCM
/// memory ceiling. The byte check also rejects trailing overflow before decode.
fn capture_exceeds_limit(samples: usize, incoming_bytes: usize, sample_rate: u32) -> bool {
    let allowed = (u64::from(sample_rate) * 60).min(4_000_000) as usize;
    samples > allowed || incoming_bytes > allowed.saturating_sub(samples).saturating_mul(4)
}

#[cfg(test)]
mod tests {
    struct FakeStream {
        cancelled: bool,
        generation: u64,
    }

    impl crate::voice::provider::StreamingSttSession for FakeStream {
        fn push_audio(
            &mut self,
            _samples: &[f32],
        ) -> anyhow::Result<Vec<crate::voice::provider::StreamingSttEvent>> {
            if self.cancelled {
                return Ok(Vec::new());
            }
            Ok(vec![crate::voice::provider::StreamingSttEvent::partial(
                self.generation,
                "partial",
            )])
        }

        fn finish(&mut self) -> anyhow::Result<Vec<crate::voice::provider::StreamingSttEvent>> {
            if self.cancelled {
                return Ok(Vec::new());
            }
            Ok(vec![crate::voice::provider::StreamingSttEvent::final_text(
                self.generation,
                "final",
            )])
        }

        fn cancel(&mut self) {
            self.cancelled = true;
        }
    }

    #[tokio::test]
    async fn streaming_worker_preserves_partial_then_single_final_order() {
        let (command_tx, mut event_rx, _worker) =
            super::spawn_streaming_stt_worker(Box::new(FakeStream {
                cancelled: false,
                generation: 3,
            }));
        command_tx
            .try_send(super::StreamingSttCommand::Audio(vec![0.1]))
            .unwrap();
        let partial = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            partial,
            crate::voice::provider::StreamingSttEvent::partial(3, "partial")
        );

        command_tx
            .try_send(super::StreamingSttCommand::Finish)
            .unwrap();
        let final_event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            final_event,
            crate::voice::provider::StreamingSttEvent::final_text(3, "final")
        );
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), event_rx.recv()).await,
            Ok(None) | Err(_)
        ));
    }

    #[tokio::test]
    async fn cancelling_stream_worker_drops_late_provider_output() {
        let (command_tx, mut event_rx, _worker) =
            super::spawn_streaming_stt_worker(Box::new(FakeStream {
                cancelled: false,
                generation: 8,
            }));
        command_tx
            .try_send(super::StreamingSttCommand::Cancel)
            .unwrap();
        drop(command_tx);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), event_rx.recv())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_live_stream_worker_blocks_a_second_start_without_spawning_another() {
        let worker = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        assert!(!super::streaming_worker_is_available(Some(&worker)));
        assert!(!super::native_stt_workers_are_available(
            Some(&worker),
            None
        ));
        worker.abort();
    }

    #[test]
    fn stt_wait_controls_do_not_abort_a_live_provider() {
        assert!(matches!(
            super::classify_stt_wait_message(Message::Pong(Vec::new().into())),
            super::SttWaitDisposition::KeepWaiting
        ));
        assert!(matches!(
            super::classify_stt_wait_message(Message::Binary(Vec::new().into())),
            super::SttWaitDisposition::KeepWaiting
        ));
        assert!(matches!(
            super::classify_stt_wait_message(Message::Text(r#"{"type":"stop"}"#.into())),
            super::SttWaitDisposition::KeepWaiting
        ));
        assert!(matches!(
            super::classify_stt_wait_message(Message::Text(
                r#"{"type":"start","sample_rate":16000}"#.into()
            )),
            super::SttWaitDisposition::Deferred(Message::Text(_))
        ));
        assert!(matches!(
            super::classify_stt_wait_message(Message::Close(None)),
            super::SttWaitDisposition::Disconnected
        ));
    }

    #[tokio::test]
    async fn a_blocked_batch_worker_is_retained_across_start_storm() {
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let worker: super::BatchSttTask = tokio::task::spawn_blocking(move || {
            release_rx
                .recv()
                .expect("the fake native decode is released by the test");
            Ok::<String, anyhow::Error>(String::new())
        });
        let mut retained = Some(worker);
        for _ in 0..16 {
            assert!(!super::batch_worker_is_available(retained.as_ref()));
            assert!(!super::native_stt_workers_are_available(
                None,
                retained.as_ref()
            ));
        }
        let worker = retained.take().expect("the single worker is retained");
        release_tx.send(()).expect("release the fake native decode");
        assert!(worker.await.unwrap().is_ok());
    }

    #[test]
    fn busy_stt_terminal_contract_always_returns_client_to_idle() {
        let frames = super::terminal_idle_frames(
            super::VoiceTurnOutcome::SttBusy,
            super::VoiceTurnOutcomeReason::SttBusy,
        );
        assert!(matches!(
            &frames[0],
            &super::ServerMessage::TurnOutcome {
                outcome: super::VoiceTurnOutcome::SttBusy,
                reason: super::VoiceTurnOutcomeReason::SttBusy,
            }
        ));
        assert!(matches!(&frames[1], &super::ServerMessage::Idle));
        let wire = serde_json::to_value(&frames[0]).unwrap();
        assert_eq!(wire["type"], "turn_outcome");
        assert_eq!(wire["outcome"], "stt_busy");
        assert_eq!(wire["reason"], "stt_busy");
    }

    #[test]
    fn streaming_partial_uses_existing_transcript_wire_shape() {
        let wire = serde_json::to_value(super::ServerMessage::TranscriptPartial {
            text: "hello".into(),
        })
        .unwrap();
        assert_eq!(wire["type"], "transcript_partial");
        assert_eq!(wire["text"], "hello");
    }

    #[test]
    fn partials_stay_private_until_speaker_gate_admits_or_opens() {
        use crate::voice::speaker_print::Gate;

        assert!(!super::speaker_gate_allows_stream_text(None));
        assert!(super::speaker_gate_allows_stream_text(Some(&Gate::Open)));
        assert!(super::speaker_gate_allows_stream_text(Some(&Gate::Admit {
            score: 0.9,
        })));
        assert!(!super::speaker_gate_allows_stream_text(Some(
            &Gate::Reject { score: 0.1 }
        )));
        assert!(!super::speaker_gate_allows_stream_text(Some(
            &Gate::Unavailable
        )));
    }

    #[test]
    fn capture_limit_bounds_duration_memory_and_overflow_before_decode() {
        assert!(!super::capture_exceeds_limit(959_999, 4, 16_000));
        assert!(super::capture_exceeds_limit(960_000, 4, 16_000));
        assert!(super::capture_exceeds_limit(0, 3_840_001, 16_000));
        assert!(!super::capture_exceeds_limit(2_879_999, 4, 48_000));
        assert!(super::capture_exceeds_limit(4_000_000, 4, u32::MAX));
        assert!(super::capture_exceeds_limit(
            usize::MAX,
            usize::MAX,
            u32::MAX
        ));
        assert!(super::capture_exceeds_limit(0, 4, 0));
    }
    use super::*;
    use permagent::download_manager::DownloadManager;

    // ── The voice model knob ─────────────────────────────────────────────────

    /// The measured default routes voice away from the session model. If this
    /// ever quietly reverts, the 7.4 s TTFT comes back with it.
    #[test]
    fn the_voice_path_defaults_to_the_benched_voice_model() {
        let (route, source) = permagent::config::voice_model::resolve_voice_model(|_| None)
            .expect("an unconfigured daemon still routes voice to the measured default");
        assert_eq!(route, permagent::config::default_voice_model());
        assert_eq!(source, permagent::config::VoiceModelSource::Default);
    }

    /// The fallback path: an invalid model id must NOT take the turn down. The
    /// user is mid-sentence; the session model still answers.
    #[test]
    fn an_invalid_voice_model_id_falls_back_rather_than_failing_the_turn() {
        assert!(
            voice_model_config(&permagent::config::VoiceModel {
                provider: "minimax".to_string(),
                model: String::new(),
            })
            .is_none(),
            "an empty model id must resolve to None so the caller keeps the session model"
        );
    }

    #[test]
    fn a_valid_voice_model_id_builds_a_model_config_for_that_model() {
        let route = permagent::config::default_voice_model();
        let config = voice_model_config(&route).expect("the default must build");
        assert_eq!(config.model_name, route.model);
        assert!(
            config.context_limit.is_some_and(|limit| limit > 0),
            "canonical limits must be applied so the voice turn does not overflow"
        );
    }

    /// Turning it off is the way back to one model for everything.
    #[test]
    fn session_turns_the_voice_model_off_entirely() {
        let read =
            |key: &str| (key == permagent::config::VOICE_MODEL_KEY).then(|| "session".to_string());
        assert!(permagent::config::voice_model::resolve_voice_model(read).is_none());
    }

    // ── The spoken-reply style contract ──────────────────────────────────────

    /// The bench's biggest perceived-latency finding, as a prompt rule.
    #[test]
    fn the_voice_style_forbids_opening_a_turn_in_silence() {
        assert!(VOICE_REPLY_STYLE.contains("NEVER OPEN A TURN IN SILENCE"));
        assert!(
            VOICE_REPLY_STYLE.contains("say one \n             short sentence")
                || VOICE_REPLY_STYLE.contains("say one short sentence"),
            "the rule must say WHAT to do, not only what not to do"
        );
    }

    /// Voice-only: the style contract is spoken-reply advice (no markdown,
    /// contractions, delivery tags) and would be wrong on the chat path. It must
    /// reach the agent through `extend_system_prompt` on the voice route and not
    /// be baked into the shared base prompt every session gets.
    #[test]
    fn the_voice_style_is_not_part_of_the_shared_base_prompt() {
        let base = include_str!(
            "../../../goose/src/agents/snapshots/permagent__agents__prompt_manager__tests__all_platform_extensions.snap"
        );
        assert!(
            !base.contains("NEVER OPEN A TURN IN SILENCE"),
            "the voice style leaked into the base system prompt — chat turns would get spoken-reply rules"
        );
        assert!(
            !base.contains("The user is speaking to you by voice"),
            "the voice style leaked into the base system prompt"
        );
    }

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
    fn initialism_period_does_not_split_a_sentence_mid_predicate() {
        let text = "The U.S. has imposed new tariffs.";
        let (end, _) = find_speakable_boundary(text, false).expect("final sentence boundary");
        assert_eq!(text.get(..=end), Some(text));

        let two = "I moved to the U.S. Canada came next.";
        let (end, _) = find_speakable_boundary(two, false).expect("first sentence boundary");
        assert_eq!(two.get(..=end), Some("I moved to the U.S."));
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

    #[test]
    fn reconnect_replays_a_parked_teach_word() {
        let sid = "teach-replay-test";
        permagent::events::voice_pronounce::clear(sid);
        permagent::events::voice_pronounce::begin(sid, "Elspeth", None);
        let msg = pending_teach_msg(sid).expect("parked teach");
        let v = serde_json::to_value(msg).unwrap();
        assert_eq!(v["type"], "teach");
        assert_eq!(v["word"], "Elspeth");
        permagent::events::voice_pronounce::clear(sid);
        assert!(pending_teach_msg(sid).is_none());
    }

    #[test]
    fn enroll_client_frames_deserialize() {
        for ty in ["enroll_start", "enroll_done", "enroll_skip", "enroll_clear"] {
            let raw = format!(r#"{{"type":"{ty}"}}"#);
            serde_json::from_str::<ClientMessage>(&raw).unwrap_or_else(|e| panic!("{ty}: {e}"));
        }
    }

    #[test]
    fn enroll_server_frames_are_not_teach() {
        let status = serde_json::to_value(enroll_status_msg(0)).unwrap();
        assert_eq!(status["type"], "enroll_status");
        assert_eq!(status["have"], 0);
        assert_eq!(status["need"], 3);
        assert_eq!(status["prompt"], crate::voice::speaker_print::PROMPTS[0]);
        let enrolled = serde_json::to_value(ServerMessage::Enrolled).unwrap();
        assert_eq!(enrolled["type"], "enrolled");
        assert!(enrolled.get("word").is_none());
        let print = serde_json::to_value(ServerMessage::VoicePrint {
            enrolled: false,
            available: true,
            downloading: false,
        })
        .unwrap();
        assert_eq!(print["type"], "voice_print");
        assert_eq!(print["enrolled"], serde_json::json!(false));
        assert_eq!(print["available"], serde_json::json!(true));
        assert_eq!(print["downloading"], serde_json::json!(false));
        let rejected = serde_json::to_value(ServerMessage::SpeakerRejected).unwrap();
        assert_eq!(rejected["type"], "speaker_rejected");
        assert!(rejected.get("message").is_none());
    }

    #[test]
    fn early_speaker_gate_waits_for_two_seconds_and_never_gates_enrollment() {
        assert!(!early_speaker_gate_ready(
            false, false, true, 31_999, 16_000
        ));
        assert!(early_speaker_gate_ready(false, false, true, 32_000, 16_000));
        assert!(!early_speaker_gate_ready(true, false, true, 64_000, 16_000));
        assert!(!early_speaker_gate_ready(false, true, true, 64_000, 16_000));
        assert!(!early_speaker_gate_ready(
            false, false, false, 64_000, 16_000
        ));
        assert!(!early_speaker_gate_ready(false, false, true, 64_000, 0));
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

    #[test]
    fn terminal_no_reply_frame_is_typed_private_and_precedes_legacy_idle() {
        let cases = [
            (
                VoiceTurnOutcome::CaptureRejectedMalformed,
                VoiceTurnOutcomeReason::MalformedPcm,
                "capture_rejected_malformed",
                "malformed_pcm",
            ),
            (
                VoiceTurnOutcome::CaptureRejectedShort,
                VoiceTurnOutcomeReason::ShortCapture,
                "capture_rejected_short",
                "short_capture",
            ),
            (
                VoiceTurnOutcome::EmptyStt,
                VoiceTurnOutcomeReason::NearSilentPcm,
                "empty_stt",
                "near_silent_pcm",
            ),
        ];

        for (outcome, reason, expected_outcome, expected_reason) in cases {
            let frames = terminal_idle_frames(outcome, reason);
            let values = frames
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                values,
                vec![
                    serde_json::json!({
                        "type": "turn_outcome",
                        "outcome": expected_outcome,
                        "reason": expected_reason,
                    }),
                    serde_json::json!({"type": "idle"}),
                ]
            );
            assert_eq!(values[0].as_object().unwrap().len(), 3);
        }
    }

    #[test]
    fn estimated_word_timings_are_ordered_and_cover_the_audio_duration() {
        let timings = estimate_word_timings("Hello, world!", 1_000);
        assert_eq!(timings.len(), 2);
        assert_eq!(timings[0].word, "Hello,");
        assert_eq!(timings[1].word, "world!");
        assert_eq!((timings[0].start_ms, timings[0].end_ms), (0, 500));
        assert_eq!((timings[1].start_ms, timings[1].end_ms), (500, 1_000));
        assert_eq!((timings[0].start_utf16, timings[0].end_utf16), (0, 6));
        assert_eq!((timings[1].start_utf16, timings[1].end_utf16), (7, 13));
        assert!(timings.windows(2).all(|pair| {
            pair[0].start_ms <= pair[0].end_ms
                && pair[0].end_ms <= pair[1].start_ms
                && pair[0].end_utf16 <= pair[1].start_utf16
        }));
    }

    #[test]
    fn estimated_word_timings_use_ios_utf16_ranges_for_non_bmp_text() {
        let timings = estimate_word_timings("Hi 👋 there", 900);
        assert_eq!(
            timings.iter().map(|t| t.word.as_str()).collect::<Vec<_>>(),
            ["Hi", "👋", "there"]
        );
        // The waving-hand emoji occupies two UTF-16 code units.
        assert_eq!((timings[1].start_utf16, timings[1].end_utf16), (3, 5));
        assert_eq!((timings[2].start_utf16, timings[2].end_utf16), (6, 11));
        assert_eq!(timings.last().unwrap().end_ms, 900);
    }

    #[test]
    fn audio_segment_metadata_is_additive_and_explicitly_estimated() {
        let audio = AudioOutput {
            samples: vec![0.0; 24_000],
            sample_rate: 24_000,
        };
        let value =
            serde_json::to_value(audio_segment_metadata(&audio, "A short reply.", 7)).unwrap();
        assert_eq!(value["type"], "audio_segment");
        assert_eq!(value["segment_id"], 7);
        assert_eq!(value["text"], "A short reply.");
        assert_eq!(value["sample_rate"], 24_000);
        assert_eq!(value["duration_ms"], 1_000);
        assert_eq!(value["timing_source"], "estimated_proportional");
        assert_eq!(value["word_timings"].as_array().unwrap().len(), 3);
        // Existing framing remains serializable and unchanged.
        assert_eq!(
            serde_json::to_value(ServerMessage::ReplyStart).unwrap()["type"],
            "reply_start"
        );
        assert_eq!(
            serde_json::to_value(ServerMessage::ReplyEnd {
                sample_rate: 24_000
            })
            .unwrap()["type"],
            "reply_end"
        );
    }

    #[test]
    fn latency_classification_surfaces_the_known_long_stall() {
        assert_eq!(
            classify_voice_latency(Some(120), Some(1_200), Some(19_900), 19_950),
            "critical_stall"
        );
        assert_eq!(
            classify_voice_latency(Some(2_100), Some(1_000), Some(2_500), 4_000),
            "slow_stt"
        );
        assert_eq!(
            classify_voice_latency(Some(100), Some(5_100), Some(5_200), 6_000),
            "slow_llm_ttft"
        );
        assert_eq!(
            classify_voice_latency(Some(100), Some(1_000), Some(3_100), 4_000),
            "elevated_first_audio"
        );
        assert_eq!(
            classify_voice_latency(Some(100), Some(1_000), Some(1_900), 2_500),
            "healthy"
        );
    }

    #[test]
    fn telemetry_emits_only_one_terminal_outcome() {
        let mut telemetry = VoiceTurnTelemetry::new(7, 3, None, std::time::Instant::now());
        assert!(!telemetry.outcome_logged);
        telemetry.log_outcome("empty_stt");
        assert!(telemetry.outcome_logged);
        // Caller-side error handling may run after an inner streaming path has
        // already classified the turn; the summary must remain single-shot.
        telemetry.log_outcome("reply_error");
        assert!(telemetry.outcome_logged);
    }

    fn pcm_bytes(samples: &[f32]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn capture_health_distinguishes_no_frames_silence_signal_and_malformed_pcm() {
        let no_frames = CaptureHealth::default();
        assert_eq!(no_frames.label(), "no_frames");

        let mut zero = CaptureHealth::default();
        zero.observe_frame(&pcm_bytes(&[0.0, -0.0, 0.0]));
        assert_eq!(zero.label(), "zero_pcm");
        assert_eq!(empty_stt_reason(&zero), "zero_pcm");

        let mut quiet = CaptureHealth::default();
        quiet.observe_frame(&pcm_bytes(&[0.0005, -0.0005, 0.0004]));
        assert_eq!(quiet.label(), "near_silent_pcm");
        assert_eq!(empty_stt_reason(&quiet), "near_silent_pcm");
        assert!(quiet.rms_millionths().is_some());

        let mut signal = CaptureHealth::default();
        signal.observe_frame(&pcm_bytes(&[0.15, -0.10, 0.05]));
        assert_eq!(signal.label(), "finite_signal_pcm");
        assert_eq!(empty_stt_reason(&signal), "finite_signal_no_words");

        let mut malformed = CaptureHealth::default();
        malformed.observe_frame(&pcm_bytes(&[f32::NAN, f32::INFINITY]));
        malformed.observe_frame(&[0xAB, 0xCD, 0xEF]);
        assert_eq!(malformed.label(), "malformed_pcm");
        assert!(malformed.is_malformed());
        assert_eq!(malformed.nonfinite_samples, 2);
        assert_eq!(malformed.trailing_bytes, 3);
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

    #[test]
    fn speaker_identity_url_and_digest_pass_download_policy() {
        permagent::download_manager::validate_download_url(crate::voice::speaker_print::MODEL_URL)
            .expect("CAM++ URL must be allowlisted");
        let digest = crate::voice::speaker_print::MODEL_SHA256;
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
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
