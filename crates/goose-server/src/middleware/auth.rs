//! Bearer token validation middleware.
//!
//! Validates the `Authorization: Bearer <token>` header against
//! `AppState::daemon_token` OR any non-revoked per-device token in the
//! device registry (#628). Returns 401 on missing or invalid token.
//!
//! Security posture (C1/C2 launch audit):
//! - FAIL-CLOSED: if the daemon has no token (`daemon_token == None`, i.e. the
//!   secrets file could not be created/read — a broken install), protected
//!   routes refuse with 503 instead of allowing anonymous access (H3). This
//!   holds even when device tokens exist: a hub without its master token is a
//!   broken install, not a degraded one.
//! - Constant-time comparison via `subtle::ct_eq`, mirroring the voice WS.
//!   Device tokens are checked as SHA-256 digests, ct_eq'd against every
//!   registered hash with no early exit (see `DeviceRegistry::verify`); the
//!   master check and the device scan both run before the verdict, so timing
//!   does not reveal which class of token (if any) matched.
//! - A device-token match records last-seen on the registry (in-memory always,
//!   persisted throttled — never a write per request).
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

use crate::device_registry::DeviceRegistry;
use crate::state::AppState;

/// Which credential admitted a request.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthPrincipal {
    /// The master `daemon_token` (the hub's own app — legacy single token).
    Master,
    /// A per-device pairing token; carries the device id for last-seen.
    Device(String),
}

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
        Some(t)
            if bool::from(subtle::ConstantTimeEq::ct_eq(
                t.as_bytes(),
                expected.as_bytes(),
            )) =>
        {
            Ok(())
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Validation core for master-OR-device credentials (#628). FAIL-CLOSED like
/// [`validate_token_value`]: no master token → 503 (a broken install refuses,
/// device tokens notwithstanding). Both the master compare and the full device
/// scan execute before the verdict — no short-circuit — so timing does not
/// reveal which class matched. On a device match, returns the device id so
/// callers can record last-seen.
pub fn validate_with_devices(
    expected_master: Option<&str>,
    registry: &DeviceRegistry,
    provided: Option<&str>,
) -> Result<AuthPrincipal, StatusCode> {
    let expected = expected_master.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let provided = provided.ok_or(StatusCode::UNAUTHORIZED)?;
    let master_ok = bool::from(subtle::ConstantTimeEq::ct_eq(
        provided.as_bytes(),
        expected.as_bytes(),
    ));
    // Always run the device scan (constant-time over the whole set inside).
    let device_match = registry.verify(provided);
    if master_ok {
        return Ok(AuthPrincipal::Master);
    }
    if let Some(id) = device_match {
        return Ok(AuthPrincipal::Device(id));
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Validate against the daemon token in `AppState` (fail-closed, see
/// [`validate_token_value`]) or any non-revoked device token (#628). Shared by
/// every auth path — the HTTP middlewares below, the `/events` WS upgrade, and
/// the voice WS upgrade — so per-device tokens work on all rails. A device
/// match touches the registry's last-seen (throttled persistence).
pub fn validate_daemon_token(state: &AppState, provided: Option<&str>) -> Result<(), StatusCode> {
    match validate_with_devices(
        state.daemon_token.as_deref(),
        &state.device_registry,
        provided,
    )? {
        AuthPrincipal::Master => {}
        AuthPrincipal::Device(id) => state.device_registry.touch(&id),
    }
    Ok(())
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

    // ── #628: master-OR-device validation core ──

    fn temp_registry() -> (tempfile::TempDir, DeviceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device_tokens.json");
        (dir, DeviceRegistry::load(path))
    }

    #[test]
    fn legacy_master_token_still_admits() {
        let (_dir, reg) = temp_registry();
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some("master")),
            Ok(AuthPrincipal::Master),
            "zero-breakage: the single daemon_token keeps working"
        );
    }

    #[test]
    fn valid_device_token_admits_with_device_id() {
        let (_dir, reg) = temp_registry();
        let (token, view) = reg.pair("iPhone");
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some(&token)),
            Ok(AuthPrincipal::Device(view.id)),
        );
    }

    #[test]
    fn revoked_device_token_is_401() {
        let (_dir, reg) = temp_registry();
        let (token, view) = reg.pair("Lost Phone");
        reg.revoke(&view.id).unwrap();
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some(&token)),
            Err(StatusCode::UNAUTHORIZED),
        );
    }

    #[test]
    fn unknown_token_is_401_with_devices_registered() {
        let (_dir, reg) = temp_registry();
        reg.pair("Real Device");
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some("intruder")),
            Err(StatusCode::UNAUTHORIZED),
        );
        assert_eq!(
            validate_with_devices(Some("master"), &reg, None),
            Err(StatusCode::UNAUTHORIZED),
        );
    }

    #[test]
    fn no_master_token_fails_closed_503_even_with_valid_device_token() {
        // H3 preserved: a hub without its master token is a broken install —
        // refuse everything, device tokens notwithstanding.
        let (_dir, reg) = temp_registry();
        let (token, _) = reg.pair("iPhone");
        assert_eq!(
            validate_with_devices(None, &reg, Some(&token)),
            Err(StatusCode::SERVICE_UNAVAILABLE),
        );
        assert_eq!(
            validate_with_devices(None, &reg, None),
            Err(StatusCode::SERVICE_UNAVAILABLE),
        );
    }

    #[test]
    fn device_token_prefix_variants_are_401() {
        // ct over sha256 digests is inherently length-independent; prove the
        // observable contract anyway.
        let (_dir, reg) = temp_registry();
        let (token, _) = reg.pair("iPad");
        let truncated = &token[..token.len() - 2];
        let extended = format!("{token}00");
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some(truncated)),
            Err(StatusCode::UNAUTHORIZED),
        );
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some(&extended)),
            Err(StatusCode::UNAUTHORIZED),
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
