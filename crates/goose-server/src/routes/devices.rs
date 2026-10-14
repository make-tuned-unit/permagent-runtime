//! Device registry routes (#628): per-device pairing tokens.
//!
//! Protected (bearer — master or device token):
//! - `GET  /api/devices`              — list devices (NEVER token material)
//! - `POST /api/devices/pair`         — mint a one-time claim code for a named
//!   device; the pairing URL carries `#claim=<code>` instead of a raw token
//! - `PATCH /api/devices/{id}`        — rename
//! - `POST /api/devices/{id}/revoke`  — revoke (persisted immediately)
//! - `POST /api/devices/{id}/watch-credential` — a paired phone (or the master
//!   token) mints a `voice_only` credential for its own watch: a separate
//!   device identity named "<phone name> Watch", 30-day expiry, revocable
//!   like any device, accepted ONLY by the `/voice` socket. Returns the raw
//!   token once, plus the LAN gateway URL and certificate pin. Re-minting
//!   rotates (revokes the previous watch credential).
//!
//! Public:
//! - `POST /pair/claim` — exchange a claim code for a fresh device token. The
//!   claiming companion holds no credential yet, so this endpoint cannot be
//!   bearer-protected; safety comes from the code being 128-bit random,
//!   single-use, and short-lived (10 min), plus the origin guard that fronts
//!   every route. The response is the ONE place a device token value ever
//!   appears; it is never echoed by any list endpoint.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::device_registry::{DeviceScope, DeviceView};
use crate::middleware::auth::AuthPrincipal;
use crate::state::AppState;

#[derive(Deserialize)]
struct PairRequest {
    name: String,
}

#[derive(Serialize)]
struct PairResponse {
    /// One-time claim code — rides the pairing URL as `#claim=<code>`.
    claim_code: String,
    /// RFC 3339 expiry of the code.
    expires_at: String,
}

#[derive(Deserialize)]
struct RenameRequest {
    name: String,
}

#[derive(Deserialize)]
struct ClaimRequest {
    code: String,
}

#[derive(Serialize)]
struct ClaimResponse {
    /// The freshly minted device bearer token. Shown ONCE — never stored in
    /// clear, never listed again.
    token: String,
    device: DeviceView,
}

#[derive(Deserialize, Default)]
struct WatchCredentialRequest {
    /// Shared conversation to bind the credential to. Omitted = the watch
    /// claims the first `session_id` it opens.
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Serialize)]
struct WatchCredentialResponse {
    /// `wss://host:port` of the LAN voice gateway (path `/voice`).
    hub_url_lan: String,
    /// The `voice_only` bearer token. Shown ONCE; never listed again.
    voice_token: String,
    conversation_id: Option<String>,
    /// RFC 3339 expiry.
    expires_at: String,
    /// SHA-256 of the gateway's leaf certificate (colon-separated hex) for pinning.
    cert_sha256: String,
    device: DeviceView,
}

/// Who may mint a watch credential for device `target`: the master token, or
/// the device itself (a phone mints for ITS OWN watch only).
pub(crate) fn watch_mint_allowed(principal: &AuthPrincipal, target: &str) -> bool {
    match principal {
        AuthPrincipal::Master => true,
        AuthPrincipal::Device(id) => id == target,
    }
}

async fn mint_watch_credential(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<WatchCredentialResponse>, StatusCode> {
    if !watch_mint_allowed(&principal, &id) {
        return Err(StatusCode::FORBIDDEN);
    }
    let req: WatchCredentialRequest = if body.is_empty() {
        WatchCredentialRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let conversation_id = match req.conversation_id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(c) => Some(
            // Same shape rule `/voice` enforces, so a bad id fails here, not later.
            crate::voice_gateway::authorize_session(None, Some(c))
                .map_err(|_| StatusCode::BAD_REQUEST)?,
        ),
    };
    let settings = crate::voice_gateway::settings();
    let facts =
        crate::voice_gateway::provisioning(&settings, &crate::voice_gateway::default_cert_dir())
            // Gateway off / no LAN host: nothing reachable to hand the watch.
            .map_err(|_| StatusCode::CONFLICT)?;
    let ttl = chrono::Duration::days(settings.credential_ttl_days.clamp(1, 365));
    let (voice_token, device) = state
        .device_registry
        .mint_voice_credential(&id, conversation_id.as_deref(), ttl, chrono::Utc::now())
        .ok_or(StatusCode::NOT_FOUND)?;
    tracing::info!(
        target: "permagentd::voice_gateway",
        event = "watch_credential_minted",
        phone = id.as_str(),
        watch = device.id.as_str(),
        "watch credential minted (token not logged)"
    );
    Ok(Json(WatchCredentialResponse {
        hub_url_lan: facts.hub_url_lan,
        voice_token,
        conversation_id: device.conversation_id.clone(),
        expires_at: device.expires_at.clone().unwrap_or_default(),
        cert_sha256: facts.cert_sha256,
        device,
    }))
}

async fn list_devices(State(state): State<Arc<AppState>>) -> Json<Vec<DeviceView>> {
    Json(state.device_registry.list())
}

async fn create_pairing(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PairRequest>,
) -> Result<Json<PairResponse>, StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (claim_code, expires_at) = state.device_registry.create_claim(name);
    Ok(Json(PairResponse {
        claim_code,
        expires_at: expires_at.to_rfc3339(),
    }))
}

async fn rename_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Result<Json<DeviceView>, StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .device_registry
        .rename(&id, name)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn revoke_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeviceView>, StatusCode> {
    state
        .device_registry
        .revoke(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn claim(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ClaimRequest>,
) -> Result<Json<ClaimResponse>, StatusCode> {
    // Unknown and expired codes answer identically (404): no oracle for
    // whether a guessed code once existed.
    state
        .device_registry
        .claim(&body.code)
        .map(|(token, device)| Json(ClaimResponse { token, device }))
        .ok_or(StatusCode::NOT_FOUND)
}

/// Bearer-protected registry management (mounted under the auth middleware).
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/devices", get(list_devices))
        .route("/api/devices/pair", post(create_pairing))
        .route("/api/devices/{id}", patch(rename_device))
        .route("/api/devices/{id}/revoke", post(revoke_device))
        .route(
            "/api/devices/{id}/watch-credential",
            post(mint_watch_credential),
        )
        .with_state(state)
}

/// The public claim exchange (the claiming device has no credential yet).
pub fn public_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/pair/claim", post(claim))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_mints_only_for_itself_master_for_anyone() {
        let phone = AuthPrincipal::Device("phone-1".into());
        assert!(watch_mint_allowed(&phone, "phone-1"));
        assert!(!watch_mint_allowed(&phone, "phone-2"));
        assert!(watch_mint_allowed(&AuthPrincipal::Master, "phone-2"));
    }

    #[test]
    fn device_view_lists_scope_without_token_material() {
        let dir = tempfile::tempdir().unwrap();
        let reg = crate::device_registry::DeviceRegistry::load(dir.path().join("d.json"));
        let (_t, phone) = reg.pair("Phone");
        let (tok, _w) = reg
            .mint_voice_credential(
                &phone.id,
                Some("c1"),
                chrono::Duration::days(30),
                chrono::Utc::now(),
            )
            .unwrap();
        let listed = serde_json::to_string(&reg.list()).unwrap();
        assert!(listed.contains("\"scope\":\"voice_only\""));
        assert!(listed.contains("\"scope\":\"full\""));
        assert!(!listed.contains(&tok));
        assert!(!listed.contains("token_hash"));
        let _ = DeviceScope::VoiceOnly;
    }
}
