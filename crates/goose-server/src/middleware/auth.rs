//! Bearer token validation middleware.
//!
//! Validates the `Authorization: Bearer <token>` header against
//! `AppState::daemon_token`. Returns 401 on missing or invalid token.
//!
//! Security posture (C1/C2 launch audit):
//! - FAIL-CLOSED: if the daemon has no token (`daemon_token == None`, i.e. the
//!   secrets file could not be created/read — a broken install), protected
//!   routes refuse with 503 instead of allowing anonymous access (H3).
//! - Constant-time comparison via `subtle::ct_eq`, mirroring the voice WS.
//! - `require_token_header_or_query` additionally accepts `?token=` for the
//!   streaming endpoints whose browser clients cannot set headers
//!   (`EventSource` for the per-session SSE, `WebSocket` for `/events`).
//!   The access log never emits query strings (see `access_log.rs`), so a
//!   query-borne token does not leak into daemon logs.

use std::sync::Arc;

use axum::{
    extract::{Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;

use crate::state::AppState;

/// Query shape for token-in-URL clients (EventSource / WebSocket).
/// Unknown params are ignored, so this composes with e.g. `?last_event_id=`.
#[derive(Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

/// Pure validation core, shared by every auth path (HTTP middleware, the
/// `/events` WS upgrade, the voice WS upgrade). FAIL-CLOSED by construction:
///
/// - `expected == None` (no daemon token exists) → `503 SERVICE_UNAVAILABLE`.
///   A tokenless daemon is a broken install, not an open one.
/// - `provided` missing or mismatched → `401 UNAUTHORIZED`.
/// - Comparison is constant-time (`subtle`), so the token cannot be recovered
///   byte-by-byte through timing.
pub fn validate_token_value(
    expected: Option<&str>,
    provided: Option<&str>,
) -> Result<(), StatusCode> {
    let expected = expected.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    match provided {
        Some(t) if bool::from(subtle::ConstantTimeEq::ct_eq(t.as_bytes(), expected.as_bytes())) => {
            Ok(())
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Validate against the daemon token in `AppState` (fail-closed, see
/// [`validate_token_value`]).
pub fn validate_daemon_token(state: &AppState, provided: Option<&str>) -> Result<(), StatusCode> {
    validate_token_value(state.daemon_token.as_deref(), provided)
}

/// Extract the token from an `Authorization: Bearer <token>` header, if any.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Standard bearer-header middleware for the protected router.
pub async fn require_bearer_token(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    validate_daemon_token(&state, bearer_token(request.headers()))?;
    Ok(next.run(request).await)
}

/// Token middleware for streaming endpoints: accepts the bearer header
/// (native clients — the CLI's `activity tail` sends it on the WS handshake)
/// OR `?token=` (browser `EventSource`/`WebSocket`, which cannot set
/// headers). Same fail-closed, constant-time core as the bearer middleware.
pub async fn require_token_header_or_query(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TokenQuery>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let header = bearer_token(request.headers());
    let provided = header.or(query.token.as_deref());
    validate_daemon_token(&state, provided)?;
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_token_passes() {
        assert!(validate_token_value(Some("secret"), Some("secret")).is_ok());
    }

    #[test]
    fn wrong_token_is_401() {
        assert_eq!(
            validate_token_value(Some("secret"), Some("wrong")),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn missing_token_is_401() {
        assert_eq!(
            validate_token_value(Some("secret"), None),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn no_configured_token_fails_closed_503() {
        // H3: a daemon with no token must REFUSE, not allow-through — with or
        // without a caller-provided credential.
        assert_eq!(
            validate_token_value(None, None),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(
            validate_token_value(None, Some("anything")),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    #[test]
    fn prefix_and_length_variants_are_401() {
        // ct_eq over differing lengths must still reject (subtle handles this).
        assert_eq!(
            validate_token_value(Some("secret"), Some("secret-longer")),
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            validate_token_value(Some("secret"), Some("sec")),
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(
            validate_token_value(Some("secret"), Some("")),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn bearer_header_parsing() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer abc123".parse().unwrap());
        assert_eq!(bearer_token(&headers), Some("abc123"));

        let mut wrong_scheme = HeaderMap::new();
        wrong_scheme.insert("authorization", "Basic abc123".parse().unwrap());
        assert_eq!(bearer_token(&wrong_scheme), None);

        assert_eq!(bearer_token(&HeaderMap::new()), None);
    }
}
