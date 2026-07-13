//! Dictation transcription route.
//!
//! `POST /api/dictation/transcribe` — multipart upload of a short audio clip
//! (the browser records mic PCM and encodes it as WAV so it decodes cleanly on
//! any build). The clip is transcribed **locally** via the on-device Whisper
//! model and the plain text is returned. This is the "give me back text"
//! primitive the conversational voice loop (`useVoice`) never exposed: it lets
//! surfaces like the project Notes composer offer a Dictate button without
//! standing up a bespoke audio pipeline.
//!
//! Transcription is local-only (the `local-inference` feature, on by default).
//! When that feature is absent, or no local Whisper model is configured, the
//! route answers `503` with a clear message the UI can surface — never a bare
//! `500` that reads as "dictation is broken".

use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 25 MB — a generous ceiling for a dictated clip (minutes of 16-bit WAV),
/// well under anything that would stall local transcription.
const MAX_AUDIO_SIZE: usize = 25 * 1024 * 1024;

/// The model bundled in the shipped app (see scripts/copy-whisper-model.sh).
/// "base" is the size/quality sweet spot for dictation.
const BUNDLED_MODEL_ID: &str = "base";

#[derive(Serialize)]
pub struct TranscribeResponse {
    /// The transcribed text (may be empty for silence / too-short audio).
    pub text: String,
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/dictation/transcribe",
            post(transcribe_handler).layer(DefaultBodyLimit::max(MAX_AUDIO_SIZE * 2)),
        )
        .route("/api/dictation/provision", post(provision_handler))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct ProvisionRequest {
    /// Absolute path to the bundled model, resolved by the frontend via Tauri
    /// `resolveResource('whisper/whisper-base-q8_0.gguf')`.
    pub bundled_path: String,
}

#[derive(Serialize)]
pub struct ProvisionResponse {
    /// "ready" — the model is installed and `LOCAL_WHISPER_MODEL` is set, so
    /// dictation works. "unavailable" — no bundled model was found (e.g. a dev
    /// build); dictation stays unconfigured and the mic shows its setup hint.
    pub status: String,
    pub model: Option<String>,
}

/// POST /api/dictation/provision — install the bundled dictation model on first
/// run so a shipped install dictates offline with zero setup. Copies the
/// bundled Whisper model into the data dir (once) and points
/// `LOCAL_WHISPER_MODEL` at it. Idempotent and cheap on subsequent calls: if the
/// model is already installed it only re-affirms the config. Never an error when
/// the bundle is simply absent — that's the normal dev case.
async fn provision_handler(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, (StatusCode, String)> {
    let model = permagent::dictation::whisper::get_model(BUNDLED_MODEL_ID).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "unknown bundled model id".to_string(),
    ))?;
    let target = model.local_path();

    // Already installed → make sure the config points at it and return.
    if target.exists() {
        set_dictation_model(BUNDLED_MODEL_ID)?;
        return Ok(Json(ProvisionResponse {
            status: "ready".to_string(),
            model: Some(BUNDLED_MODEL_ID.to_string()),
        }));
    }

    // No bundled model present (dev/unbundled build) — not an error.
    let src = std::path::Path::new(&req.bundled_path);
    if !src.is_file() {
        return Ok(Json(ProvisionResponse {
            status: "unavailable".to_string(),
            model: None,
        }));
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create models dir: {e}"),
            )
        })?;
    }
    // Copy to a temp sibling then rename, so a crash mid-copy never leaves a
    // truncated file that is_downloaded() would treat as a valid model.
    let tmp = target.with_extension("partial");
    tokio::fs::copy(src, &tmp).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy model: {e}"),
        )
    })?;
    tokio::fs::rename(&tmp, &target).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("finalize model: {e}"),
        )
    })?;

    set_dictation_model(BUNDLED_MODEL_ID)?;
    tracing::info!(model = BUNDLED_MODEL_ID, path = %target.display(), "bundled dictation model installed");
    Ok(Json(ProvisionResponse {
        status: "ready".to_string(),
        model: Some(BUNDLED_MODEL_ID.to_string()),
    }))
}

fn set_dictation_model(model_id: &str) -> Result<(), (StatusCode, String)> {
    permagent::config::Config::global()
        .set_param(
            permagent::dictation::whisper::LOCAL_WHISPER_MODEL_CONFIG_KEY,
            model_id,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("set config: {e}"),
            )
        })
}

async fn transcribe_handler(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<TranscribeResponse>, (StatusCode, String)> {
    // One clip per request — take the first multipart field.
    let Ok(Some(field)) = multipart.next_field().await else {
        return Err((StatusCode::BAD_REQUEST, "no audio field".to_string()));
    };
    let data = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read failed: {e}")))?;
    if data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty audio".to_string()));
    }
    if data.len() > MAX_AUDIO_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("audio exceeds the {MAX_AUDIO_SIZE}-byte limit"),
        ));
    }

    transcribe(data.to_vec()).await
}

/// Transcribe raw audio bytes to text, mapping the local-model failure modes to
/// HTTP the UI can act on. Split out from the handler so the `local-inference`
/// cfg-gate stays small and the "feature absent" fallback is explicit.
#[cfg(feature = "local-inference")]
async fn transcribe(audio: Vec<u8>) -> Result<Json<TranscribeResponse>, (StatusCode, String)> {
    match permagent::dictation::providers::transcribe_local(audio).await {
        Ok(text) => Ok(Json(TranscribeResponse { text })),
        Err(e) => {
            let msg = e.to_string();
            // "not configured" / "Unknown model" are setup gaps, not crashes —
            // surface them as 503 so the UI can prompt the user to set up a
            // dictation model rather than reporting a hard failure.
            if msg.contains("not configured") || msg.contains("Unknown model") {
                tracing::info!(error = %msg, "dictation requested but no local model is set up");
                Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "No local dictation model is configured".to_string(),
                ))
            } else {
                tracing::warn!(error = %msg, "local transcription failed");
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

#[cfg(not(feature = "local-inference"))]
async fn transcribe(_audio: Vec<u8>) -> Result<Json<TranscribeResponse>, (StatusCode, String)> {
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "Dictation is not available in this build".to_string(),
    ))
}
