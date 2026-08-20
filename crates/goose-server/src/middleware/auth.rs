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
//! - `require_token_header_or_query` additionally accepts scoped `?token=`
//!   credentials for streaming endpoints whose browser clients cannot set
//!   headers (`EventSource` for the per-session SSE, `WebSocket` for `/events`).
//! - `require_daemon_token_header_or_query` accepts the established daemon or
//!   device credential via either transport, but never scoped stream tokens.
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

use permagent::security::auth_audit::{self, AuthEventRecord, AuthOutcome, CredentialKind};

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
    // One constant-time compare core for the master token — the same
    // `validate_token_value` every pre-#628 call path used.
    let master_ok = validate_token_value(Some(expected), Some(provided)).is_ok();
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

/// Authenticate a long-lived daemon/device credential and return the principal
/// downstream code may use for audit attribution.
///
/// Validates against the daemon token in `AppState` (fail-closed, see
/// [`validate_token_value`]) or any non-revoked device token (#628). Shared by
/// every auth path — the HTTP middlewares below, the `/events` WS upgrade and
/// the voice WS upgrade (both via [`validate_stream_token`]) — so per-device
/// tokens work on all rails. A device match touches the registry's last-seen
/// (throttled persistence).
pub fn authenticate_daemon_token(
    state: &AppState,
    provided: Option<&str>,
) -> Result<AuthPrincipal, StatusCode> {
    let principal = validate_with_devices(
        state.daemon_token.as_deref(),
        &state.device_registry,
        provided,
    )?;
    match &principal {
        AuthPrincipal::Master => {}
        AuthPrincipal::Device(id) => state.device_registry.touch(id),
    }
    Ok(principal)
}

/// Validate a browser stream credential without changing the established
/// daemon/device-token path. A missing daemon token remains a hard 503. Only
/// after the existing validator returns 401 may a query-borne scoped token
/// admit the request.
pub fn validate_stream_token(
    state: &AppState,
    existing_provided: Option<&str>,
    query_token: Option<&str>,
) -> Result<(), StatusCode> {
    authenticate_stream_token(state, existing_provided, query_token).map(|_| ())
}

/// Which credential admitted a request on a stream rail.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamPrincipal {
    /// A long-lived daemon or device credential.
    Long(AuthPrincipal),
    /// A short-lived, stream-scoped token minted by `/sse-token`.
    Scoped,
}

/// Authenticating core behind [`validate_stream_token`], returning the
/// admitting credential so the auth audit can attribute the request.
///
/// This is the SINGLE implementation of the stream-rail decision; the
/// unit-returning `validate_stream_token` delegates to it. Keeping one body
/// means the audit cannot drift away from the decision it claims to record.
/// The decision itself is unchanged: a missing daemon token is still a hard
/// 503, and only after the established validator returns 401 may a query-borne
/// scoped token admit the request.
pub fn authenticate_stream_token(
    state: &AppState,
    existing_provided: Option<&str>,
    query_token: Option<&str>,
) -> Result<StreamPrincipal, StatusCode> {
    match authenticate_daemon_token(state, existing_provided) {
        Ok(principal) => Ok(StreamPrincipal::Long(principal)),
        Err(StatusCode::UNAUTHORIZED)
            if query_token.is_some_and(|token| state.stream_tokens.contains_unexpired(token)) =>
        {
            Ok(StreamPrincipal::Scoped)
        }
        Err(status) => Err(status),
    }
}

/// Extract the token from an `Authorization: Bearer <token>` header, if any.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

// ── Auth audit (#trust-boundary) ─────────────────────────────────────────────
//
// The daemon token is a 0600 file, which separates other USERS from it but not
// other PROCESSES running as this user. Same-user misuse cannot be prevented
// here, so it is recorded instead: every refusal, and every admitted request
// whose route can execute code, touch secrets, spend money or write user data.
// Reads and status polls are not recorded on success — see
// `RouteClass::is_audited_on_success`. This is detection, not prevention; the
// limits are stated in `docs/design/daemon-trust-boundary.md`.

/// The audit labels for an admitted principal: (principal, credential kind).
fn principal_labels(principal: &AuthPrincipal) -> (String, CredentialKind) {
    match principal {
        AuthPrincipal::Master => ("master".to_string(), CredentialKind::Master),
        AuthPrincipal::Device(id) => (id.clone(), CredentialKind::Device),
    }
}

/// What a refused caller presented. A wrong token and no token at all are
/// different events: only one of them is somebody trying.
fn denied_credential(provided: Option<&str>) -> CredentialKind {
    match provided {
        Some(_) => CredentialKind::Unrecognised,
        None => CredentialKind::None,
    }
}

