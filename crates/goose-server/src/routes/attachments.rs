//! Attachment routes for file upload/download (Phase 2 Track 1).
//!
//! Endpoints:
//!   POST   /api/sessions/:session_id/upload                       — Upload files
//!   GET    /api/sessions/:session_id/attachments/:attachment_id   — Stream file
//!   DELETE /api/sessions/:session_id/attachments/:attachment_id   — Remove file

use crate::state::AppState;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use permagent::attachments;
use serde::Serialize;
use std::sync::Arc;
use tokio::fs;
use tokio_util::io::ReaderStream;

const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50 MB

#[derive(Serialize)]
struct AttachmentInfo {
    id: String,
    filename: String,
    mime_type: String,
    size_bytes: i64,
    created_at: String,
}

#[derive(Serialize)]
struct UploadResponse {
    attachments: Vec<AttachmentInfo>,
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/sessions/{session_id}/upload",
            post(upload_handler).layer(DefaultBodyLimit::max(MAX_FILE_SIZE * 10)),
        )
        .route(
            "/api/sessions/{session_id}/attachments/{attachment_id}",
            get(get_handler),
        )
        .route(
            "/api/sessions/{session_id}/attachments/{attachment_id}",
            delete(delete_handler),
        )
        .with_state(state)
}

async fn upload_handler(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !attachments::session_exists(&pool, &session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let uploads_base = dirs::home_dir()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
        .join(".permagent")
        .join("uploads")
        .join(&session_id);

    let mut results = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unnamed".to_string());

        let mime_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;

        if data.len() > MAX_FILE_SIZE {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }

        let attachment_id = uuid::Uuid::now_v7().to_string();
        let dir = uploads_base.join(&attachment_id);
        fs::create_dir_all(&dir)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let file_path = dir.join(&filename);
        fs::write(&file_path, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let size_bytes = data.len() as i64;
        let path_str = file_path.to_string_lossy().to_string();

        let created_at = attachments::insert_attachment(
            &pool,
            &attachment_id,
            &session_id,
            &filename,
            &mime_type,
            size_bytes,
            &path_str,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        results.push(AttachmentInfo {
            id: attachment_id,
            filename,
            mime_type,
            size_bytes,
            created_at,
        });
    }

    Ok(Json(UploadResponse {
        attachments: results,
    }))
}

async fn get_handler(
    State(state): State<Arc<AppState>>,
    Path((session_id, attachment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let record = attachments::get_attachment(&pool, &session_id, &attachment_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let file = tokio::fs::File::open(&record.path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let disposition = format!("inline; filename=\"{}\"", record.filename);

    Ok((
        [
            (header::CONTENT_TYPE, record.mime_type),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        body,
    ))
}

async fn delete_handler(
    State(state): State<Arc<AppState>>,
    Path((session_id, attachment_id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let path = attachments::delete_attachment(&pool, &session_id, &attachment_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let _ = fs::remove_file(&path).await;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = fs::remove_dir(parent).await;
    }

    Ok(StatusCode::NO_CONTENT)
}
