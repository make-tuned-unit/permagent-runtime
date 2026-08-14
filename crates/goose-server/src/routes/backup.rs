//! API routes for backup management.
//!
//! GET  /api/backups      — list all snapshots
//! POST /api/backups/run  — force immediate snapshot of both DBs

use axum::{extract::State, routing, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use crate::backup::{self, DbTarget, SnapshotInfo};
use crate::state::AppState;

#[derive(Serialize)]
struct ListResponse {
    snapshots: Vec<SnapshotInfo>,
}

#[derive(Serialize)]
struct RunResponse {
    brain: RunResult,
    spectral: RunResult,
}

#[derive(Serialize)]
#[serde(tag = "status")]
enum RunResult {
    #[serde(rename = "ok")]
    Ok(SnapshotInfo),
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "skipped")]
    Skipped { reason: String },
}

#[utoipa::path(get, path = "/api/backups",
    responses(
        (status = 200, description = "List all backup snapshots"),
    )
)]
async fn list_backups(State(_state): State<Arc<AppState>>) -> Json<ListResponse> {
    let backup_root = permagent::config::paths::Paths::data_dir().join("backups");
    let mut snapshots = Vec::new();
    snapshots.extend(backup::list_snapshot_info(&backup_root, DbTarget::Brain));
    snapshots.extend(backup::list_snapshot_info(&backup_root, DbTarget::Spectral));
    Json(ListResponse { snapshots })
}

#[utoipa::path(post, path = "/api/backups/run",
    responses(
        (status = 200, description = "Force immediate backup of both databases"),
    )
)]
async fn run_backup(State(_state): State<Arc<AppState>>) -> Json<RunResponse> {
    let base = permagent::config::paths::Paths::data_dir();
    let backup_root = base.join("backups");

    let brain_result = tokio::task::spawn_blocking({
        let backup_root = backup_root.clone();
        move || {
            let source = permagent::config::paths::Paths::brain_dir().join("memory.db");
            backup::force_snapshot(
                &source,
                &backup_root,
                DbTarget::Brain,
                backup::SnapshotMode::Compacted,
            )
        }
    })
    .await;

    let spectral_result = tokio::task::spawn_blocking({
        let backup_root = backup_root.clone();
        move || {
            let source = permagent::config::paths::Paths::spectral_db();
            backup::force_snapshot(
                &source,
                &backup_root,
                DbTarget::Spectral,
                backup::SnapshotMode::Compacted,
            )
        }
    })
    .await;

    let brain = match brain_result {
        Ok(Ok(info)) => RunResult::Ok(info),
        Ok(Err(backup::BackupError::SourceMissing(_))) => RunResult::Skipped {
            reason: "brain/memory.db does not exist".to_string(),
        },
        Ok(Err(e)) => RunResult::Error {
            message: e.to_string(),
        },
        Err(e) => RunResult::Error {
            message: format!("task panicked: {e}"),
        },
    };

    let spectral = match spectral_result {
        Ok(Ok(info)) => RunResult::Ok(info),
        Ok(Err(backup::BackupError::SourceMissing(_))) => RunResult::Skipped {
            reason: "spectral/permagent.db does not exist".to_string(),
        },
        Ok(Err(e)) => RunResult::Error {
            message: e.to_string(),
        },
        Err(e) => RunResult::Error {
            message: format!("task panicked: {e}"),
        },
    };

    Json(RunResponse { brain, spectral })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/backups", routing::get(list_backups))
        .route("/api/backups/run", routing::post(run_backup))
        .with_state(state)
}
