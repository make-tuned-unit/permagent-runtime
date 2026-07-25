//! Librarian module — schedule management, consolidation, pruning, and status.
//!
//! Split from the original ollama.rs monolith. Each submodule owns a single
//! concern; this file wires them together for route registration.

pub mod consolidation;
mod pruning;
mod scheduling;

// Re-export types used by external callers (state.rs, tests).
pub use scheduling::librarian_scheduler_loop;

use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};

use crate::state::AppState;

/// Register the 4 Librarian HTTP routes.
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/librarian/schedule",
            get(scheduling::get_librarian_schedule),
        )
        .route(
            "/api/librarian/schedule",
            axum::routing::put(scheduling::set_librarian_schedule),
        )
        .route(
            "/api/librarian/run-now",
            post(scheduling::run_librarian_now),
        )
        .route(
            "/api/librarian/status",
            get(scheduling::get_librarian_status),
        )
        .route(
            "/api/librarian/run-status",
            get(scheduling::librarian_run_status),
        )
        .with_state(state)
}