/// Write one audit row, subject to the class policy. Never fails the request:
/// `record_auth_event` logs loudly and swallows, because a full disk must not
/// become a total loss of access to the user's own daemon.
async fn audit(
    outcome: AuthOutcome,
    principal: Option<String>,
    credential: CredentialKind,
    method: &str,
    path: &str,
    status: Option<u16>,
) {
    let class = auth_audit::classify(method, path);
    if outcome == AuthOutcome::Admitted && !class.is_audited_on_success() {
        return;
    }
    auth_audit::record_auth_event(AuthEventRecord {
        outcome,
        principal,
        credential,
        class,
        method: method.to_string(),
        // Path only, never the query string: the long-lived token rides
        // `?token=` on the SSE and WebSocket rails.
        path: path.to_string(),
        status,
        peer: None,
    })
    .await;
}

/// Record a refusal on any auth rail.
async fn audit_denied(provided: Option<&str>, method: &str, path: &str, status: StatusCode) {
    audit(
        AuthOutcome::Denied,
        None,
        denied_credential(provided),
        method,
        path,
        Some(status.as_u16()),
    )
    .await;
}

/// Standard bearer-header middleware for the protected router.
pub async fn require_bearer_token(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let provided = bearer_token(request.headers()).map(str::to_owned);

    // The authentication decision itself is UNCHANGED — same validator, same
    // fail-closed 503, same constant-time compare. Only the recording is new.
    let principal = match authenticate_daemon_token(&state, provided.as_deref()) {
        Ok(principal) => principal,
        Err(status) => {
            audit_denied(provided.as_deref(), &method, &path, status).await;
            return Err(status);
        }
    };

    let (label, credential) = principal_labels(&principal);
    request.extensions_mut().insert(principal);
    let response = next.run(request).await;
    audit(
        AuthOutcome::Admitted,
        Some(label),
        credential,
        &method,
        &path,
        Some(response.status().as_u16()),
    )
    .await;
    Ok(response)
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
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let header = bearer_token(request.headers());
    let provided = header.or(query.token.as_deref());
    // Unchanged decision, evaluated exactly once; the scoped-token fallback
    // still only applies after the established validator has returned 401.
    let (label, credential) =
        match authenticate_stream_token(&state, provided, query.token.as_deref()) {
            Ok(StreamPrincipal::Long(ref principal)) => principal_labels(principal),
            Ok(StreamPrincipal::Scoped) => ("stream".to_string(), CredentialKind::Stream),
            Err(status) => {
                let owned = provided.map(str::to_owned);
                audit_denied(owned.as_deref(), &method, &path, status).await;
                return Err(status);
            }
        };
    // Admitted. These are the stream rails, whose routes classify as reads, so
    // the class policy normally drops the row; keep the call so a future
    // audited-class route mounted here is recorded without further wiring.
    let response = next.run(request).await;
    audit(
        AuthOutcome::Admitted,
        Some(label),
        credential,
        &method,
        &path,
        Some(response.status().as_u16()),
    )
    .await;
    Ok(response)
}

/// Token middleware for endpoints whose native and browser clients use either
/// the bearer header or `?token=`, but which require a long-lived daemon/device
/// credential. Scoped stream tokens are intentionally not considered.
pub async fn require_daemon_token_header_or_query(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TokenQuery>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let header = bearer_token(request.headers());
    let provided = header.or(query.token.as_deref()).map(str::to_owned);
    // Unchanged decision: scoped stream tokens are still never admitted here.
    let principal = match authenticate_daemon_token(&state, provided.as_deref()) {
        Ok(principal) => principal,
        Err(status) => {
            audit_denied(provided.as_deref(), &method, &path, status).await;
            return Err(status);
        }
    };
    let (label, credential) = principal_labels(&principal);
    let response = next.run(request).await;
    audit(
        AuthOutcome::Admitted,
        Some(label),
        credential,
        &method,
        &path,
        Some(response.status().as_u16()),
    )
    .await;
    Ok(response)
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
        let mut truncated = token.clone();
        truncated.truncate(token.len() - 2);
        let extended = format!("{token}00");
        assert_eq!(
            validate_with_devices(Some("master"), &reg, Some(truncated.as_str())),
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

    #[test]
    fn stream_token_store_accepts_fresh_and_rejects_expired_tokens() {
        let store = crate::state::StreamTokenStore::default();
        store.insert(
            "fresh".to_string(),
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        );
        store.insert(
            "expired".to_string(),
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        );

        assert!(store.contains_unexpired("fresh"));
        assert!(!store.contains_unexpired("expired"));
        assert!(!store.contains_unexpired("unknown"));
    }
}
