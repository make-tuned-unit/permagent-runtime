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

    // Write findings to the standard findings path for UI consumption
    let findings_dir = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        std::path::PathBuf::from(home).join(".permagent/automation/findings")
    };
    let _ = std::fs::create_dir_all(&findings_dir);
    let findings_path = findings_dir.join(format!("{}.json", result.run_id));

    // Convert ScanFinding to the same shape findings.rs expects
    let findings_file = serde_json::json!({
        "run_id": result.run_id,
        "findings": result.findings,
    });
    let _ = std::fs::write(
        &findings_path,
        serde_json::to_string_pretty(&findings_file).unwrap_or_default(),
    );

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
