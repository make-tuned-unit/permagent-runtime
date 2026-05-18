//! Bearer token validation middleware.
//!
//! Validates the `Authorization: Bearer <token>` header against
//! `AppState::daemon_token`. Returns 401 on missing or invalid token.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use crate::state::AppState;

pub async fn require_bearer_token(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let expected = match state.daemon_token.as_deref() {
        Some(t) => t,
        // No token configured — allow through (dev mode / unconfigured)
        None => return Ok(next.run(request).await),
    };

    let provided = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match provided {
        Some(token) if token == expected => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
