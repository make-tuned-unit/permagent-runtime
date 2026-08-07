//! Desktop presence and remote wake.
//!
//! The daemon is launchd-managed and fully independent of the desktop app:
//! it starts at login (`RunAtLoad`), restarts on failure, and survives the app
//! quitting. So a phone talks to a live hub whether or not anything is open on
//! the Mac — chat, voice, notes, projects, schedules and decisions all work.
//!
//! What does NOT work app-closed is anything that needs the UI process: the
//! in-app browser and the terminal ride the event bus to a frontend that must
//! be listening. Those now refuse honestly (`events::has_listeners`), and this
//! module gives the phone the two things it needs to recover: ask whether the
//! desktop is attached, and wake it.
//!
//! Waking is `open -a Permagent` — the same idiom the Ollama one-click setup
//! uses. It requires the Mac to be awake; a sleeping machine cannot be started
//! from here (that is Wake-on-LAN's job, out of scope), and we say so rather
//! than reporting a launch that did not happen.

use axum::{routing::get, routing::post, Json, Router};
use std::sync::Arc;

use crate::state::AppState;

/// Is a desktop UI attached to the event bus right now?
fn ui_attached() -> bool {
    permagent::events::has_listeners()
}

/// GET /api/desktop/status — what the phone needs to decide whether to offer
/// a wake, and what to tell the user about which features are reachable.
async fn desktop_status() -> Json<serde_json::Value> {
    let attached = ui_attached();
    Json(serde_json::json!({
        "ui_attached": attached,
        // Stated rather than implied: the honest capability line for this
        // moment, so the phone never has to guess what will work.
        "available_without_ui": [
            "chat", "voice", "notes", "projects", "todos", "schedules", "decisions", "memory"
        ],
        "needs_ui": ["in_app_browser", "terminal"],
        "summary": if attached {
            "The desktop app is open — everything is available."
        } else {
            "The desktop app is closed. Chat, voice, notes, projects and automations all \
             work; the in-app browser and terminal need the app open."
        },
    }))
}

/// POST /api/desktop/launch — wake the desktop app on this Mac.
///
/// Idempotent: launching an already-running app just focuses it, so the phone
/// may call this without checking first.
async fn desktop_launch() -> Json<serde_json::Value> {
    if ui_attached() {
        return Json(serde_json::json!({
            "launched": false,
            "already_running": true,
            "message": "The desktop app is already open."
        }));
    }

    #[cfg(target_os = "macos")]
    {
        match tokio::process::Command::new("open")
            .args(["-a", "Permagent"])
            .status()
            .await
        {
            Ok(status) if status.success() => {
                tracing::info!(target: "permagentd::desktop", "desktop app launched remotely");
                return Json(serde_json::json!({
                    "launched": true,
                    "already_running": false,
                    // The UI takes a moment to boot and attach to the event
                    // bus; the caller should poll status rather than assume.
                    "message": "Launching the desktop app — give it a few seconds to come up."
                }));
            }
            Ok(status) => {
                tracing::warn!(
                    target: "permagentd::desktop",
                    "desktop launch failed: open exited {status}"
                );
                return Json(serde_json::json!({
                    "launched": false,
                    "already_running": false,
                    "message": "Couldn't launch the app — is the Mac awake and Permagent \
                                installed in /Applications?"
                }));
            }
            Err(e) => {
                tracing::warn!(target: "permagentd::desktop", "desktop launch error: {e}");
                return Json(serde_json::json!({
                    "launched": false,
                    "already_running": false,
                    "message": format!("Couldn't launch the app: {e}")
                }));
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    Json(serde_json::json!({
        "launched": false,
        "already_running": false,
        "message": "Remote launch is implemented for macOS only."
    }))
}

pub fn routes(_state: Arc<AppState>) -> Router {
    // LOOPBACK-ONLY. `/api/desktop/launch` spawns a process, and these routes
    // sit on the unauthenticated public rail alongside the browser bridges —
    // which each carry this same guard for the same reason. Without it, any
    // page or process that can reach the daemon's port could spawn the app.
    // Remote use (the phone over tailscale serve) reaches this through the
    // authenticated agent tool, not directly.
    Router::new()
        .route("/api/desktop/status", get(desktop_status))
        .route("/api/desktop/launch", post(desktop_launch))
        .layer(axum::middleware::from_fn(
            crate::middleware::loopback::require_loopback,
        ))
}
