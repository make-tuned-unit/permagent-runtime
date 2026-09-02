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
//!
//! Dated to-dos, for the dashboard's cross-project due list:
//!   GET    /api/cards/due                                       — Every dated, unfinished to-do, soonest first
//!   PUT    /api/projects/:project_id/cards/:card_id/due-date    — Set (or clear, with null) a due date
//!   POST   /api/projects/:project_id/cards/:card_id/dismiss-due — Hide a to-do from the due list
//!
//! Post-creation roadmap editing (#251) + per-goal auto-approve (#252):
//!   POST   /api/projects/:project_id/roadmap/goals                        — Insert a goal into the roadmap
//!   PUT    /api/projects/:project_id/roadmap/goals/:card_id/dependencies  — Set a goal's depends_on (validated)
//!   POST   /api/projects/:project_id/roadmap/goals/:card_id/remove        — Splice a goal out (rewire dependents, cancel it)
//!   POST   /api/projects/:project_id/cards/:card_id/auto-approve          — Per-goal auto-approve opt-in (#252)

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use permagent::cards;
use permagent::goal_transition::{self, GuardError};
use permagent::grow_media;
use permagent::projects;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_util::io::ReaderStream;

/// Map a goal-transition guard error onto the HTTP surface.
fn guard_status(e: &GuardError) -> StatusCode {
    match e {
        GuardError::NotFound(_) => StatusCode::NOT_FOUND,
        GuardError::Invalid(_) => StatusCode::BAD_REQUEST,
        GuardError::Denied(_) => StatusCode::FORBIDDEN,
        GuardError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn guard_err(e: GuardError) -> (StatusCode, String) {
    (guard_status(&e), e.to_string())
}

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

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RetryMediaRequest {
    #[serde(default)]
    feedback: Option<String>,
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
    /// and requires this risk_gate decision to be approved by the user first.
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
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let card_type = req.card_type.clone().unwrap_or_else(|| "standard".into());
    let mut metadata_json = req.metadata_json;
    if card_type == "social_post" {
        metadata_json = Some(
            grow_media::enrich_new_social_post(
                &pool,
                &project,
                &req.title,
                req.description.as_deref(),
                metadata_json.unwrap_or_else(|| serde_json::json!({})),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?,
        );
    }
    let card = cards::create_card(
        &pool,
        cards::CreateCard {
            project_id: project.id.clone(),
            title: req.title,
            description: req.description,
            card_type: req.card_type,
            column_id: req.column_id,
            created_by: req.created_by,
            metadata_json,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if card.card_type == "social_post" {
        grow_media::enqueue_after_create(pool, project.id, card.id.clone());
    }
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
    let existing = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    let metadata_json = if let Some(incoming) = req.metadata_json {
        let merged = if existing.card_type == "social_post" {
            let merged = cards::preserve_media_keys(&existing.metadata_json, incoming);
            let next = merged.get(cards::POST_STATUS_KEY).and_then(|v| v.as_str());
            let prev = existing
                .metadata_json
                .get(cards::POST_STATUS_KEY)
                .and_then(|v| v.as_str());
            if next == Some("scheduled") && prev != Some("scheduled") {
                cards::assert_ready_to_schedule(&merged)
                    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
            }
            merged
        } else {
            incoming
        };
        Some(merged)
    } else {
        None
    };
    let updated = cards::update_card(
        &pool,
        &card_id,
        cards::UpdateCard {
            title: req.title,
            description: req.description,
            column_id: req.column_id,
            position: req.position,
            assigned_to: req.assigned_to,
            metadata_json,
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
    // risk_gate decision and return 202 — the deletion executes when the user
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

// ── Post-creation roadmap editing (#251) ───────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertRoadmapGoalRequest {
    title: String,
    description: Option<String>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDependenciesRequest {
    depends_on: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveRoadmapGoalResponse {
    removed: bool,
    /// Whether the goal was cancelled (non-terminal goals are; a goal already
    /// Complete/Cancelled is only spliced out of the graph).
    cancelled: bool,
    /// Number of dependent goals rewired onto the removed goal's own deps.
    rewired_dependents: u32,
}

/// Insert a goal into an existing roadmap: validated dependency wiring, Triage
/// (or straight to Ready when its dependencies are already satisfied).
async fn insert_roadmap_goal_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<InsertRoadmapGoalRequest>,
) -> Result<(StatusCode, Json<CardResponse>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let card = goal_transition::insert_roadmap_goal(
        &pool,
        &project_id,
        goal_transition::NewRoadmapGoal {
            title: req.title,
            description: req.description,
            acceptance_criteria: req.acceptance_criteria,
            tags: req.tags,
            depends_on: req.depends_on,
        },
        // This route is the human's own click arriving over HTTP. Stated here
        // rather than assumed inside the guard, so a future non-human caller
        // has to state its own actor instead of inheriting the user's.
        permagent::decisions::ACTOR_JESSE,
    )
    .await
    .map_err(guard_err)?;
    Ok((StatusCode::CREATED, Json(CardResponse::from(card))))
}

/// Set a goal's dependencies (reorder / re-parent within the graph). The graph
/// is re-validated (no cycles, no dangling ids) before anything is written;
/// afterwards eligible dependents are promoted so auto-dispatch respects the
/// new wiring.
async fn set_goal_dependencies_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id)): Path<(String, String)>,
    Json(req): Json<SetDependenciesRequest>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    if card.project_id != project_id {
        return Err((
            StatusCode::NOT_FOUND,
            "Card does not belong to this project".to_string(),
        ));
    }

    goal_transition::set_goal_dependencies(
        &pool,
        &card_id,
        &req.depends_on,
        permagent::decisions::ACTOR_JESSE,
    )
    .await
    .map_err(guard_err)?;

    // Auto-dispatch respects the change: a goal whose (new) deps are all
    // Complete is promoted Triage → Ready now, not on the next approval —
    // and started, rather than waiting for a manual resume_roadmap (D10).
    permagent::agents::platform_extensions::orchestrator::promote_and_dispatch_dependents(
        &pool,
        &project_id,
        &card_id,
    )
    .await;

    let updated = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    Ok(Json(CardResponse::from(updated)))
}

/// Remove a goal from its roadmap: dependents are rewired onto the removed
/// goal's own dependencies (graph stays valid), then a non-terminal goal is
/// cancelled (kills its worker, supersedes its open decisions). The card
/// itself is kept — hard deletion stays Tier-2 gated via DELETE /cards/:id.
async fn remove_roadmap_goal_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id)): Path<(String, String)>,
) -> Result<Json<RemoveRoadmapGoalResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    if card.project_id != project_id {
        return Err((
            StatusCode::NOT_FOUND,
            "Card does not belong to this project".to_string(),
        ));
    }
    if card.card_type != "goal" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only goal cards can be removed from a roadmap".to_string(),
        ));
    }

    let rewired = goal_transition::detach_goal_from_dependents(
        &pool,
        &card_id,
        permagent::decisions::ACTOR_JESSE,
    )
    .await
    .map_err(guard_err)?;

    // Cancel the goal itself unless it is already terminal.
    let col = cards::get_column(&pool, &card.column_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let binding = col.and_then(|c| c.state_binding).unwrap_or_default();
    let cancelled = if binding == "complete" || binding == "cancelled" {
        false
    } else {
        permagent::agents::platform_extensions::orchestrator::cancel_goal(&pool, &card_id)
            .await
            .map_err(|e| (StatusCode::CONFLICT, e))?;
        true
    };

    // Dependents whose remaining deps are all Complete become dispatchable
    // now — and are dispatched, not left waiting for a human (D10).
    permagent::agents::platform_extensions::orchestrator::promote_and_dispatch_dependents(
        &pool,
        &project_id,
        &card_id,
    )
    .await;

    Ok(Json(RemoveRoadmapGoalResponse {
        removed: true,
        cancelled,
        rewired_dependents: rewired,
    }))
}

