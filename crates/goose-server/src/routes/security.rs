//! Sovereignty controls + the egress audit log.
//!
//! The sovereignty guarantee: with sovereign mode on, all **cloud** inference is
//! blocked (fail-closed) and never happens; every cloud call (blocked or, when
//! mode is off, allowed) is recorded in an append-only local audit log. These
//! endpoints let the UI toggle the boundary and read "everything that has left
//! this machine, and when".
//!
//! Auth is handled by the bearer-token middleware (protected group).
//!
//! Endpoints:
//!   GET  /api/security/sovereignty  — current status
//!   POST /api/security/sovereignty  — set { enabled?, capturePrompts? }
//!   GET  /api/security/egress-log   — recent cloud-egress audit entries (?limit=)

use axum::{
    extract::{Json, Query},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use permagent::config::Config;
use permagent::providers::providers as list_providers;
use permagent::sovereignty::{self, EgressLogEntry};

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SovereigntyStatus {
    /// Global sovereign mode: when on, all cloud inference is blocked + audited.
    enabled: bool,
    /// Whether the audit log captures full prompts (vs a hash only).
    capture_prompts: bool,
    /// Whether a LOCAL inference provider (`local` or `ollama`) is registered,
    /// so sovereign work has somewhere to run rather than only being refused.
    local_provider_available: bool,
}

async fn current_status() -> SovereigntyStatus {
    SovereigntyStatus {
        enabled: sovereignty::global_sovereign_mode(),
        capture_prompts: sovereignty::capture_prompts_enabled(),
        local_provider_available: local_provider_available().await,
    }
}

/// A local provider is present if the built-in in-process `local` provider or
/// `ollama` is registered.
async fn local_provider_available() -> bool {
    list_providers()
        .await
        .iter()
        .any(|(m, _)| m.name == "local" || m.name == "ollama")
}

async fn get_status() -> Json<SovereigntyStatus> {
    Json(current_status().await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSovereigntyRequest {
    enabled: Option<bool>,
    capture_prompts: Option<bool>,
}

async fn set_status(
    Json(req): Json<SetSovereigntyRequest>,
) -> Result<Json<SovereigntyStatus>, (StatusCode, String)> {
    if let Some(enabled) = req.enabled {
        sovereignty::set_global_sovereign_mode(enabled)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(capture) = req.capture_prompts {
        Config::global()
            .set_param(sovereignty::SOVEREIGN_CAPTURE_PROMPTS_KEY, capture)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(Json(current_status().await))
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

async fn get_egress_log(
    Query(params): Query<LogQuery>,
) -> Result<Json<Vec<EgressLogEntry>>, (StatusCode, String)> {
    let limit = params.limit.clamp(1, 1000);
    let entries = sovereignty::recent_egress(limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(entries))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/security/sovereignty", get(get_status))
        .route("/api/security/sovereignty", post(set_status))
        .route("/api/security/egress-log", get(get_egress_log))
        .with_state(state)
}
