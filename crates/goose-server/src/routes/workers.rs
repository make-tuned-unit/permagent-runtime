use crate::state::AppState;
use axum::{
    extract::{Path, State},
    routing::{delete, get, put},
    Json, Router,
};
use permagent::config::agent_identity::{self, WorkerPersona};
use permagent::config::worker_probe;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Serialize)]
struct WorkerResponse {
    key: String,
    first_name: String,
    last_name: Option<String>,
    nickname: Option<String>,
    display_name: String,
    role: String,
    traits: Vec<String>,
    tone: String,
    /// How this worker is run: "internal_subagent" | "external_cli" | "pending".
    engine: String,
    /// Live availability from the worker's probe (e.g. is the CLI on PATH).
    available: bool,
    /// Why unavailable; `None` when available.
    reason: Option<String>,
}

impl WorkerResponse {
    fn from_entry(key: &str, w: &WorkerPersona, available: bool, reason: Option<String>) -> Self {
        Self {
            key: key.to_string(),
            first_name: w.first_name.clone(),
            last_name: w.last_name.clone(),
            nickname: w.nickname.clone(),
            display_name: w.display_name(),
            role: w.role.clone(),
            traits: w.traits.clone(),
            tone: w.tone.clone(),
            engine: w.engine.label().to_string(),
            available,
            reason,
        }
    }
}

async fn list_workers(State(state): State<Arc<AppState>>) -> Json<HashMap<String, WorkerResponse>> {
    let workers: Vec<(String, WorkerPersona)> = {
        let ac = state.agent_config.read().await;
        ac.workers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };

    // Probe off the async runtime — `model_loaded:` does a blocking HTTP call.
    let map = tokio::task::spawn_blocking(move || {
        workers
            .into_iter()
            .map(|(k, w)| {
                let (available, reason) = worker_probe::probe_worker(&w.availability_check);
                let resp = WorkerResponse::from_entry(&k, &w, available, reason);
                (k, resp)
            })
            .collect::<HashMap<_, _>>()
    })
    .await
    .unwrap_or_default();

    Json(map)
}

async fn get_worker(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<WorkerResponse>, axum::http::StatusCode> {
    let worker = {
        let ac = state.agent_config.read().await;
        ac.workers.get(&key).cloned()
    }
    .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    let resp = tokio::task::spawn_blocking(move || {
        let (available, reason) = worker_probe::probe_worker(&worker.availability_check);
        WorkerResponse::from_entry(&key, &worker, available, reason)
    })
    .await
    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(resp))
}

async fn put_worker(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(worker): Json<WorkerPersona>,
) -> Result<Json<WorkerResponse>, axum::http::StatusCode> {
    let check = worker.availability_check.clone();
    let (available, reason) =
        tokio::task::spawn_blocking(move || worker_probe::probe_worker(&check))
            .await
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let response = WorkerResponse::from_entry(&key, &worker, available, reason);

    // Update in-memory config
    {
        let mut ac = state.agent_config.write().await;
        ac.workers.insert(key.clone(), worker);
        // Persist to disk
        let disk_config = agent_identity::AgentConfig {
            primary: {
                let p = state.persona.read().await;
                p.clone()
            },
            workers: ac.workers.clone(),
        };
        agent_identity::save_agent_config(&disk_config)
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    tracing::info!(
        target: "permagentd::agent",
        "Worker '{}' updated: {}",
        key,
        response.display_name
    );

    Ok(Json(response))
}

async fn delete_worker(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<axum::http::StatusCode, axum::http::StatusCode> {
    let mut ac = state.agent_config.write().await;
    if ac.workers.remove(&key).is_none() {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }

    let disk_config = agent_identity::AgentConfig {
        primary: {
            let p = state.persona.read().await;
            p.clone()
        },
        workers: ac.workers.clone(),
    };
    agent_identity::save_agent_config(&disk_config)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(target: "permagentd::agent", "Worker '{}' removed", key);
    Ok(axum::http::StatusCode::OK)
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/agent/workers", get(list_workers))
        .route("/api/agent/workers/{key}", get(get_worker))
        .route("/api/agent/workers/{key}", put(put_worker))
        .route("/api/agent/workers/{key}", delete(delete_worker))
        .with_state(state)
}
