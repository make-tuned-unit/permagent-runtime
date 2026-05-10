//! Workspace routes for Command Center workspaces.
//!
//! Endpoints:
//!   GET    /api/workspaces          — List workspaces for current user
//!   GET    /api/workspaces/active   — Get active workspace ID
//!   POST   /api/workspaces/active   — Set active workspace
//!   GET    /api/workspaces/:id      — Get single workspace
//!   PUT    /api/workspaces/:id/layout — Update layout after resize

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use permagent::workspaces;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    id: String,
    name: String,
    icon: String,
    sort_order: i32,
    layout_json: serde_json::Value,
    is_default: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWorkspaceResponse {
    workspace_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveWorkspaceRequest {
    workspace_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLayoutRequest {
    layout_json: serde_json::Value,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn list_workspaces_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WorkspaceResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let items = workspaces::list_workspaces(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        items
            .into_iter()
            .map(|w| WorkspaceResponse {
                id: w.id,
                name: w.name,
                icon: w.icon,
                sort_order: w.sort_order,
                layout_json: w.layout_json,
                is_default: w.is_default,
                created_at: w.created_at,
                updated_at: w.updated_at,
            })
            .collect(),
    ))
}

async fn get_workspace_handler(
    State(state): State<Arc<AppState>>,
    Path(workspace_id): Path<String>,
) -> Result<Json<WorkspaceResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let ws = workspaces::get_workspace(&pool, &workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(WorkspaceResponse {
        id: ws.id,
        name: ws.name,
        icon: ws.icon,
        sort_order: ws.sort_order,
        layout_json: ws.layout_json,
        is_default: ws.is_default,
        created_at: ws.created_at,
        updated_at: ws.updated_at,
    }))
}

async fn update_layout_handler(
    State(state): State<Arc<AppState>>,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateLayoutRequest>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = workspaces::update_layout(&pool, &workspace_id, &req.layout_json)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if updated {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn get_active_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ActiveWorkspaceResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let id = workspaces::get_active_workspace_id(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ActiveWorkspaceResponse { workspace_id: id }))
}

async fn set_active_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetActiveWorkspaceRequest>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let ok = workspaces::set_active_workspace(&pool, &req.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if ok {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/workspaces", get(list_workspaces_handler))
        .route("/api/workspaces/active", get(get_active_handler))
        .route("/api/workspaces/active", post(set_active_handler))
        .route("/api/workspaces/{workspace_id}", get(get_workspace_handler))
        .route(
            "/api/workspaces/{workspace_id}/layout",
            put(update_layout_handler),
        )
        .with_state(state)
}
