use crate::state::AppState;
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct BrainSearchQuery {
    pub q: String,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// "memory", "chat", or "both" (default).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrainSearchResult {
    /// "memory" or "chat"
    pub source: String,
    pub id: String,
    pub preview: String,
    pub score: f64,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrainSearchResponse {
    pub results: Vec<BrainSearchResult>,
    pub total: usize,
    pub query: String,
    pub offset: usize,
    pub limit: usize,
    pub fts_count: usize,
    pub spectral_count: usize,
    pub dedup_count: usize,
}

/// Query chat history FTS via SessionManager.
/// FTS has no relevance scoring — all keyword matches get a constant 0.7.
async fn query_fts(
    state: &AppState,
    query: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> anyhow::Result<Vec<BrainSearchResult>> {
    let fts = state
        .session_manager()
        .search_chat_history(
            query,
            Some(100), // fetch up to 100 sessions for brain search
            since,
            until,
            None, // no session exclusion
            vec![SessionType::User, SessionType::Scheduled],
        )
        .await?;

    let mut results = Vec::new();
    for session_result in fts.results {
        for msg in session_result.messages {
            let preview = truncate_preview(&msg.content, 200);
            results.push(BrainSearchResult {
                source: "chat".to_string(),
                id: format!("{}:{}", session_result.session_id, msg.timestamp.timestamp()),
                preview,
                score: 0.7,
                timestamp: msg.timestamp,
                session_id: Some(session_result.session_id.clone()),
            });
        }
    }
    Ok(results)
}

/// Query Spectral Brain recall (semantic fingerprint similarity).
/// Uses signal_score directly (already ~0.0-1.0).
async fn query_spectral(
    state: &AppState,
    query: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> anyhow::Result<Vec<BrainSearchResult>> {
    let brain = match state.brain.as_ref() {
        Some(b) => b.clone(),
        None => return Ok(Vec::new()),
    };

    let query_owned = query.to_string();
    let recall_result = tokio::task::spawn_blocking(move || {
        brain.recall(&query_owned, spectral::Visibility::Private)
    })
    .await??;

    let mut results = Vec::new();
    for (i, hit) in recall_result.memory_hits.into_iter().enumerate() {
        // Spectral doesn't expose timestamps on recall hits.
        // Use Utc::now() as a placeholder; filter by score floor instead.
        let ts = Utc::now();

        // Apply date filter if provided (best-effort — Spectral lacks native date support)
        if let Some(ref s) = since {
            if ts < *s {
                continue;
            }
        }
        if let Some(ref u) = until {
            if ts > *u {
                continue;
            }
        }

        let preview = truncate_preview(&hit.content, 200);
        results.push(BrainSearchResult {
            source: "memory".to_string(),
            id: format!("spectral:{}", i),
            preview,
            score: hit.signal_score,
            timestamp: ts,
            session_id: None,
        });
    }
    Ok(results)
}

/// Merge results from both backends, sort by score, deduplicate.
/// v1.0: no cross-backend dedup. FTS returns chat messages, Spectral returns
/// distilled memories — different granularities that don't naturally overlap.
fn merge_and_rank(
    fts: Vec<BrainSearchResult>,
    spectral: Vec<BrainSearchResult>,
) -> (Vec<BrainSearchResult>, usize) {
    let mut combined: Vec<BrainSearchResult> = fts.into_iter().chain(spectral).collect();
    combined.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (combined, 0)
}

fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

async fn brain_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BrainSearchQuery>,
) -> Result<Json<BrainSearchResponse>, crate::routes::errors::ErrorResponse> {
    let limit = params.limit.unwrap_or(50).min(200);
    let source_filter = params.source.as_deref().unwrap_or("both");

    let (fts_results, spectral_results) = match source_filter {
        "memory" => (
            Vec::new(),
            query_spectral(&state, &params.q, params.since, params.until)
                .await
                .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?,
        ),
        "chat" => (
            query_fts(&state, &params.q, params.since, params.until)
                .await
                .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?,
            Vec::new(),
        ),
        _ => {
            let (fts, spectral) = tokio::try_join!(
                query_fts(&state, &params.q, params.since, params.until),
                query_spectral(&state, &params.q, params.since, params.until),
            )
            .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
            (fts, spectral)
        }
    };

    let fts_count = fts_results.len();
    let spectral_count = spectral_results.len();

    let (merged, dedup_count) = merge_and_rank(fts_results, spectral_results);
    let total = merged.len();

    let paginated: Vec<BrainSearchResult> = merged
        .into_iter()
        .skip(params.offset)
        .take(limit)
        .collect();

    Ok(Json(BrainSearchResponse {
        results: paginated,
        total,
        query: params.q,
        offset: params.offset,
        limit,
        fts_count,
        spectral_count,
        dedup_count,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/brain/search", get(brain_search))
        .with_state(state)
}
