//! Council of LLMs HTTP surface: latest report, one session, membership.

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use permagent::council::{membership, store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestResponse {
    session: Option<store::Session>,
    report: Option<store::Report>,
    positions: Vec<store::Position>,
    open_actions: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MembersResponse {
    enabled: bool,
    exclude: Vec<String>,
    seats: Vec<membership::Seat>,
}

#[derive(Deserialize)]
struct MembersPut {
    exclude: Vec<String>,
}

async fn latest(State(state): State<Arc<AppState>>) -> Result<Json<LatestResponse>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    let pair = store::latest_finished(&pool)
        .await
        .map_err(|e| ErrorResponse::internal(e))?;
    let open_actions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM decisions WHERE kind = 'council_action' AND status = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(0);
    match pair {
        Some((session, report)) => {
            let positions = store::list_positions(&pool, &session.id)
                .await
                .unwrap_or_default();
            Ok(Json(LatestResponse {
                session: Some(session),
                report,
                positions,
                open_actions,
            }))
        }
        None => Ok(Json(LatestResponse {
            session: None,
            report: None,
            positions: Vec::new(),
            open_actions,
        })),
    }
}

async fn one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LatestResponse>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    let session = store::get_session(&pool, &id)
        .await
        .map_err(|e| ErrorResponse::internal(e))?
        .ok_or_else(|| ErrorResponse::not_found(format!("no council session {id}")))?;
    let report = store::get_report_for_session(&pool, &id)
        .await
        .map_err(|e| ErrorResponse::internal(e))?;
    let positions = store::list_positions(&pool, &id)
        .await
        .map_err(|e| ErrorResponse::internal(e))?;
    Ok(Json(LatestResponse {
        session: Some(session),
        report,
        positions,
        open_actions: 0,
    }))
}

async fn members() -> Json<MembersResponse> {
    Json(MembersResponse {
        enabled: permagent::council::is_enabled(),
        exclude: membership::excluded_providers(),
        seats: membership::resolve_seats().await,
    })
}

async fn put_members(Json(body): Json<MembersPut>) -> Result<Json<MembersResponse>, ErrorResponse> {
    membership::set_excluded(&body.exclude).map_err(ErrorResponse::unprocessable)?;
    Ok(members().await)
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/council/latest", get(latest))
        .route("/api/council/members", get(members).put(put_members))
        .route("/api/council/{id}", get(one))
        .with_state(state)
}
