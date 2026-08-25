//! Activity event endpoints for the Permagent awareness layer.
//!
//! Auth is handled by the bearer token middleware in middleware::auth.

use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use permagent::activity::context_builder::DigestOpts;
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
    Json(mut event): Json<ActivityEvent>,
) -> Result<Json<EmitResponse>, (StatusCode, Json<ErrorBody>)> {
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
    if !(-5..=60).contains(&age) {
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

    // Capture the session→project association the client just told us about.
    //
    // This is the seam where it actually arrives: the desktop shells create
    // their chat sessions over ACP rather than through `POST /api/sessions`, so
    // a `project_selected` event carrying a session id is the moment the UI
    // says "this chat belongs to this project". Recorded as a HYPOTHESIS on the
    // session row, never as the session's wing — see `permagent::session_wing`
    // for the measurement (21% verified precision) that makes the difference.
    //
    // Once-only: `set_session_project_hint` refuses to overwrite an existing
    // hint, so a later selection cannot re-scope a conversation already under
    // way. Best-effort — an activity emit must never fail over this.
    capture_session_project_hint(&state, &event).await;

    let event_id = event.event_id.clone();
    activity::emit_activity(event);

    Ok(Json(EmitResponse {
        accepted: true,
        event_id,
    }))
}

/// Record a `project_selected` event's session→project association, if it has
/// one. Silent no-op for every other event, for a selection with no session
/// (opening a project board is a genuine sessionless selection), and for a
/// project id that is not in canonical `project:<slug>` form.
async fn capture_session_project_hint(state: &AppState, event: &ActivityEvent) {
    use permagent::events::activity::ActivityEventType;

    if event.event_type != ActivityEventType::ProjectSelected {
        return;
    }
    let Some(session_id) = event
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let Some(project_id) = event
        .project_id
        .as_deref()
        .or_else(|| event.payload.get("project_id").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return;
    };

    let pool = match state.session_manager().pool_clone().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::warn!(
                target: "permagentd::activity",
                error = %e,
                "no session pool for the project hint"
            );
            return;
        }
    };

    match permagent::session_wing::set_session_project_hint(&pool, session_id, project_id).await {
        Ok(true) => tracing::info!(
            target: "permagentd::activity",
            session = %session_id,
            project = %project_id,
            "recorded the session project hint"
        ),
        Ok(false) => tracing::debug!(
            target: "permagentd::activity",
            session = %session_id,
            "session already has a project hint — not overwritten"
        ),
        Err(e) => tracing::warn!(
            target: "permagentd::activity",
            session = %session_id,
            project = %project_id,
            error = %e,
            "could not record the session project hint"
        ),
    }
}

// ── GET /api/activity — durable activity journal (#619) ────────────────

#[derive(Deserialize)]
pub struct JournalQuery {
    /// Exclusive `ts` cursor: return rows strictly older than this timestamp.
    /// Pass the previous page's `next_before` to paginate.
    before: Option<String>,
    limit: Option<i64>,
    /// Comma-separated journal kinds (validated against `KNOWN_KINDS` server
    /// side — unknown names match nothing); absent = all kinds.
    kind: Option<String>,
    /// Exact actor match ("henry", "watcher", "librarian", …); absent = all.
    actor: Option<String>,
}

#[derive(Serialize)]
struct JournalPage {
    items: Vec<permagent::activity_journal::JournalItem>,
    /// Cursor for the next (older) page; absent when this page is the end.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_before: Option<String>,
}

async fn get_journal(
    State(state): State<Arc<AppState>>,
    Query(q): Query<JournalQuery>,
) -> Result<Json<JournalPage>, (StatusCode, Json<ErrorBody>)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let pool = state.session_manager().pool_clone().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("db unavailable: {}", e),
            }),
        )
    })?;
    let kinds: Option<Vec<String>> = q.kind.as_deref().map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect()
    });
    let items = permagent::activity_journal::page(
        &pool,
        q.before.as_deref(),
        limit,
        kinds.as_deref(),
        q.actor.as_deref(),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("journal query failed: {}", e),
            }),
        )
    })?;
    // A full page means there may be older rows; hand back the last ts as the
    // next exclusive cursor. A short page is the end of the journal.
    let next_before = if items.len() as i64 == limit {
        items.last().map(|i| i.ts.clone())
    } else {
        None
    };
    Ok(Json(JournalPage { items, next_before }))
}

