//! Skills CRUD routes — Section F of PHASE1_ARCHITECTURE.
//!
//! Endpoints:
//!   POST   /permagent/skills         — Create a skill from a task definition
//!   GET    /permagent/skills         — List all skills for the current user
//!   GET    /permagent/skills/:id     — Get skill detail with full definition_json
//!   DELETE /permagent/skills/:id     — Delete a skill and its triggers
//!   POST   /permagent/skills/dismiss — Dismiss a proposed skill for 30 days

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use permagent::skills;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkillRequest {
    name: String,
    description: Option<String>,
    tool_used: String,
    argument_shape_hash: String,
    definition_json: serde_json::Value,
    source_task_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkillResponse {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummaryResponse {
    id: String,
    name: String,
    description: Option<String>,
    tool_used: Option<String>,
    trigger_count: i64,
    last_triggered_at: Option<String>,
    usage_count: i64,
    status: String,
    created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillProposalResponse {
    tool_used: String,
    argument_shape_hash: String,
    occurrence_count: i64,
    description: String,
    source_task_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetailResponse {
    id: String,
    name: String,
    description: Option<String>,
    tool_used: Option<String>,
    definition_json: serde_json::Value,
    trigger_type: String,
    trigger_value: Option<String>,
    status: String,
    version: i32,
    source_task_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissSkillRequest {
    argument_shape_hash: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn create_skill_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<CreateSkillResponse>), StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let created = skills::create_skill(
        &pool,
        skills::CreateSkillParams {
            name: req.name,
            description: req.description,
            tool_used: req.tool_used,
            argument_shape_hash: req.argument_shape_hash,
            definition_json: req.definition_json,
            source_task_id: req.source_task_id,
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateSkillResponse {
            id: created.id,
            name: created.name,
        }),
    ))
}

async fn list_skills_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SkillSummaryResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let items = skills::list_skills(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        items
            .into_iter()
            .map(|s| SkillSummaryResponse {
                id: s.id,
                name: s.name,
                description: s.description,
                tool_used: s.tool_used,
                trigger_count: s.trigger_count,
                last_triggered_at: s.last_triggered_at,
                usage_count: s.usage_count,
                status: s.status,
                created_at: s.created_at,
            })
            .collect(),
    ))
}

async fn get_skill_handler(
    State(state): State<Arc<AppState>>,
    Path(skill_id): Path<String>,
) -> Result<Json<SkillDetailResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let detail = skills::get_skill(&pool, &skill_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SkillDetailResponse {
        id: detail.id,
        name: detail.name,
        description: detail.description,
        tool_used: detail.tool_used,
        definition_json: detail.definition_json,
        trigger_type: detail.trigger_type,
        trigger_value: detail.trigger_value,
        status: detail.status,
        version: detail.version,
        source_task_id: detail.source_task_id,
        created_at: detail.created_at,
        updated_at: detail.updated_at,
    }))
}

async fn delete_skill_handler(
    State(state): State<Arc<AppState>>,
    Path(skill_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let deleted = skills::delete_skill(&pool, &skill_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn dismiss_skill_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DismissSkillRequest>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    skills::dismiss_skill(&pool, &req.argument_shape_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

async fn list_proposals_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SkillProposalResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let proposals = skills::list_proposals(&pool, 2)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        proposals
            .into_iter()
            .map(|p| SkillProposalResponse {
                tool_used: p.tool_used,
                argument_shape_hash: p.argument_shape_hash,
                occurrence_count: p.occurrence_count,
                description: p.latest_description,
                source_task_ids: p.source_task_ids,
            })
            .collect(),
    ))
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/permagent/skills", post(create_skill_handler))
        .route("/permagent/skills", get(list_skills_handler))
        .route("/permagent/skills/dismiss", post(dismiss_skill_handler))
        .route("/permagent/skills/proposals", get(list_proposals_handler))
        .route("/permagent/skills/{skill_id}", get(get_skill_handler))
        .route("/permagent/skills/{skill_id}", delete(delete_skill_handler))
        .with_state(state)
}
