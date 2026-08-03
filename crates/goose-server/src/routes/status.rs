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

/// GET /api/tailnet/access — is this hub reachable by your other devices?
///
/// Reports the state of `tailscale serve`, NOT a bind address. The daemon
/// deliberately stays on localhost: Tailscale's own security guidance is that a
/// backend listening on 0.0.0.0 (or even directly on the tailnet IP) can have
/// its identity headers spoofed by any peer that reaches it, so the backend
/// should be localhost-only and fronted by Serve. Serve also means nothing has
/// to change when the machine's tailnet IP changes — there is no address stored
/// anywhere to go stale.
async fn tailnet_access_get() -> Json<serde_json::Value> {
    let serve = read_serve_state().await;
    Json(serde_json::json!({
        "enabled": serve.is_some(),
        "serve_url": serve,
        "available": tailscale_bin().is_some(),
    }))
}

#[derive(serde::Deserialize)]
struct TailnetAccessRequest {
    enabled: bool,
}

/// PUT /api/tailnet/access — turn remote reachability on or off.
///
/// On: `tailscale serve --bg --http=80 <port>` publishes the loopback daemon to
/// the tailnet and nowhere else. Off: `tailscale serve --http=80 off`. Both are
/// idempotent and instant — no daemon restart, because the daemon's own bind
/// never changes.
async fn tailnet_access_put(
    Json(req): Json<TailnetAccessRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let bin = tailscale_bin().ok_or((
        StatusCode::PRECONDITION_FAILED,
        "Tailscale is not installed".to_string(),
    ))?;
    let port = std::env::var("PORT").unwrap_or_else(|_| "3001".to_string());

    let args: Vec<String> = if req.enabled {
        vec!["serve".into(), "--bg".into(), "--http=80".into(), port]
    } else {
        vec!["serve".into(), "--http=80".into(), "off".into()]
    };

    let out = tokio::process::Command::new(&bin)
        .args(&args)
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            if msg.is_empty() {
                "tailscale serve failed".into()
            } else {
                msg
            },
        ));
    }

    let serve = read_serve_state().await;
    Ok(Json(serde_json::json!({
        "enabled": serve.is_some(),
        "serve_url": serve,
        "available": true,
    })))
}

fn tailscale_bin() -> Option<String> {
    for bin in [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        if std::process::Command::new(bin)
            .arg("version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(bin.to_string());
        }
    }
    None
}

/// The URL Serve is publishing this daemon on, or None when Serve is off.
/// Parsed from `tailscale serve status --json` rather than the human output,
/// which is formatted for reading and changes between releases.
async fn read_serve_state() -> Option<String> {
    let bin = tailscale_bin()?;
    let out = tokio::process::Command::new(bin)
        .args(["serve", "status", "--json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    // Web handlers are keyed "<host>:<port>"; presence of any handler means
    // Serve is publishing something for this node.
    let web = parsed.get("Web")?.as_object()?;
    let key = web.keys().next()?;
    let host = key.split(':').next()?;
    let port = key.rsplit(':').next().unwrap_or("80");
    Some(if port == "443" {
        format!("https://{host}")
    } else if port == "80" {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/system_info", get(system_info))
        // /api-prefixed alias (#381): the wizard's hardware scan calls the
        // modern /api surface (also what the vite dev proxy forwards).
        .route("/api/system_info", get(system_info))
        .route("/api/tailnet/status", get(tailnet_status))
        .route(
            "/api/tailnet/access",
            get(tailnet_access_get).put(tailnet_access_put),
        )
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
