//! Authenticated handoff for the declarative program DAG.
//!
//! The route is intentionally small: the eval CLI supplies a manifest and
//! explicit receipts, while the daemon verifies them against an existing,
//! terminal-success goal and then uses the normal roadmap dispatcher.

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use permagent::agents::platform_extensions::program_bridge::{
    apply_handoff, register_program, HandoffStatus, ProgramHandoffError, ProgramHandoffRequest,
    ProgramHandoffResponse, ProgramRegistrationRequest, ProgramRegistrationResponse,
};
use serde::Serialize;
use std::sync::Arc;

use crate::state::AppState;

#[derive(Debug, Serialize)]
struct ProgramError {
    error: String,
}

fn error_response(error: ProgramHandoffError) -> (StatusCode, Json<ProgramError>) {
    let status = match error {
        ProgramHandoffError::Invalid(_) => StatusCode::BAD_REQUEST,
        ProgramHandoffError::Conflict(_) => StatusCode::CONFLICT,
        ProgramHandoffError::Pending(_) => StatusCode::SERVICE_UNAVAILABLE,
        ProgramHandoffError::Storage(_) => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(ProgramError {
            error: error.to_string(),
        }),
    )
}

async fn handoff(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProgramHandoffRequest>,
) -> Result<(StatusCode, Json<ProgramHandoffResponse>), (StatusCode, Json<ProgramError>)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|error| error_response(ProgramHandoffError::Storage(error.to_string())))?;
    let response = apply_handoff(&pool, request)
        .await
        .map_err(error_response)?;
    let status = match response.status {
        HandoffStatus::PendingDispatch => StatusCode::ACCEPTED,
        _ => StatusCode::OK,
    };
    Ok((status, Json(response)))
}

async fn register(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProgramRegistrationRequest>,
) -> Result<(StatusCode, Json<ProgramRegistrationResponse>), (StatusCode, Json<ProgramError>)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|error| error_response(ProgramHandoffError::Storage(error.to_string())))?;
    let response = register_program(&pool, request)
        .await
        .map_err(error_response)?;
    Ok((StatusCode::OK, Json(response)))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/program/handoff", post(handoff))
        .route("/api/program/register", post(register))
        .with_state(state)
}
