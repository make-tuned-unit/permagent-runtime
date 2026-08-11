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

        let file_path = dir.join(&attachment_id);
        let canonical_uploads_base = fs::canonicalize(&uploads_base)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let canonical_parent =
            fs::canonicalize(file_path.parent().expect("attachment path has parent"))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !canonical_parent.starts_with(&canonical_uploads_base) {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let file_path = canonical_parent.join(
            file_path
                .file_name()
                .expect("generated attachment path has basename"),
        );
        fs::write(&file_path, &data)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let size_bytes = data.len() as i64;
        let path_str = file_path.to_string_lossy().to_string();

        let created_at = match attachments::insert_attachment(
            &pool,
            &attachment_id,
            &session_id,
            &filename,
            &mime_type,
            size_bytes,
            &path_str,
        )
        .await
        {
            Ok(ts) => ts,
            Err(_) => {
                // The bytes are already on disk; if the DB row fails, remove the
                // file so it isn't orphaned with no record pointing at it.
                let _ = fs::remove_file(&file_path).await;
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

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

    let disposition = content_disposition(&record.filename);

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

/// Build an RFC 6266 `Content-Disposition` for a user-supplied filename.
///
/// The previous version emitted a bare `filename="…"` after stripping quotes,
/// backslashes and control characters — and its comment claimed that prevented
/// "a 500 on an invalid header value". It did not: non-ASCII survived the
/// filter, and `HeaderValue` accepts only visible ASCII, so a perfectly
/// ordinary upload named `café.pdf` produced an invalid header and failed the
/// download outright. The sanitizer was guarding against injection, which is a
/// different problem from encoding, and the comment made the gap look covered.
///
/// So: an ASCII-only `filename=` fallback for old clients, plus `filename*=`
/// carrying the real UTF-8 name percent-encoded. Every byte outside the RFC
/// 5987 `attr-char` set is escaped, which subsumes the injection concern —
/// quotes, backslashes and control bytes cannot survive percent-encoding.
fn content_disposition(filename: &str) -> String {
    // ASCII fallback: anything non-ASCII or structurally unsafe becomes '_'.
    let ascii: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ascii = if ascii.trim().is_empty() {
        "download".to_string()
    } else {
        ascii
    };

    // RFC 5987 attr-char: ALPHA / DIGIT / !#$&+-.^_`|~
    fn is_attr_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b)
    }
    let mut encoded = String::new();
    for b in filename.as_bytes() {
        if is_attr_char(*b) {
            encoded.push(*b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }

    format!("inline; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::content_disposition;
    use axum::http::HeaderValue;

    /// The bug: `café.pdf` is an ordinary filename, and it made the download
    /// return 500 because the header value was not valid ASCII. Every case here
    /// asserts the result is a CONSTRUCTIBLE HeaderValue — that is the property
    /// that was actually broken, not the string's shape.
    #[test]
    fn non_ascii_filenames_produce_a_valid_header() {
        for name in [
            "café.pdf",
            "日本語のファイル.txt",
            "Ünterlagen — final (v2).docx",
            "emoji 🎉.png",
        ] {
            let v = content_disposition(name);
            assert!(
                HeaderValue::from_str(&v).is_ok(),
                "{name} produced an invalid header value: {v}"
            );
            assert!(
                v.contains("filename*=UTF-8''"),
                "{name} must carry the encoded name"
            );
        }
    }

    #[test]
    fn the_real_name_survives_percent_encoded() {
        let v = content_disposition("café.pdf");
        // c a f + é (0xC3 0xA9) + . p d f
        assert!(v.contains("filename*=UTF-8''caf%C3%A9.pdf"), "{v}");
        // ...and the ASCII fallback stays present for old clients.
        assert!(v.contains("filename=\"caf_.pdf\""), "{v}");
    }

    /// Quotes and backslashes were the ORIGINAL concern; percent-encoding
    /// subsumes it, so the injection guarantee must not regress.
    #[test]
    fn quotes_and_backslashes_cannot_escape_the_quoted_string() {
        let v = content_disposition(r#"evil".pdf"#);
        assert!(HeaderValue::from_str(&v).is_ok());
        // Exactly two quotes: the pair delimiting the ASCII fallback.
        assert_eq!(v.matches('"').count(), 2, "{v}");
        let v2 = content_disposition(r"back\slash.pdf");
        assert!(HeaderValue::from_str(&v2).is_ok());
        assert!(!v2.contains('\\'), "{v2}");
    }

    #[test]
    fn control_bytes_cannot_inject_a_header() {
        let v = content_disposition("bad\r\nX-Injected: 1.pdf");
        assert!(HeaderValue::from_str(&v).is_ok());
        assert!(!v.contains('\r') && !v.contains('\n'), "{v}");
    }

    #[test]
    fn an_empty_or_unnameable_file_still_downloads() {
        let v = content_disposition("");
        assert!(HeaderValue::from_str(&v).is_ok());
        assert!(v.contains("filename=\"download\""), "{v}");
    }
}
