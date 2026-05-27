pub mod action_required;
pub mod activity;
pub mod agent;
pub mod agents;
pub mod attachments;
pub mod brain;
pub mod browser_content;
pub mod config_management;
pub mod dashboard;
pub mod errors;
pub mod events;
pub mod features;
pub mod findings;
pub mod gateway;
pub mod henry_status;
pub mod identity;
pub mod integrations;
pub mod librarian;
#[cfg(feature = "local-inference")]
pub mod local_inference;
pub mod ollama;
pub mod prompts;
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
pub mod workers;
pub mod workspaces;

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
        .merge(session_events::routes(state.clone()));

    // Static UI assets
    let ui_dir = ui_dist_path();
    if let Some(dir) = ui_dir {
        let index = dir.join("index.html");
        public = public.nest_service("/ui", ServeDir::new(&dir).fallback(ServeFile::new(index)));
    }

    // ── Protected routes (bearer token required) ──
    let mut protected = Router::new()
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
        .merge(brain::routes(state.clone()))
        .merge(browser_content::routes(state.clone()))
        .merge(dashboard::routes(state.clone()))
        .merge(identity::routes(state.clone()))
        .merge(workers::routes(state.clone()))
        .merge(findings::routes(state.clone()))
        .merge(storage::routes(state.clone()))
        .merge(ollama::routes(state.clone()))
        .merge(librarian::routes(state.clone()))
        .merge(henry_status::routes(state.clone()))
        .merge(agents::routes(state.clone()))
        .merge(crate::app_catalog::routes(state.clone()));

    #[cfg(feature = "local-inference")]
    {
        protected = protected.merge(local_inference::routes(state.clone()));
    }

    protected = protected.layer(middleware::from_fn_with_state(state, require_bearer_token));

    public.merge(protected)
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
