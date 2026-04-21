use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

pub async fn check_token(
    State(_state): State<String>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Phase 1: localhost-only, no auth required
    Ok(next.run(request).await)
}
