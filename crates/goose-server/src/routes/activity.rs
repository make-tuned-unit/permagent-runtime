//! Activity event endpoints for the Permagent awareness layer.
//!
//! - GET  /activity/recent?limit=N — debug endpoint, returns last N events
//!   from in-memory ring buffer. TODO: auth-gate in Phase 3 when the
//!   inspection panel becomes the canonical reader.
//!
//! - POST /activity/emit — authenticated endpoint for frontend surfaces
//!   to push activity events onto the daemon bus. Bearer token auth,
//!   rate-limited.

use axum::{
    extract::{Json, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use permagent::events::activity::{self, ActivityEvent};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};

use crate::state::AppState;

// ── GET /activity/recent ───────────────────────────────────────────────

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

// ── POST /activity/emit ────────────────────────────────────────────────

#[derive(Serialize)]
struct EmitResponse {
    accepted: bool,
    event_id: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

async fn emit_event(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut event): Json<ActivityEvent>,
) -> Result<Json<EmitResponse>, (StatusCode, Json<ErrorBody>)> {
    // ── Auth ──
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let expected = state.daemon_token.as_deref();
    match (token, expected) {
        (_, None) => {
            // No daemon token configured — reject all external emit calls
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "daemon token not configured".into(),
                }),
            ));
        }
        (None, Some(_)) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "missing Authorization: Bearer <token> header".into(),
                }),
            ));
        }
        (Some(provided), Some(expected)) if provided != expected => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "invalid token".into(),
                }),
            ));
        }
        _ => {} // Auth OK
    }

    // ── Rate limit ──
    if !check_rate_limit() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorBody {
                error: "rate limit exceeded (100/s or 1000/60s)".into(),
            }),
        ));
    }

    // ── Validate timestamp (within last 60 seconds) ──
    let age = Utc::now()
        .signed_duration_since(event.timestamp)
        .num_seconds();
    if age > 60 || age < -5 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("timestamp out of range (age={}s, max=60s)", age),
            }),
        ));
    }

    // ── Validate event_id is valid UUID ──
    if uuid::Uuid::parse_str(&event.event_id).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "event_id must be a valid UUID".into(),
            }),
        ));
    }

    // ── Enforce canonical tier (server-side override) ──
    event.tier = activity::canonical_tier(&event.event_type);

    let event_id = event.event_id.clone();
    activity::emit_activity(event);

    Ok(Json(EmitResponse {
        accepted: true,
        event_id,
    }))
}

// ── Rate limiter ───────────────────────────────────────────────────────
//
// Simple sliding-window: 100 events/second, 1000 events/60 seconds.
// Intentionally generous — foot-gun guard, not a quota. Limits are
// global (not per-token) since there's only one token in Phase 2.

struct RateLimiter {
    timestamps: VecDeque<i64>, // unix millis
}

static RATE_LIMITER: LazyLock<Mutex<RateLimiter>> = LazyLock::new(|| {
    Mutex::new(RateLimiter {
        timestamps: VecDeque::with_capacity(1100),
    })
});

fn check_rate_limit() -> bool {
    let now_ms = Utc::now().timestamp_millis();
    let mut limiter = match RATE_LIMITER.lock() {
        Ok(l) => l,
        Err(_) => return true, // Poisoned — allow
    };

    // Prune entries older than 60 seconds
    let cutoff_60s = now_ms - 60_000;
    while limiter
        .timestamps
        .front()
        .map_or(false, |&t| t < cutoff_60s)
    {
        limiter.timestamps.pop_front();
    }

    // Check 60-second window (1000 max)
    if limiter.timestamps.len() >= 1000 {
        return false;
    }

    // Check 1-second window (100 max)
    let cutoff_1s = now_ms - 1_000;
    let count_1s = limiter
        .timestamps
        .iter()
        .rev()
        .take_while(|&&t| t >= cutoff_1s)
        .count();
    if count_1s >= 100 {
        return false;
    }

    limiter.timestamps.push_back(now_ms);
    true
}

// ── GET /activity/ingest-status ─────────────────────────────────────────

async fn ingest_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    // Auth-gated with daemon token
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    if let Some(expected) = state.daemon_token.as_deref() {
        if token != Some(expected) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "unauthorized".into(),
                }),
            ));
        }
    }

    let mut result = serde_json::json!({
        "events_ingested": { "always": 0, "aggregated": 0 },
        "events_observed": { "ephemeral": 0 },
        "ingestion_failures": 0,
        "aggregation_queue_size": 0,
        "last_ingested_at": null,
        "context_builder": {
            "recent_events_buffered": 0,
            "live_state": {}
        }
    });

    if let Some(ref ingester) = state.activity_ingester {
        result["events_ingested"]["always"] = serde_json::json!(ingester.always_count());
        result["events_ingested"]["aggregated"] = serde_json::json!(ingester.aggregated_count());
        result["events_observed"]["ephemeral"] = serde_json::json!(ingester.ephemeral_count());
        result["ingestion_failures"] = serde_json::json!(ingester.failure_count());
        result["aggregation_queue_size"] = serde_json::json!(ingester.aggregation_queue_size());
        if let Some(ts) = ingester.last_ingested_at() {
            result["last_ingested_at"] = serde_json::json!(ts.to_rfc3339());
        }
    }

    if let Some(ref cb) = state.context_builder {
        result["context_builder"]["recent_events_buffered"] =
            serde_json::json!(cb.buffered_count());
        result["context_builder"]["live_state"] =
            serde_json::to_value(cb.live_state_snapshot()).unwrap_or_default();
    }

    Ok(Json(result))
}

// ── Routes ─────────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/activity/recent", get(get_recent))
        .route("/activity/emit", post(emit_event))
        .route("/activity/ingest-status", get(ingest_status))
        .with_state(state)
}
