//! Ollama status and control routes.
//! Proxies to the local Ollama instance for model state queries and warm-load.

use crate::routes::errors::ErrorResponse;
use crate::state::AppState;
use axum::{
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const OLLAMA_BASE: &str = "http://localhost:11434";

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub modified_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaRunningModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub expires_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaPsResponse {
    #[serde(default)]
    models: Vec<OllamaRunningModel>,
}

/// Combined status for the frontend
#[derive(Debug, Serialize)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub installed: Vec<OllamaModelInfo>,
    pub running: Vec<OllamaRunningModel>,
}

#[derive(Debug, Deserialize)]
pub struct WarmLoadRequest {
    pub model: String,
    /// How long to keep the model loaded, in seconds
    pub keep_alive_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct WarmLoadResponse {
    pub success: bool,
    pub model: String,
    pub keep_alive_secs: u64,
}

// ── Handlers ────────────────────────────────────────────────────────

/// GET /api/ollama/status — combined installed + running state
async fn ollama_status() -> Json<OllamaStatus> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let tags = client.get(format!("{}/api/tags", OLLAMA_BASE)).send().await;
    let ps = client.get(format!("{}/api/ps", OLLAMA_BASE)).send().await;

    let installed = match tags {
        Ok(resp) => resp
            .json::<OllamaTagsResponse>()
            .await
            .map(|r| r.models)
            .unwrap_or_default(),
        Err(_) => vec![],
    };

    let running = match ps {
        Ok(resp) => resp
            .json::<OllamaPsResponse>()
            .await
            .map(|r| r.models)
            .unwrap_or_default(),
        Err(_) => vec![],
    };

    let reachable = !installed.is_empty() || {
        // If tags returned empty but didn't error, Ollama is reachable
        client
            .get(format!("{}/api/tags", OLLAMA_BASE))
            .send()
            .await
            .is_ok()
    };

    Json(OllamaStatus {
        reachable,
        installed,
        running,
    })
}

/// POST /api/ollama/warm — warm-load a model with keep_alive duration
async fn ollama_warm(
    Json(req): Json<WarmLoadRequest>,
) -> Result<Json<WarmLoadResponse>, ErrorResponse> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| ErrorResponse::internal(format!("HTTP client error: {}", e)))?;

    let body = serde_json::json!({
        "model": req.model,
        "prompt": "ok",
        "stream": false,
        "keep_alive": format!("{}s", req.keep_alive_secs),
        "options": { "num_predict": 1 }
    });

    let resp = client
        .post(format!("{}/api/generate", OLLAMA_BASE))
        .json(&body)
        .send()
        .await
        .map_err(|e| ErrorResponse::internal(format!("Ollama unreachable: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(ErrorResponse::internal(format!(
            "Ollama warm-load failed ({}): {}",
            status, text
        )));
    }

    tracing::info!(model = %req.model, keep_alive = req.keep_alive_secs, "Ollama model warm-loaded");

    Ok(Json(WarmLoadResponse {
        success: true,
        model: req.model,
        keep_alive_secs: req.keep_alive_secs,
    }))
}

// ── Router ──────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/ollama/status", get(ollama_status))
        .route("/api/ollama/warm", post(ollama_warm))
        .with_state(state)
}
