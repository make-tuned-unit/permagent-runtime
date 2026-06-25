//! Card and board column routes for project Kanban boards.
//!
//! Endpoints:
//!   GET    /api/projects/:project_id/columns              — List columns
//!   POST   /api/projects/:project_id/columns              — Add column
//!   PATCH  /api/projects/:project_id/columns/:col_id      — Update column
//!   DELETE /api/projects/:project_id/columns/:col_id      — Delete column (refuses if cards present)
//!
//!   GET    /api/projects/:project_id/cards                 — List cards (optional ?card_type= filter)
//!   GET    /api/projects/:project_id/cards/:card_id        — Get card detail
//!   POST   /api/projects/:project_id/cards                 — Create card
//!   PATCH  /api/projects/:project_id/cards/:card_id        — Update card
//!   DELETE /api/projects/:project_id/cards/:card_id        — Hard delete card
//!   POST   /api/projects/:project_id/cards/:card_id/cancel — Cancel a goal (kills worker, terminal)
//!   POST   /api/projects/:project_id/cards/reorder         — Batch reorder cards

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use permagent::cards;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Response types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnResponse {
    id: String,
    project_id: String,
    name: String,
    position: i32,
    column_kind: String,
    state_binding: Option<String>,
    wip_limit: Option<i32>,
    created_at: String,
}

impl From<cards::BoardColumn> for ColumnResponse {
    fn from(c: cards::BoardColumn) -> Self {
        Self {
            id: c.id,
            project_id: c.project_id,
            name: c.name,
            position: c.position,
            column_kind: c.column_kind,
            state_binding: c.state_binding,
            wip_limit: c.wip_limit,
            created_at: c.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardResponse {
    id: String,
    project_id: String,
    card_type: String,
    title: String,
    description: String,
    column_id: String,
    position: i32,
    created_by: String,
    assigned_to: Option<String>,
    metadata_json: serde_json::Value,
    created_at: String,
    updated_at: String,
    archived_at: Option<String>,
}

impl From<cards::Card> for CardResponse {
    fn from(c: cards::Card) -> Self {
        Self {
            id: c.id,
            project_id: c.project_id,
            card_type: c.card_type,
            title: c.title,
            description: c.description,
            column_id: c.column_id,
            position: c.position,
            created_by: c.created_by,
            assigned_to: c.assigned_to,
            metadata_json: c.metadata_json,
            created_at: c.created_at,
            updated_at: c.updated_at,
            archived_at: c.archived_at,
        }
    }
}

// ── Request types ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateColumnRequest {
    name: String,
    position: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateColumnRequest {
    name: Option<String>,
    wip_limit: Option<Option<i32>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCardRequest {
    title: String,
    description: Option<String>,
    card_type: Option<String>,
    column_id: Option<String>,
    created_by: Option<String>,
    metadata_json: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCardRequest {
    title: Option<String>,
    description: Option<String>,
    column_id: Option<String>,
    position: Option<i32>,
    assigned_to: Option<Option<String>>,
    metadata_json: Option<serde_json::Value>,
    archived_at: Option<Option<String>>,
}

#[derive(Deserialize)]
pub struct ListCardsQuery {
    card_type: Option<String>,
    column_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderEntry {
    card_id: String,
    column_id: String,
    position: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResponse {
    deleted: bool,
    /// Set when the card is a goal: deletion is Tier 2 (user_data_deletion)
    /// and requires this risk_gate decision to be approved by Jesse first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_decision_id: Option<String>,
}

// ── Column handlers ────────────────────────────────────────────────────────

async fn list_columns_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<ColumnResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cols = cards::list_columns(&pool, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(cols.into_iter().map(ColumnResponse::from).collect()))
}

async fn create_column_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateColumnRequest>,
) -> Result<(StatusCode, Json<ColumnResponse>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let col = cards::create_column(
        &pool,
        cards::CreateColumn {
            project_id,
            name: req.name,
            position: req.position,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(ColumnResponse::from(col))))
}

async fn update_column_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, col_id)): Path<(String, String)>,
    Json(req): Json<UpdateColumnRequest>,
) -> Result<Json<ColumnResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let updated = cards::update_column(
        &pool,
        &col_id,
        cards::UpdateColumn {
            name: req.name,
            wip_limit: req.wip_limit,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    .ok_or((StatusCode::NOT_FOUND, "Column not found".to_string()))?;
    Ok(Json(ColumnResponse::from(updated)))
}

async fn delete_column_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, col_id)): Path<(String, String)>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let deleted = cards::delete_column(&pool, &col_id)
        .await
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))?;
    Ok(Json(DeleteResponse {
        deleted,
        pending_decision_id: None,
    }))
}

// ── Card handlers ──────────────────────────────────────────────────────────

async fn list_cards_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(query): Query<ListCardsQuery>,
) -> Result<Json<Vec<CardResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = cards::list_cards(
        &pool,
        &project_id,
        query.card_type.as_deref(),
        query.column_id.as_deref(),
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items.into_iter().map(CardResponse::from).collect()))
}

async fn get_card_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
) -> Result<Json<CardResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(CardResponse::from(card)))
}

