//! File-intake inbox routes.
//!
//! Endpoints:
//!   GET  /api/inbox   — list inbox files (metadata rows), newest first
//!   POST /api/inbox   — record a file that landed in the Permagent inbox
//!
//! The in-app Browser webview (desktop process) redirects a download onto disk
//! under `~/.permagent/inbox/` and then POSTs the metadata here so it persists in
//! permagent.db and is listable. Capture + persist + list slice of epic #392
//! (#393); routing (#394/#395) is a later slice. See [`permagent::inbox`].

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use permagent::inbox::{self, InboxFile, NewInboxFile};
use std::sync::Arc;

async fn list_inbox_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<InboxFile>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let files = inbox::list_inbox_files(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(files))
}

async fn create_inbox_handler(
    State(state): State<Arc<AppState>>,
    Json(new): Json<NewInboxFile>,
) -> Result<(StatusCode, Json<InboxFile>), StatusCode> {
    if new.filename.trim().is_empty() || new.disk_path.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let saved = inbox::insert_inbox_file(&pool, &new)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(saved)))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/inbox",
            get(list_inbox_handler).post(create_inbox_handler),
        )
        .with_state(state)
}
