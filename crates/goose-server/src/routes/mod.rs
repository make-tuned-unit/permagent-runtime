pub mod action_required;
pub mod agent;
pub mod config_management;
pub mod errors;
pub mod events;
pub mod features;
pub mod gateway;
#[cfg(feature = "local-inference")]
pub mod local_inference;
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
pub mod telemetry;
pub mod tunnel;
pub mod integrations;
pub mod utils;

use std::sync::Arc;

use axum::Router;

// Function to configure all routes
pub fn configure(state: Arc<crate::state::AppState>, _secret_key: String) -> Router {
    let router = Router::new()
        .merge(status::routes(state.clone()))
        .merge(reply::routes(state.clone()))
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
        .merge(session_events::routes(state.clone()))
        .merge(sampling::routes(state.clone()))
        .merge(features::routes())
        .merge(events::routes(state.clone()))
        .merge(skills::routes(state.clone()))
        .merge(integrations::routes(state.clone()));

    #[cfg(feature = "local-inference")]
    let router = router.merge(local_inference::routes(state));

    router
}
