use crate::state::AppState;
use axum::{
    extract::{Path, State},
    routing::{delete, get, put},
    Json, Router,
};
use permagent::config::agent_identity::{self, WorkerPersona};
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
}

impl WorkerResponse {
    fn from_entry(key: &str, w: &WorkerPersona) -> Self {
        Self {
            key: key.to_string(),
            first_name: w.first_name.clone(),
            last_name: w.last_name.clone(),
            nickname: w.nickname.clone(),
            display_name: w.display_name(),
            role: w.role.clone(),
            traits: w.traits.clone(),
            tone: w.tone.clone(),
        }
    }
}

async fn list_workers(
    State(state): State<Arc<AppState>>,
) -> Json<HashMap<String, WorkerResponse>> {
    let ac = state.agent_config.read().await;
    let map = ac
        .workers
        .iter()
        .map(|(k, v)| (k.clone(), WorkerResponse::from_entry(k, v)))
        .collect();
    Json(map)
}

async fn get_worker(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> Result<Json<WorkerResponse>, axum::http::StatusCode> {
    let ac = state.agent_config.read().await;
    ac.workers
        .get(&key)
        .map(|w| Json(WorkerResponse::from_entry(&key, w)))
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

async fn put_worker(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(worker): Json<WorkerPersona>,
) -> Result<Json<WorkerResponse>, axum::http::StatusCode> {
    let response = WorkerResponse::from_entry(&key, &worker);

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
