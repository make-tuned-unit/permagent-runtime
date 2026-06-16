#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

pub mod acp;
pub mod action_required_manager;
pub mod activity;
pub mod agents;
pub mod app_catalog;
pub mod attachments;
pub mod brain_handle;
pub mod builtin_extension;
pub mod cards;
pub mod config;
pub mod context_mgmt;
pub mod conversation;
pub mod decision_inbox;
pub mod decisions;
pub mod dictation;
pub mod doctor;
pub mod download_manager;
pub mod events;
pub mod execution;
pub mod gateway;
pub mod goal_state;
pub mod goal_transition;
pub mod goose_apps;
pub mod hints;
pub mod identity;
pub mod instance_id;
pub mod logging;
pub mod mcp_utils;
pub mod model;
pub mod oauth;
#[cfg(feature = "otel")]
pub mod otel;
pub mod permission;
#[cfg(feature = "telemetry")]
pub mod posthog;
pub mod projects;
pub mod prompt_template;
pub mod providers;
pub mod recipe;
pub mod recipe_deeplink;
pub mod scheduler;
pub mod scheduler_trait;
pub mod security;
pub mod session;
pub mod session_context;
pub mod skills;
pub mod slash_commands;
pub mod sources;
pub mod steward;
pub mod storage_health;
pub mod subprocess;
pub mod tasks;
pub mod token_counter;
pub mod tool_inspection;
pub mod tool_monitor;
pub mod tracing;
pub mod utils;
pub mod workspaces;

/// Re-exported so dependents (goose-server) can name pool types from the
/// exact sqlx version this crate links (e.g. decisions/goal_transition APIs).
pub use sqlx;

#[cfg(test)]
pub mod test_sigabrt_handler;
