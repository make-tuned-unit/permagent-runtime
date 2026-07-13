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
use serde::Serialize;
use std::sync::Arc;

/// 25 MB — a generous ceiling for a dictated clip (minutes of 16-bit WAV),
/// well under anything that would stall local transcription.
const MAX_AUDIO_SIZE: usize = 25 * 1024 * 1024;

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
        .with_state(state)
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
    match permagent::dictation::transcribe_local(audio).await {
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