// ── Per-goal auto-approve override (#252) ──────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoApproveRequest {
    enabled: bool,
}

/// Toggle a goal's `auto_approve` flag: when set, a VERIFIED PASS from the L2
/// verifier is answered by henry-policy instead of waiting for a manual
/// Review answer (same single gate as the verifier.json goal-type allow-list;
/// see verification::auto_approve_allowed). Default remains Review-required;
/// the flag is a protected metadata key so this audited endpoint — a user
/// surface, not an orchestrator tool — is its only writer.
async fn set_auto_approve_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id)): Path<(String, String)>,
    Json(req): Json<AutoApproveRequest>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    if card.project_id != project_id {
        return Err((
            StatusCode::NOT_FOUND,
            "Card does not belong to this project".to_string(),
        ));
    }

    goal_transition::set_goal_auto_approve(
        &pool,
        &card_id,
        req.enabled,
        permagent::decisions::ACTOR_JESSE,
    )
    .await
    .map_err(guard_err)?;

    let updated = cards::get_card(&pool, &card_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    Ok(Json(CardResponse::from(updated)))
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

// ── Due to-dos (cross-project dashboard) ───────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DueCardResponse {
    id: String,
    title: String,
    project_id: String,
    project_name: String,
    column_id: String,
    column_name: String,
    due_date: String,
    assigned_to: Option<String>,
    updated_at: String,
}

