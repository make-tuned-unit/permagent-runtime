pub mod action_required;
pub mod activity;
pub mod agent;
pub mod agents;
pub mod analytics_attribution;
pub mod analytics_classify;
pub mod analytics_funnel;
pub mod analytics_verify;
pub mod attachments;
pub mod backup;
pub mod brain;
pub mod browser_act;
pub mod browser_content;
pub mod browser_state;
pub mod cards;
pub mod coding_session;
pub mod config_management;
pub mod dashboard;
pub mod dashboard_cards;
pub mod decisions;
pub mod desktop_control;
pub mod dev_roots;
pub mod devices;
pub mod dictation;
pub mod errors;
pub mod events;
pub mod features;
pub mod findings;
pub mod first_party_analytics;
pub mod gateway;
pub mod governance;
pub mod grow;
pub mod grow_analytics;
pub mod growth_actions;
pub mod growth_verify;
pub mod henry_status;
pub mod identity;
pub mod inbox;
pub mod incidents;
pub mod integrations;
pub mod librarian;
#[cfg(feature = "local-inference")]
pub mod local_inference;
pub mod ollama;
pub mod onboarding;
pub mod people;
pub mod projects;
pub mod prompts;
pub mod reader;
pub mod recipe;
pub mod recipe_utils;
pub mod reply;
pub mod runs;
pub mod sampling;
pub mod schedule;
pub mod security;
pub mod session;
pub mod session_events;
pub mod setup;
pub mod skills;
pub mod sse_token;
pub mod status;
pub mod storage;
pub mod telemetry;
pub mod terminal_supervision;
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

use crate::middleware::auth::{
    require_bearer_token, require_daemon_token_header_or_query, require_token_header_or_query,
};

