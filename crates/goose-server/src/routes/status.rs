use axum::body::Body;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::response::IntoResponse;
use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
use permagent::session::{generate_diagnostics, get_system_info, SystemInfo};
use std::sync::Arc;

use crate::state::AppState;

#[utoipa::path(get, path = "/status",
    responses(
        (status = 200, description = "ok", body = String),
    )
)]
async fn status() -> String {
    "ok".to_string()
}

#[utoipa::path(get, path = "/system_info",
    responses(
        (status = 200, description = "System information", body = SystemInfo),
    )
)]
async fn system_info() -> Json<SystemInfo> {
    Json(get_system_info())
}

#[utoipa::path(get, path = "/diagnostics/{session_id}",
    responses(
        (status = 200, description = "Diagnostics zip file", content_type = "application/zip", body = Vec<u8>),
        (status = 500, description = "Failed to generate diagnostics"),
    )
)]
async fn diagnostics(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match generate_diagnostics(state.session_manager(), &session_id).await {
        Ok(zip_data) => {
            let filename = format!("attachment; filename=\"diagnostics_{}.zip\"", session_id);
            let headers = [
                (
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/zip"),
                ),
                (
                    http::header::CONTENT_DISPOSITION,
                    HeaderValue::from_str(&filename).map_err(|_e| StatusCode::BAD_REQUEST)?,
                ),
            ];

            Ok((headers, Body::from(zip_data)))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// GET /api/tailnet/status — deterministic Tailscale detection for the
/// Devices pairing surface (MULTI_DEVICE.md). Tries the CLI in PATH, then the
/// macOS app bundle binary. Never errors: absence is a state, not a failure.
async fn tailnet_status() -> Json<serde_json::Value> {
    let candidates = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    for bin in candidates {
        let out = tokio::process::Command::new(bin)
            .args(["status", "--json"])
            .output()
            .await;
        let Ok(out) = out else { continue };
        if !out.status.success() {
            // Installed but not up/logged in.
            return Json(serde_json::json!({
                "installed": true, "running": false,
                "magic_dns_name": null, "ips": [],
            }));
        }
        let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        let dns = parsed
            .pointer("/Self/DNSName")
            .and_then(|v| v.as_str())
            .map(|d| d.trim_end_matches('.').to_string());
        let ips = parsed
            .pointer("/Self/TailscaleIPs")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        let running = parsed
            .pointer("/BackendState")
            .and_then(|v| v.as_str())
            .map(|st| st == "Running")
            .unwrap_or(false);
        return Json(serde_json::json!({
            "installed": true, "running": running,
            "magic_dns_name": dns, "ips": ips,
        }));
    }
    Json(serde_json::json!({
        "installed": false, "running": false,
        "magic_dns_name": null, "ips": [],
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/system_info", get(system_info))
        // /api-prefixed alias (#381): the wizard's hardware scan calls the
        // modern /api surface (also what the vite dev proxy forwards).
        .route("/api/system_info", get(system_info))
        .route("/api/tailnet/status", get(tailnet_status))
        .with_state(state)
}

/// Bearer-protected: the diagnostics zip bundles daemon logs, config.yaml and
/// system info for an arbitrary session id — an exfiltration surface when
/// public (C2-class). Registered under the protected router in `routes::configure`.
pub fn protected_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/diagnostics/{session_id}", get(diagnostics))
        .with_state(state)
}
