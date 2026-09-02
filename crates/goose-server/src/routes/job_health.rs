//! `GET /api/job-health` — the daily digest's data source.
//!
//! One endpoint, one job: hand back the table of everything the runtime has
//! promised to do on a cadence, with what actually happened to each. It is the
//! answer to "is anything quietly not working", and it answers even when the
//! answer is "no" — see `permagent::job_health` for why a green report still
//! has to be a report.
//!
//! Rendering is not here. The notification and UI layers decide what a row
//! looks like; this returns the truth table they render.

use axum::{extract::State, routing::get, Json, Router};
use permagent::job_health::{self, JobHealthDigest};
use std::sync::Arc;

use crate::state::AppState;

async fn job_health_digest(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let Ok(pool) = state.session_manager().pool_clone().await else {
        // Honest failure: an empty table would read as "everything is fine".
        return Json(serde_json::json!({
            "error": "the runtime database is not available, so job health is unknown",
        }));
    };
    let scheduled = state.scheduler().list_scheduled_jobs().await;
    let digest: JobHealthDigest = job_health::collect(&pool, &scheduled).await;
    Json(serde_json::to_value(&digest).unwrap_or_else(
        |e| serde_json::json!({ "error": format!("job health could not be encoded: {e}") }),
    ))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/job-health", get(job_health_digest))
        .with_state(state)
}