impl From<cards::DueCard> for DueCardResponse {
    fn from(c: cards::DueCard) -> Self {
        Self {
            id: c.id,
            title: c.title,
            project_id: c.project_id,
            project_name: c.project_name,
            column_id: c.column_id,
            column_name: c.column_name,
            due_date: c.due_date,
            assigned_to: c.assigned_to,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDueDateRequest {
    /// `null` clears the due date, removing the to-do from the dashboard.
    due_date: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDismissedRequest {
    dismissed: bool,
}

/// Every dated, unfinished to-do across all projects, soonest first.
async fn list_due_cards_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DueCardResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = cards::list_due_cards(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items.into_iter().map(DueCardResponse::from).collect()))
}

async fn set_due_date_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
    Json(req): Json<SetDueDateRequest>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let updated = cards::set_card_due_date(&pool, &card_id, req.due_date.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    Ok(Json(CardResponse::from(updated)))
}

async fn set_due_dismissed_handler(
    State(state): State<Arc<AppState>>,
    Path((_project_id, card_id)): Path<(String, String)>,
    Json(req): Json<SetDismissedRequest>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let now = chrono::Utc::now().to_rfc3339();
    let updated = cards::set_card_due_dismissed(&pool, &card_id, req.dismissed, &now)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?
        .ok_or((StatusCode::NOT_FOUND, "Card not found".to_string()))?;
    Ok(Json(CardResponse::from(updated)))
}

async fn approve_social_post_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id)): Path<(String, String)>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let card = grow_media::approve_post(&pool, &project.id, &card_id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(CardResponse::from(card)))
}

async fn retry_social_media_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id)): Path<(String, String)>,
    body: Option<Json<RetryMediaRequest>>,
) -> Result<Json<CardResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let feedback = body.as_ref().and_then(|req| req.0.feedback.as_deref());
    let card = grow_media::retry_media(&pool, &project.id, &card_id, feedback)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(CardResponse::from(card)))
}

