//! Reader ingestion route (#296).
//!
//! `POST /api/reader/ingest` — multipart upload of a single file. The Reader
//! OCRs/extracts it **locally**, writes the full text to the Brain, and returns
//! a compact digest (summary + recall handle). This is the interception point
//! that keeps raw file bytes out of the agent's context: the command-center
//! chat surface POSTs dropped files here *before* base64-ing them into a
//! message. Phase 1 handles images; documents (PDF/DOCX) arrive in Phase 2.

use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use permagent::reader;
use std::sync::Arc;

const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50 MB, matches the chat surface

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/reader/ingest",
            post(ingest_handler).layer(DefaultBodyLimit::max(MAX_FILE_SIZE * 2)),
        )
        .with_state(state)
}

async fn ingest_handler(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<reader::Digest>, StatusCode> {
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "dropped".to_string());
        let mime = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        if data.len() > MAX_FILE_SIZE {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }

        // Phase 1: images only. Documents (PDF/DOCX) are Phase 2 — until then
        // the caller falls back to its prior behavior for non-images.
        if !mime.starts_with("image/") {
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
        }

        let digest = reader::ingest_image(&data, &filename).await.map_err(|e| {
            tracing::warn!("reader: ingest failed for {filename}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        return Ok(Json(digest));
    }
    Err(StatusCode::BAD_REQUEST)
}