/// Configure all routes, applying bearer token middleware to protected endpoints.
pub fn configure(state: Arc<crate::state::AppState>) -> Router {
    // ── Public routes (no auth required) ──
    let mut public = Router::new()
        .merge(status::routes(state.clone()))
        .merge(version::routes(state.clone()))
        // `/events` WS: token-validated inside its upgrade handler (bearer
        // header for native clients, `?token=` for browsers), like `/voice`.
        .merge(events::routes(state.clone()))
        // Browser content bridge: unauthenticated, localhost-only.
        // Called by the in-process MCP tool (no access to daemon token).
        .merge(browser_content::routes(state.clone()))
        .merge(desktop_control::routes(state.clone()))
        // Act-on-page bridge (#649): snapshot + click/type/select. Same rail.
        .merge(browser_act::routes(state.clone()))
        // Voice WebSocket: does its own token validation via query param
        // (WebSocket upgrade can't use the Bearer middleware).
        .merge(voice::routes(state.clone()))
        // Device pairing claim exchange (#628): the claiming companion has no
        // credential yet. Codes are 128-bit random, single-use, short-lived;
        // the origin guard below still fronts it.
        .merge(devices::public_routes(state.clone()));

    // ── Session event stream: long-lived OR scoped token ──
    // The browser EventSource cannot set an Authorization header, so the SSE
    // route accepts `?token=`, including short-lived stream-scoped tokens.
    let session_event_stream = session_events::event_routes(state.clone()).layer(
        middleware::from_fn_with_state(state.clone(), require_token_header_or_query),
    );

    // ── Session control plane: long-lived token only, header OR ?token= ──
    // Reply invokes the agent and cancel mutates a live turn. Preserve the
    // established query-token transport for native clients, but do not admit
    // stream-scoped credentials to either endpoint.
    let session_control = session_events::control_routes(state.clone()).layer(
        middleware::from_fn_with_state(state.clone(), require_daemon_token_header_or_query),
    );

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
        // Session diagnostics zip (logs + config + system info): bearer-only —
        // its only HTTP consumers hold the token (the CLI builds diagnostics
        // in-process, not over HTTP).
        .merge(status::protected_routes(state.clone()))
        .merge(sse_token::routes(state.clone()))
        .merge(backup::routes(state.clone()))
        .merge(reply::routes(state.clone()))
        .merge(activity::routes(state.clone()))
        .merge(action_required::routes(state.clone()))
        .merge(incidents::routes(state.clone()))
        .merge(agent::routes(state.clone()))
        .merge(config_management::routes(state.clone()))
        .merge(coding_session::routes(state.clone()))
        .merge(security::routes(state.clone()))
        .merge(prompts::routes())
        .merge(recipe::routes(state.clone()))
        .merge(session::routes(state.clone()))
        .merge(schedule::routes(state.clone()))
        .merge(setup::routes(state.clone()))
        .merge(telemetry::routes(state.clone()))
        // S2 (#428) + S3 (#429): supervised-terminal tee ingest — the Tauri
        // PTY reader POSTs raw output chunks of SUPERVISED sessions here
        // (bearer token); detected gates are bridged into the Decision Inbox.
        .merge(terminal_supervision::routes(state.clone()))
        .merge(tunnel::routes(state.clone()))
        .merge(gateway::routes(state.clone()))
        .merge(sampling::routes(state.clone()))
        .merge(features::routes())
        .merge(dev_roots::routes())
        .merge(onboarding::routes())
        // Built-in browser bookmarks + saved tab sets (#790): small JSON state.
        .merge(browser_state::routes())
        .merge(skills::routes(state.clone()))
        .merge(integrations::routes(state.clone()))
        .merge(workspaces::routes(state.clone()))
        .merge(attachments::routes(state.clone()))
        .merge(reader::routes(state.clone()))
        .merge(dictation::routes(state.clone()))
        .merge(brain::routes(state.clone()))
        .merge(people::routes(state.clone()))
        .merge(inbox::routes(state.clone()))
        .merge(dashboard::routes(state.clone()))
        .merge(dashboard_cards::routes(state.clone()))
        .merge(identity::routes(state.clone()))
        .merge(workers::routes(state.clone()))
        .merge(findings::routes(state.clone()))
        .merge(storage::routes(state.clone()))
        .merge(ollama::routes(state.clone()))
        .merge(librarian::routes(state.clone()))
        .merge(henry_status::routes(state.clone()))
        .merge(runs::routes(state.clone()))
        .merge(world::routes(state.clone()))
        .merge(projects::routes(state.clone()))
        .merge(cards::routes(state.clone()))
        .merge(grow::routes(state.clone()))
        .merge(grow_analytics::routes(state.clone()))
        .merge(growth_actions::routes(state.clone()))
        .merge(first_party_analytics::routes(state.clone()))
        .merge(governance::routes(state.clone()))
        .merge(decisions::routes(state.clone()))
        .merge(devices::routes(state.clone()))
        .merge(agents::routes(state.clone()))
        // Voice HTTP endpoints — on-demand model downloader + synth primitive
        // (the public `/voice` WS is merged above; these are bearer-protected).
        .merge(voice::http_routes(state.clone()))
        .merge(crate::app_catalog::routes(state.clone()));

    #[cfg(feature = "local-inference")]
    {
        protected = protected.merge(local_inference::routes(state.clone()));
    }

    let collect_state = state.clone();
    protected = protected.layer(middleware::from_fn_with_state(state, require_bearer_token));

    // Origin guard over EVERYTHING (public + protected): a remote web page in
    // the user's browser must not be able to reach the daemon cross-origin,
    // token or not. Native/no-Origin clients pass. Access log stays outermost
    // so it sees every request's final status, including the 401/403s produced
    // by the auth and origin layers above.
    public
        .merge(protected)
        .merge(session_event_stream)
        .merge(session_control)
        .layer(middleware::from_fn(
            crate::middleware::origin_guard::require_allowed_origin,
        ))
        // First-party analytics beacon (#23): merged AFTER the origin guard
        // layer because the user's own website posts beacons cross-origin from
        // visitors' browsers by design. Site-key scoped, rate-limited,
        // insert-only — see routes/first_party_analytics.rs module docs.
        // Still inside the access log (outermost layer sees every request).
        .merge(first_party_analytics::collect_routes(collect_state))
        .layer(middleware::from_fn(
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
