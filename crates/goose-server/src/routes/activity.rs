//! Debug endpoint for the activity event ring buffer.
//!
//! GET /activity/recent?limit=N — returns the last N activity events.
//! In-memory only; lost on daemon restart. Phase 2 replaces this with
//! Brain queries.

use axum::{extract::Query, routing::get, Json, Router};
use permagent::events::activity::{self, ActivityEvent};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct RecentQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn get_recent(Query(params): Query<RecentQuery>) -> Json<Vec<ActivityEvent>> {
    let limit = params.limit.min(500);
    Json(activity::recent_activity(limit))
}

pub fn routes(_state: Arc<crate::state::AppState>) -> Router {
    Router::new().route("/activity/recent", get(get_recent))
}
