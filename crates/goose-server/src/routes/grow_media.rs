//! Grow media credentials and per-project publisher connections.
//!
//! Higgsfield and Postiz API keys live in this user's secret store (once per
//! install). Channel bindings (which Instagram account posts for this project)
//! live on the project. Nothing is shared across installs or hardcoded to a
//! particular product.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use permagent::config::Config;
use permagent::grow_media::{
    credentials_configured, disconnect_channel, postiz_base_url, postiz_configured,
    publisher_snapshot, start_connect, PublisherSnapshot, HF_KEY_ID, HF_KEY_SECRET, POSTIZ_API_KEY,
    POSTIZ_BASE_URL_KEY, POSTIZ_DEFAULT_BASE,
};
use permagent::projects;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HiggsfieldStatus {
    configured: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveHiggsfieldRequest {
    key_id: String,
    secret: String,
}

async fn get_higgsfield() -> Json<HiggsfieldStatus> {
    Json(HiggsfieldStatus {
        configured: credentials_configured(),
    })
}

async fn save_higgsfield(
    Json(req): Json<SaveHiggsfieldRequest>,
) -> Result<Json<HiggsfieldStatus>, (StatusCode, String)> {
    let key_id = req.key_id.trim();
    let secret = req.secret.trim();
    if key_id.is_empty() || secret.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "keyId and secret are required".into(),
        ));
    }
    let cfg = Config::global();
    cfg.set_secret(HF_KEY_ID, &key_id.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    cfg.set_secret(HF_KEY_SECRET, &secret.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(HiggsfieldStatus { configured: true }))
}

async fn delete_higgsfield() -> Result<Json<HiggsfieldStatus>, (StatusCode, String)> {
    let cfg = Config::global();
    cfg.delete_secret(HF_KEY_ID)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    cfg.delete_secret(HF_KEY_SECRET)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(HiggsfieldStatus { configured: false }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostizStatus {
    configured: bool,
    base_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavePostizRequest {
    api_key: String,
    base_url: Option<String>,
}

async fn get_postiz() -> Json<PostizStatus> {
    Json(PostizStatus {
        configured: postiz_configured(),
        base_url: postiz_base_url(),
    })
}

async fn save_postiz(
    Json(req): Json<SavePostizRequest>,
) -> Result<Json<PostizStatus>, (StatusCode, String)> {
    let api_key = req.api_key.trim();
    if api_key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "apiKey is required".into()));
    }
    let cfg = Config::global();
    cfg.set_secret(POSTIZ_API_KEY, &api_key.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let base = req
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(POSTIZ_DEFAULT_BASE);
    cfg.set_param(POSTIZ_BASE_URL_KEY, &base.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PostizStatus {
        configured: true,
        base_url: postiz_base_url(),
    }))
}

async fn delete_postiz() -> Result<Json<PostizStatus>, (StatusCode, String)> {
    let cfg = Config::global();
    cfg.delete_secret(POSTIZ_API_KEY)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PostizStatus {
        configured: false,
        base_url: postiz_base_url(),
    }))
}

async fn pool(state: &AppState) -> Result<sqlx::Pool<sqlx::Sqlite>, (StatusCode, String)> {
    state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn resolve_project(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    id: &str,
) -> Result<permagent::projects::Project, (StatusCode, String)> {
    projects::get_project_by_id_or_slug(pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))
}

async fn get_publisher(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<PublisherSnapshot>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let project = resolve_project(&pool, &project_id).await?;
    let snap = publisher_snapshot(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(snap))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest {
    channel: String,
}

async fn connect_publisher(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<ConnectRequest>,
) -> Result<Json<permagent::grow_media::ConnectStart>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let project = resolve_project(&pool, &project_id).await?;
    let start = start_connect(&pool, &project.id, &req.channel)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(start))
}

async fn disconnect_publisher(
    State(state): State<Arc<AppState>>,
    Path((project_id, channel)): Path<(String, String)>,
) -> Result<Json<PublisherSnapshot>, (StatusCode, String)> {
    let pool = pool(&state).await?;
    let project = resolve_project(&pool, &project_id).await?;
    let snap = disconnect_channel(&pool, &project.id, &channel)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(snap))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/grow/higgsfield",
            get(get_higgsfield)
                .put(save_higgsfield)
                .delete(delete_higgsfield),
        )
        .route(
            "/api/grow/postiz",
            get(get_postiz).put(save_postiz).delete(delete_postiz),
        )
        .route("/api/projects/{project_id}/publisher", get(get_publisher))
        .route(
            "/api/projects/{project_id}/publisher/connect",
            post(connect_publisher),
        )
        .route(
            "/api/projects/{project_id}/publisher/{channel}",
            delete(disconnect_publisher),
        )
        .with_state(state)
}
