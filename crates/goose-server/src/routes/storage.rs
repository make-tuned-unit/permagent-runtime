//! Storage health scan endpoint.
//!
//! POST /permagent/storage/scan — runs a native filesystem scan and writes
//! findings to the same path the scheduler Phase 5 extractor uses.

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use permagent::storage_health;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanResponse {
    run_id: String,
    total_bytes: u64,
    total_findings: usize,
    categories: HashMap<String, CategoryStatsResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryStatsResponse {
    count: u64,
    total_bytes: u64,
}

async fn scan_handler(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<ScanResponse>, StatusCode> {
    let result = storage_health::run_scan().await;

    // Write findings to the standard findings path for UI consumption.
    // Write-then-respond: a scan whose findings never hit disk would show the
    // user an empty findings list with no error (bug-sweep wave 1), so a
    // failed write is a 500, and the write is atomic (temp + rename).
    let findings_dir = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        std::path::PathBuf::from(home).join(".permagent/automation/findings")
    };

    // Same validated ledger the automation UI reads. An unverified dump made
    // the run look like the findings could not be saved even when the rows
    // were on disk.
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()));
    storage_health::scanner::persist_structured_findings(
        &result.findings,
        &result.run_id,
        &findings_dir,
        &home,
        &storage_health::inuse::action::ActionProbe,
    )
    .map_err(|e| {
        tracing::error!(
            "Failed to persist scan findings for run {}: {}",
            result.run_id,
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let categories = result
        .categories
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                CategoryStatsResponse {
                    count: v.count,
                    total_bytes: v.total_bytes,
                },
            )
        })
        .collect();

    Ok(Json(ScanResponse {
        run_id: result.run_id,
        total_bytes: result.total_bytes,
        total_findings: result.findings.len(),
        categories,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/permagent/storage/scan", post(scan_handler))
        .with_state(state)
}
