//! Council of LLMs HTTP surface: latest report, one session, membership.

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConveneRequest {
    question: String,
    project: Option<String>,
    /// Existing harness session whose durable budget task authorizes every
    /// Council member/chair invocation.
    session_id: String,
    /// Must be true. This endpoint spends across every seated provider, so a
    /// background classifier may recommend it but cannot approve it.
    approved: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConveneAccepted {
    accepted: bool,
    message: String,
}

async fn latest(State(state): State<Arc<AppState>>) -> Result<Json<LatestResponse>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    let pair = store::latest_finished(&pool)
        .await
        .map_err(ErrorResponse::internal)?;
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
        .map_err(ErrorResponse::internal)?
        .ok_or_else(|| ErrorResponse::not_found(format!("no council session {id}")))?;
    let report = store::get_report_for_session(&pool, &id)
        .await
        .map_err(ErrorResponse::internal)?;
    let positions = store::list_positions(&pool, &id)
        .await
        .map_err(ErrorResponse::internal)?;
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

async fn convene_once(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ConveneRequest>,
) -> Result<(StatusCode, Json<ConveneAccepted>), ErrorResponse> {
    if !body.approved {
        return Err(ErrorResponse::unprocessable(
            "explicit approval is required before a Council pass spends across providers",
        ));
    }
    let question: String = body.question.trim().chars().take(24_000).collect();
    let session_id = body.session_id.trim().to_string();
    let project = body
        .project
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().take(512).collect::<String>());
    if question.is_empty() {
        return Err(ErrorResponse::bad_request(
            "the Council needs the Build request it is planning",
        ));
    }
    if session_id.is_empty() {
        return Err(ErrorResponse::bad_request(
            "the Council requires an existing harness session_id for cost attribution",
        ));
    }
    // Reserve the same process-wide slot used by the weekly sweep before
    // returning 202, so acceptance cannot race with another Council source.
    let reservation = permagent::council::try_reserve().map_err(ErrorResponse::conflict)?;
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    state
        .session_manager()
        .get_session(&session_id, false)
        .await
        .map_err(|e| ErrorResponse::bad_request(format!("unknown harness session_id: {e}")))?;
    if store::has_running(&pool)
        .await
        .map_err(ErrorResponse::internal)?
    {
        return Err(ErrorResponse::conflict(
            "a Council session is already running",
        ));
    }
    if membership::resolve_members().await.is_empty() {
        return Err(ErrorResponse::service_unavailable(
            "no connected chat providers are available for the Council",
        ));
    }
    tokio::spawn(async move {
        if let Err(error) = permagent::council::convene_approved_reserved(
            &pool,
            Some(&question),
            project.as_deref(),
            &permagent::council::debate::LiveCaller::new(
                state.agent_manager.session_manager_arc(),
                session_id,
            ),
            reservation,
        )
        .await
        {
            tracing::warn!(target: "permagentd::council", %error, "approved Build Council pass failed");
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ConveneAccepted {
            accepted: true,
            message: "Council convening from the live Build request".to_string(),
        }),
    ))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/council/latest", get(latest))
        .route("/api/council/convene", post(convene_once))
        .route("/api/council/members", get(members).put(put_members))
        .route("/api/council/{id}", get(one))
        .with_state(state)
}