async fn get_social_media_file_handler(
    State(state): State<Arc<AppState>>,
    Path((project_id, card_id, filename)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let card = cards::get_card(&pool, &card_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if card.project_id != project.id || card.card_type != "social_post" {
        return Err(StatusCode::NOT_FOUND);
    }
    let path = grow_media::resolve_media_file(&project.id, &card.id, &filename)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    };
    let body = Body::from_stream(ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        body,
    ))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // Active goals — single source of truth for every "in flight" surface
        .route("/api/goals/active", get(list_active_goals_handler))
        // Dated to-dos across every board, for the dashboard's due list
        .route("/api/cards/due", get(list_due_cards_handler))
        .route(
            "/api/projects/{project_id}/cards/{card_id}/due-date",
            axum::routing::put(set_due_date_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/{card_id}/dismiss-due",
            post(set_due_dismissed_handler),
        )
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
        .route(
            "/api/projects/{project_id}/cards/{card_id}/approve",
            post(approve_social_post_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/{card_id}/media/retry",
            post(retry_social_media_handler),
        )
        .route(
            "/api/projects/{project_id}/cards/{card_id}/media/{filename}",
            get(get_social_media_file_handler),
        )
        // Post-creation roadmap editing (#251)
        .route(
            "/api/projects/{project_id}/roadmap/goals",
            post(insert_roadmap_goal_handler),
        )
        .route(
            "/api/projects/{project_id}/roadmap/goals/{card_id}/dependencies",
            axum::routing::put(set_goal_dependencies_handler),
        )
        .route(
            "/api/projects/{project_id}/roadmap/goals/{card_id}/remove",
            post(remove_roadmap_goal_handler),
        )
        // Per-goal auto-approve override (#252)
        .route(
            "/api/projects/{project_id}/cards/{card_id}/auto-approve",
            post(set_auto_approve_handler),
        )
        .with_state(state)
}

#[cfg(test)]
mod due_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use serial_test::serial;
    use tower::ServiceExt;

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        if bytes.is_empty() {
            return serde_json::Value::Null;
        }
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Exercises the real wire contract: a to-do created over HTTP, dated over
    /// HTTP, and read back from the cross-project due list. The SQL is covered
    /// by unit tests in `permagent::cards`; what this pins is the part they
    /// cannot see — the routes are actually mounted, and the JSON is camelCase
    /// the way the dashboard reads it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn due_list_round_trips_over_http() {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let app = routes(state);

        let project_id = permagent::projects::PERSONAL_PROJECT_ID;

        // Create a to-do.
        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/projects/{}/cards", project_id))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "title": "water the plants" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let card = body_json(created).await;
        let card_id = card["id"].as_str().unwrap().to_string();

        // Undated: absent from the due list.
        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/cards/due")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let before = body_json(listed).await;
        assert!(
            !before
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == card_id.as_str()),
            "an undated card must not appear in the due list"
        );

        // Give it a due date.
        let dated = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{}/cards/{}/due-date",
                        project_id, card_id
                    ))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "dueDate": "2026-08-09" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dated.status(), StatusCode::OK);

        // Now present, with the camelCase fields the dashboard binds to.
        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/cards/due")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let after = body_json(listed).await;
        let row = after
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == card_id.as_str())
            .expect("dated to-do must appear in the due list");
        assert_eq!(row["dueDate"], "2026-08-09");
        assert_eq!(row["title"], "water the plants");
        assert!(row["projectName"].is_string());
        assert!(row["columnName"].is_string());

        // Dismiss removes it from the list.
        let dismissed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{}/cards/{}/dismiss-due",
                        project_id, card_id
                    ))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "dismissed": true }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dismissed.status(), StatusCode::OK);

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/cards/due")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let dismissed_list = body_json(listed).await;
        assert!(
            !dismissed_list
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == card_id.as_str()),
            "a dismissed to-do must not appear in the due list"
        );
    }

    /// A malformed date must be refused at the edge with a 400, not stored and
    /// left to sort into a nonsense position later.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn malformed_due_date_is_rejected_over_http() {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let project_id = permagent::projects::PERSONAL_PROJECT_ID;

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/projects/{}/cards", project_id))
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "title": "bad date" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let card_id = body_json(created).await["id"].as_str().unwrap().to_string();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{}/cards/{}/due-date",
                        project_id, card_id
                    ))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "dueDate": "next tuesday" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
