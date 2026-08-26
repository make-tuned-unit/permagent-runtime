#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

pub mod acp;
pub mod action_required_manager;
pub mod activity;
pub mod activity_journal;
pub mod after_turn;
pub mod agents;
pub mod app_catalog;
pub mod app_views;
pub mod attachments;
pub mod backup;
pub mod brain_enrichment;
pub mod brain_handle;
pub mod briefings;
pub mod builtin_extension;
pub mod cards;
pub mod concierge;
pub mod config;
pub mod context_layers;
pub mod context_mgmt;
pub mod conversation;
pub mod cost_router;
pub mod council;
pub mod decision_inbox;
pub mod decisions;
pub mod decisions_effects;
#[cfg(test)]
mod dependency_claim_guard;
pub mod dictation;
pub mod doctor;
pub mod download_manager;
pub mod echo;
pub mod events;
pub mod executable_skills;
pub mod execution;
pub mod finance_ledger;
pub mod finance_statements;
pub mod financier_close;
pub mod forecaster;
pub mod gateway;
pub mod goal_landing;
pub mod goal_refinement;
pub mod goal_state;
pub mod goal_transition;
pub mod goose_apps;
pub mod grow_media;
pub mod growth;
pub mod hall_backfill;
pub mod hall_rules;
pub mod hints;
pub mod identity;
pub mod inbox;
pub mod incidents;
pub mod initiative;
pub mod instance_id;
pub mod lessons;
pub mod logging;
pub mod market_data;
pub mod mcp_utils;
pub mod meeting_writeup;
pub mod mesh;
pub mod model;
pub mod oauth;
#[cfg(feature = "otel")]
pub mod otel;
pub mod overbought;
pub mod people;
pub mod people_bridge;
pub mod people_create;
pub mod people_merge;
pub mod people_provenance;
pub mod permission;
pub mod person_meetings;
pub mod pick_loop;
pub mod picker;
pub mod playbook;
pub mod polybot;
#[cfg(feature = "telemetry")]
pub mod posthog;
pub mod privacy;
pub mod project_association;
pub mod project_documents;
pub mod project_graph;
pub mod project_notes;
pub mod project_stack;
pub mod projects;
pub mod prompt_template;
pub mod providers;
pub mod python_runtime;
pub mod reader;
pub mod recipe;
pub mod recipe_deeplink;
pub mod recognition;
pub mod recognition_consent;
#[cfg(feature = "spectral-recognition")]
pub mod recognition_sink;
pub mod rlm;
pub mod rss;
pub mod scheduler;
pub mod scheduler_trait;
pub mod security;
pub mod session;
pub mod session_context;
pub mod session_wing;
pub mod skill_md;
pub mod skills;
pub mod slash_commands;
pub mod sources;
pub mod sovereignty;
pub mod steward;
pub mod storage_health;
pub mod strix;
pub mod subprocess;
pub mod tasks;
pub mod token_counter;
pub mod tool_inspection;
pub mod tool_monitor;
pub mod tracing;
pub mod trading_calendar;
pub mod turn_sampling;
pub mod utils;
pub mod verification_approval;
pub mod wing_backfill;
pub mod wing_rules;
pub mod workspace_trust;
pub mod workspaces;

/// Re-exported so dependents (goose-server) can name pool types from the
/// exact sqlx version this crate links (e.g. decisions/goal_transition APIs).
pub use sqlx;

#[cfg(test)]
pub mod test_sigabrt_handler;
