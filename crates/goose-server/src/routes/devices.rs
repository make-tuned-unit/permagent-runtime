//! Device registry routes (#628): per-device pairing tokens.
//!
//! Protected (bearer — master or device token):
//! - `GET  /api/devices`              — list devices (NEVER token material)
//! - `POST /api/devices/pair`         — mint a one-time claim code for a named
//!   device; the pairing URL carries `#claim=<code>` instead of a raw token
//! - `PATCH /api/devices/{id}`        — rename
//! - `POST /api/devices/{id}/revoke`  — revoke (persisted immediately)
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
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::device_registry::DeviceView;
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
        .with_state(state)
}

/// The public claim exchange (the claiming device has no credential yet).
pub fn public_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/pair/claim", post(claim))
        .with_state(state)
}