// ── Rate limiter ───────────────────────────────────────────────────────

struct RateLimiter {
    timestamps: VecDeque<i64>,
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
        Err(_) => return true,
    };

    let cutoff_60s = now_ms - 60_000;
    while limiter.timestamps.front().is_some_and(|&t| t < cutoff_60s) {
        limiter.timestamps.pop_front();
    }

    if limiter.timestamps.len() >= 1000 {
        return false;
    }

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
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let mut result = serde_json::json!({
        "events_ingested": { "always": 0, "aggregated": 0 },
        "events_observed": { "ephemeral": 0 },
        "ingestion_failures": 0,
        "aggregation_queue_size": 0,
        "last_ingested_at": null,
        "paused": false,
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
        result["paused"] = serde_json::json!(ingester.is_paused());
        if let Some(ts) = ingester.last_ingested_at() {
            result["last_ingested_at"] = serde_json::json!(ts.to_rfc3339());
        }
        result["active_project"] = match ingester.active_project() {
            Some(ap) => serde_json::to_value(ap).unwrap_or_default(),
            None => serde_json::Value::Null,
        };
    }

    if let Some(ref cb) = state.context_builder {
        result["context_builder"]["recent_events_buffered"] =
            serde_json::json!(cb.buffered_count());
        result["context_builder"]["live_state"] =
            serde_json::to_value(cb.live_state_snapshot()).unwrap_or_default();
    }

    Ok(Json(result))
}

// ── GET /activity/recent-memories ──────────────────────────────────────

#[derive(Deserialize)]
pub struct RecentMemoriesQuery {
    #[serde(default = "default_memories_limit")]
    limit: usize,
}

fn default_memories_limit() -> usize {
    20
}

async fn get_recent_memories(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RecentMemoriesQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let limit = params.limit.min(100);

    let brain = state.brain.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "brain not available".into(),
            }),
        )
    })?;

    let result = brain
        .recall("activity recent events", spectral::Visibility::Private)
        .await;

    match result {
        Ok(result) => {
            let items: Vec<serde_json::Value> = result
                .memory_hits
                .into_iter()
                .filter(|m| m.source.as_deref() == Some("permagent.activity"))
                .take(limit)
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "key": m.key,
                        "content": m.content,
                        "wing": m.wing,
                        "compaction_tier": "raw",
                        "created_at": m.created_at,
                    })
                })
                .collect();
            Ok(Json(serde_json::json!({ "memories": items })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("brain recall failed: {}", e),
            }),
        )),
    }
}

// ── GET /activity/current-digest ───────────────────────────────────────

async fn get_current_digest(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let context_builder = state.context_builder.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "context_builder not available".into(),
            }),
        )
    })?;

    let focus_wing = state
        .activity_ingester
        .as_ref()
        .and_then(|ing| ing.active_project())
        .map(|ap| ap.wing);

    let opts = DigestOpts {
        include_probe: true,
        focus_wing,
        ..Default::default()
    };

    let cb = context_builder.clone();
    let result = tokio::task::spawn_blocking(move || cb.current_digest_blocking(opts))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: format!("spawn_blocking failed: {}", e),
                }),
            )
        })?;

    match result {
        Ok(digest) => Ok(Json(serde_json::to_value(&digest).unwrap_or_default())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("digest failed: {}", e),
            }),
        )),
    }
}

// ── POST /activity/pause ───────────────────────────────────────────────

async fn pause_ingestion(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    if let Some(ref ingester) = state.activity_ingester {
        ingester.pause();
        Ok(Json(serde_json::json!({ "paused": true })))
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "activity ingester not available".into(),
            }),
        ))
    }
}

// ── POST /activity/resume ──────────────────────────────────────────────

async fn resume_ingestion(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    if let Some(ref ingester) = state.activity_ingester {
        ingester.resume();
        Ok(Json(serde_json::json!({ "paused": false })))
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody {
                error: "activity ingester not available".into(),
            }),
        ))
    }
}

// ── Routes ─────────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/activity", get(get_journal))
        .route("/activity/recent", get(get_recent))
        .route("/activity/emit", post(emit_event))
        .route("/activity/ingest-status", get(ingest_status))
        .route("/activity/recent-memories", get(get_recent_memories))
        .route("/activity/current-digest", get(get_current_digest))
        .route("/activity/pause", post(pause_ingestion))
        .route("/activity/resume", post(resume_ingestion))
        .with_state(state)
}
