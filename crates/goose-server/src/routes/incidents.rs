//! Incident triage (wave-1 item 2).
//!
//! The incidents table was insert-only: workers filed failure evidence and
//! `list_open_incidents` fed it into every decompose prompt forever — nothing
//! could ever close one. These two routes are the missing lifecycle half:
//!
//!   GET  /api/incidents            — open incidents, newest first (?limit=)
//!   POST /api/incidents/{id}/resolve — mark resolved (idempotent)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::incidents;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct IncidentView {
    id: String,
    created_at: String,
    session_id: Option<String>,
    surface: String,
    user_goal: String,
    observation: String,
    mechanism: String,
    artifact_kind: String,
    artifact_ref: String,
    status: String,
    resolved_at: Option<String>,
}

impl From<incidents::Incident> for IncidentView {
    fn from(i: incidents::Incident) -> Self {
        Self {
            id: i.id,
            created_at: i.created_at,
            session_id: i.session_id,
            surface: i.surface,
            user_goal: i.user_goal,
            observation: i.observation,
            mechanism: i.mechanism.as_str().to_string(),
            artifact_kind: i.artifact_kind.as_str().to_string(),
            artifact_ref: i.artifact_ref,
            status: i.status,
            resolved_at: i.resolved_at,
        }
    }
}

async fn pool_of(state: &Arc<AppState>) -> Result<sqlx::Pool<sqlx::Sqlite>, (StatusCode, String)> {
    state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list_incidents(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<IncidentView>>, (StatusCode, String)> {
    let pool = pool_of(&state).await?;
    let items = incidents::list_open_incidents(&pool, q.limit.unwrap_or(50).clamp(1, 500))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items.into_iter().map(IncidentView::from).collect()))
}

#[derive(Debug, Serialize)]
struct ResolveResponse {
    /// The updated incident, or null when the id was unknown or already
    /// resolved (idempotent no-op, reported honestly rather than erroring).
    incident: Option<IncidentView>,
    changed: bool,
}

async fn resolve_incident(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ResolveResponse>, (StatusCode, String)> {
    let pool = pool_of(&state).await?;
    let updated = incidents::resolve_incident(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let changed = updated.is_some();
    Ok(Json(ResolveResponse {
        incident: updated.map(IncidentView::from),
        changed,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/incidents", get(list_incidents))
        .route("/api/incidents/{id}/resolve", post(resolve_incident))
        .with_state(state)
}
