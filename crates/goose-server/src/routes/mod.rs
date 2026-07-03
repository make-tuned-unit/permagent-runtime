pub mod action_required;
pub mod activity;
pub mod agent;
pub mod agents;
pub mod attachments;
pub mod backup;
pub mod brain;
pub mod browser_content;
pub mod cards;
pub mod config_management;
pub mod dashboard;
pub mod decisions;
pub mod errors;
pub mod events;
pub mod features;
pub mod findings;
pub mod gateway;
pub mod henry_status;
pub mod identity;
pub mod inbox;
pub mod integrations;
pub mod librarian;
#[cfg(feature = "local-inference")]
pub mod local_inference;
pub mod ollama;
pub mod people;
pub mod projects;
pub mod prompts;
pub mod reader;
pub mod recipe;
pub mod recipe_utils;
pub mod reply;
pub mod sampling;
pub mod schedule;
pub mod session;
pub mod session_events;
pub mod setup;
pub mod skills;
pub mod status;
pub mod storage;
pub mod telemetry;
pub mod tunnel;
pub mod utils;
pub mod version;
pub mod voice;
pub mod workers;
pub mod workspaces;
pub mod world;

use std::sync::Arc;

use axum::{middleware, Router};
use tower_http::services::{ServeDir, ServeFile};

use crate::middleware::auth::require_bearer_token;

/// Configure all routes, applying bearer token middleware to protected endpoints.
pub fn configure(state: Arc<crate::state::AppState>) -> Router {
    // ── Public routes (no auth required) ──
    let mut public = Router::new()
        .merge(status::routes(state.clone()))
        .merge(version::routes(state.clone()))
        .merge(events::routes(state.clone()))
        .merge(session_events::routes(state.clone()))
        // Browser content bridge: unauthenticated, localhost-only.
        // Called by the in-process MCP tool (no access to daemon token).
        .merge(browser_content::routes(state.clone()))
        // Voice WebSocket: does its own token validation via query param
        // (WebSocket upgrade can't use the Bearer middleware).
        .merge(voice::routes(state.clone()));

    // ── Debug: env-gated panic-injection endpoint (durability #560 dogfood) ──
    // Mounts ONLY when PERMAGENT_DEBUG_PANIC_ENDPOINT=1, so the route does not
    // exist in normal runs and cannot be hit in production. It triggers a genuine
    // panic on the request-handling path to exercise the panic circuit-breaker
    // end-to-end (pair with PERMAGENT_PANIC_BREAKER_MAX=1 so one hit trips it).
    // NOT gated on `debug_assertions`: the dogfood runs the RELEASE binary, which
    // would compile that out — the exact build we must test. Logged loudly on
    // mount so its presence is never silent.
    if std::env::var("PERMAGENT_DEBUG_PANIC_ENDPOINT").as_deref() == Ok("1") {
        tracing::warn!(
            target: "durability",
            "[debug] panic-injection endpoint ENABLED via PERMAGENT_DEBUG_PANIC_ENDPOINT=1 — POST /debug/panic will panic the request handler"
        );
        public = public.route("/debug/panic", axum::routing::post(debug_panic_handler));
    }

    // Static UI assets
    let ui_dir = ui_dist_path();
    if let Some(dir) = ui_dir {
        let index = dir.join("index.html");
        public = public.nest_service("/ui", ServeDir::new(&dir).fallback(ServeFile::new(index)));
    }

    // ── Protected routes (bearer token required) ──
    let mut protected = Router::new()
        .merge(backup::routes(state.clone()))
        .merge(reply::routes(state.clone()))
        .merge(activity::routes(state.clone()))
        .merge(action_required::routes(state.clone()))
        .merge(agent::routes(state.clone()))
        .merge(config_management::routes(state.clone()))
        .merge(prompts::routes())
        .merge(recipe::routes(state.clone()))
        .merge(session::routes(state.clone()))
        .merge(schedule::routes(state.clone()))
        .merge(setup::routes(state.clone()))
        .merge(telemetry::routes(state.clone()))
        .merge(tunnel::routes(state.clone()))
        .merge(gateway::routes(state.clone()))
        .merge(sampling::routes(state.clone()))
        .merge(features::routes())
        .merge(skills::routes(state.clone()))
        .merge(integrations::routes(state.clone()))
        .merge(workspaces::routes(state.clone()))
        .merge(attachments::routes(state.clone()))
        .merge(reader::routes(state.clone()))
        .merge(brain::routes(state.clone()))
        .merge(people::routes(state.clone()))
        .merge(inbox::routes(state.clone()))
        .merge(dashboard::routes(state.clone()))
        .merge(identity::routes(state.clone()))
        .merge(workers::routes(state.clone()))
        .merge(findings::routes(state.clone()))
        .merge(storage::routes(state.clone()))
        .merge(ollama::routes(state.clone()))
        .merge(librarian::routes(state.clone()))
        .merge(henry_status::routes(state.clone()))
        .merge(world::routes(state.clone()))
        .merge(projects::routes(state.clone()))
        .merge(cards::routes(state.clone()))
        .merge(decisions::routes(state.clone()))
        .merge(agents::routes(state.clone()))
        // Voice HTTP endpoints — on-demand model downloader + synth primitive
        // (the public `/voice` WS is merged above; these are bearer-protected).
        .merge(voice::http_routes(state.clone()))
        .merge(crate::app_catalog::routes(state.clone()));

    #[cfg(feature = "local-inference")]
    {
        protected = protected.merge(local_inference::routes(state.clone()));
    }

    protected = protected.layer(middleware::from_fn_with_state(state, require_bearer_token));

    // Outermost layer so the access log sees every request's final status,
    // including 401s produced by the bearer middleware above.
    public.merge(protected).layer(middleware::from_fn(
        crate::middleware::access_log::http_access_log,
    ))
}

/// Deliberate panic on the request path, to exercise the panic circuit-breaker
/// during the durability dogfood. Only reachable when the route is mounted (env
/// flag set). The panic fires the global panic hook → the breaker counts it →
/// with `PERMAGENT_PANIC_BREAKER_MAX=1` it forces a clean `exit(1)` and launchd
/// relaunches a fresh daemon. Never returns.
async fn debug_panic_handler() -> axum::http::StatusCode {
    tracing::error!(
        target: "durability",
        "[debug] /debug/panic hit — triggering a deliberate panic to exercise the circuit-breaker"
    );
    panic!("[debug] deliberate panic-injection via POST /debug/panic (durability #560 dogfood)");
}

/// Locate the Command Center dist directory.
fn ui_dist_path() -> Option<std::path::PathBuf> {
    // 1. Explicit env var
    if let Ok(dir) = std::env::var("PERMAGENT_UI_DIR") {
        let p = std::path::PathBuf::from(dir);
        if p.join("index.html").exists() {
            return Some(p);
        }
    }

    // 2. Relative to current exe (../share/permagent/ui or sibling)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Development: repo root / ui/command-center/dist
            for ancestor in parent.ancestors().take(5) {
                let candidate = ancestor.join("ui/command-center/dist");
                if candidate.join("index.html").exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}
