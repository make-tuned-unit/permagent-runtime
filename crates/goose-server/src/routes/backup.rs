//! API routes for backup management.
//!
//! GET  /api/backups      — list all snapshots
//! POST /api/backups/run  — snapshot memory.db, graph.sqlite, recognition.db,
//!                          and permagent.db

use axum::{extract::State, routing, Json, Router};
use serde::Serialize;
use std::path::PathBuf;
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
    brain_graph: RunResult,
    brain_recognition: RunResult,
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
    snapshots.extend(backup::list_snapshot_info(
        &backup_root,
        DbTarget::BrainGraph,
    ));
    snapshots.extend(backup::list_snapshot_info(
        &backup_root,
        DbTarget::BrainRecognition,
    ));
    snapshots.extend(backup::list_snapshot_info(&backup_root, DbTarget::Spectral));
    Json(ListResponse { snapshots })
}

async fn run_one(backup_root: PathBuf, source: PathBuf, target: DbTarget) -> RunResult {
    match tokio::task::spawn_blocking(move || {
        backup::force_snapshot(
            &source,
            &backup_root,
            target,
            backup::SnapshotMode::Compacted,
        )
    })
    .await
    {
        Ok(Ok(info)) => RunResult::Ok(info),
        Ok(Err(backup::BackupError::SourceMissing(_))) => RunResult::Skipped {
            reason: format!("{} does not exist", target.label()),
        },
        Ok(Err(e)) => RunResult::Error {
            message: e.to_string(),
        },
        Err(e) => RunResult::Error {
            message: format!("task panicked: {e}"),
        },
    }
}

#[utoipa::path(post, path = "/api/backups/run",
    responses(
        (status = 200, description = "Force immediate backup of memory.db, graph.sqlite, recognition.db, and permagent.db"),
    )
)]
async fn run_backup(State(_state): State<Arc<AppState>>) -> Json<RunResponse> {
    let base = permagent::config::paths::Paths::data_dir();
    let backup_root = base.join("backups");

    let brain_dir = permagent::config::paths::Paths::brain_dir();
    let brain = run_one(
        backup_root.clone(),
        brain_dir.join("memory.db"),
        DbTarget::Brain,
    )
    .await;
    let brain_graph = run_one(
        backup_root.clone(),
        brain_dir.join("graph.sqlite"),
        DbTarget::BrainGraph,
    )
    .await;
    let brain_recognition = run_one(
        backup_root.clone(),
        brain_dir.join("recognition.db"),
        DbTarget::BrainRecognition,
    )
    .await;
    let spectral = run_one(
        backup_root,
        permagent::config::paths::Paths::spectral_db(),
        DbTarget::Spectral,
    )
    .await;

    Json(RunResponse {
        brain,
        brain_graph,
        brain_recognition,
        spectral,
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/backups", routing::get(list_backups))
        .route("/api/backups/run", routing::post(run_backup))
        .with_state(state)
}