async fn create_card_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateCardRequest>,
) -> Result<(StatusCode, Json<CardResponse>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let card = cards::create_card(
        &pool,
        cards::CreateCard {
            project_id,
            title: req.title,
            description: req.description,
            card_type: req.card_type,
            column_id: req.column_id,
            created_by: req.created_by,
            metadata_json: req.metadata_json,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(CardResponse::from(card))))
}

async fn update_card_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
    Json(req): Json<UpdateCardRequest>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let updated = cards::update_card(
        &pool,
        &card_id,
        cards::UpdateCard {
            title: req.title,
            description: req.description,
            column_id: req.column_id,
            position: req.position,
            assigned_to: req.assigned_to,
            metadata_json: req.metadata_json,
            archived_at: req.archived_at,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    Ok(Json(CardResponse::from(updated)))
}

async fn delete_card_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<DeleteResponse>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;

    // Goal deletion is Tier 2 (user_data_deletion): file (or surface) a
    // risk_gate decision and return 202 — the deletion executes when Jesse
    // approves the decision in the inbox.
    if card.card_type == "goal" {
        let decision =
            match permagent::decisions::find_open_decision_for_goal(&pool, &card_id, "risk_gate")
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            {
                Some(d) => d,
                None => {
                    let headline = {
                        let h = format!("Permission to delete the goal \"{}\"", card.title);
                        if h.chars().count() > permagent::decisions::MAX_HEADLINE_CHARS {
                            let cut: String = h
                                .chars()
                                .take(permagent::decisions::MAX_HEADLINE_CHARS - 1)
                                .collect();
                            format!("{}…", cut)
                        } else {
                            h
                        }
                    };
                    permagent::decisions::create_decision(
                        &pool,
                        permagent::decisions::NewDecision {
                            kind: "risk_gate".to_string(),
                            goal_id: Some(card_id.clone()),
                            project_id: Some(card.project_id.clone()),
                            headline: Some(headline),
                            detail: Some(format!(
                            "DELETE was requested for goal card {} (project {}). Goal deletion \
                             is Tier 2 (user_data_deletion); approving this decision deletes \
                             the card permanently.",
                            card_id, card.project_id
                        )),
                            payload: serde_json::json!({
                                "action_class": "user_data_deletion",
                                "description": format!("Delete goal card '{}'", card.title),
                                "requested_by": "http",
                            }),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                }
            };

        return Ok((
            StatusCode::ACCEPTED,
            Json(DeleteResponse {
                deleted: false,
                pending_decision_id: Some(decision.id),
            }),
        ));
    }

    let deleted = cards::delete_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if deleted {
        Ok((
            StatusCode::OK,
            Json(DeleteResponse {
                deleted,
                pending_decision_id: None,
            }),
        ))
    } else {
        Err((StatusCode::NOT_FOUND, "Card not found".to_string()))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelResponse {
    cancelled: bool,
    state: String,
}

/// Cancel a goal (#490): kill its worker if running and move it to the terminal
/// Cancelled state. User-initiated and immediate — no approval gate (unlike
/// delete). Shared by both the Decision Inbox and the Kanban card menu.
async fn cancel_card_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
) -> Result<Json<CancelResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    if card.card_type != "goal" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only goal cards can be cancelled".to_string(),
        ));
    }

    // A cancel from a terminal state (already Complete/Cancelled) surfaces as a
    // guard rejection → 409 Conflict.
    let new_state =
        permagent::agents::platform_extensions::orchestrator::cancel_goal(&pool, &card_id)
            .await
            .map_err(|e| (StatusCode::CONFLICT, e))?;

    Ok(Json(CancelResponse {
        cancelled: true,
        state: new_state.binding().to_string(),
    }))
}

async fn reorder_cards_handler(
    State(state): State<Arc<AppState>>,
    Path(_project_id): Path<String>,
    Json(entries): Json<Vec<ReorderEntry>>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let moves: Vec<(String, String, i32)> = entries
        .into_iter()
        .map(|e| (e.card_id, e.column_id, e.position))
        .collect();
    cards::reorder_cards(&pool, &moves)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::OK)
}

// ── Route registration ────────────────────────────────────────────────────

/// Unified "in flight" payload: the active-goal list and its count come from a
/// single query, so the dashboard's count, list, header, and status can never
/// disagree (the bug this endpoint fixes).
#[derive(Serialize)]
struct ActiveGoalsResponse {
    count: usize,
    goals: Vec<cards::ActiveGoal>,
}

async fn list_active_goals_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ActiveGoalsResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let goals = cards::list_active_goals(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ActiveGoalsResponse {
        count: goals.len(),
        goals,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // Active goals — single source of truth for every "in flight" surface
        .route("/api/goals/active", get(list_active_goals_handler))
        // Columns
        .route(
            "/api/projects/{project_id}/columns",
            get(list_columns_handler).post(create_column_handler),
        )
        .route(
            "/api/projects/{project_id}/columns/{col_id}",
            patch(update_column_handler).delete(delete_column_handler),
        )
        // Cards
        .route(
            "/api/projects/{project_id}/cards",
            get(list_cards_handler).post(create_card_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/reorder",
            post(reorder_cards_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/{card_id}",
            get(get_card_handler)
                .patch(update_card_handler)
                .delete(delete_card_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/{card_id}/cancel",
            post(cancel_card_handler),
        )
        .with_state(state)
}
