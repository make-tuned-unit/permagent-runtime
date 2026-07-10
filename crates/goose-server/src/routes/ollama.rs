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

/// POST /api/ollama/start — best-effort launch of a locally installed Ollama
/// (#381 one-click setup). On macOS try the app bundle first (`open -a
/// Ollama` starts the menubar server), then fall back to a detached
/// `ollama serve`. Returns whether a launch was attempted; the wizard polls
/// `/api/ollama/status` to observe it coming up.
async fn ollama_start() -> Json<serde_json::Value> {
    // The app bundle path (macOS): starts the full app incl. the server.
    #[cfg(target_os = "macos")]
    {
        if let Ok(status) = tokio::process::Command::new("open")
            .args(["-a", "Ollama"])
            .status()
            .await
        {
            if status.success() {
                tracing::info!("Ollama app launched via `open -a Ollama`");
                return Json(serde_json::json!({ "launched": true, "method": "app" }));
            }
        }
    }
    // CLI fallback (any platform): a detached `ollama serve`.
    let spawned = tokio::process::Command::new("ollama")
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_child) => {
            tracing::info!("Ollama started via detached `ollama serve`");
            Json(serde_json::json!({ "launched": true, "method": "serve" }))
        }
        Err(e) => {
            tracing::info!(error = %e, "Ollama not installed — cannot auto-start");
            Json(serde_json::json!({ "launched": false, "method": null }))
        }
    }
}

// ── Model pull with SSE progress (#381, salvaged from #137) ─────────────────

#[derive(Debug, Deserialize)]
pub struct PullModelRequest {
    pub model: String,
}

/// POST /api/ollama/pull — proxy an Ollama model pull with SSE progress.
/// Ollama streams NDJSON progress lines; each is forwarded as one SSE event
/// so the wizard's hardware step can render a live progress bar.
async fn ollama_pull(
    Json(req): Json<PullModelRequest>,
) -> Result<
    axum::response::Sse<
        impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ErrorResponse,
> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600)) // large models take a while
        .build()
        .map_err(|e| ErrorResponse::internal(format!("HTTP client error: {}", e)))?;

    let resp = client
        .post(format!("{}/api/pull", OLLAMA_BASE))
        .json(&serde_json::json!({ "name": req.model, "stream": true }))
        .send()
        .await
        .map_err(|e| ErrorResponse::internal(format!("Ollama unreachable: {}", e)))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ErrorResponse::internal(format!(
            "Ollama pull failed: {}",
            text
        )));
    }

    tracing::info!(model = %req.model, "Ollama model pull started");

    use futures::StreamExt;
    let byte_stream = resp.bytes_stream();

    let sse_stream = async_stream::stream! {
        let mut buffer = String::new();
        futures::pin_mut!(byte_stream);
        while let Some(chunk) = byte_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(newline_pos) = buffer.find('\n') {
                        // drain (not index-slicing): find() returns a char
                        // boundary, but clippy::string_slice is denied
                        // workspace-wide and drain sidesteps it entirely.
                        let line: String = buffer.drain(..=newline_pos).collect();
                        let line = line.trim_end();
                        if !line.trim().is_empty() {
                            yield Ok(axum::response::sse::Event::default().data(line.to_string()));
                        }
                    }
                }
                Err(e) => {
                    yield Ok(axum::response::sse::Event::default()
                        .event("error")
                        .data(format!("{{\"error\":\"{}\"}}", e)));
                    break;
                }
            }
        }
        if !buffer.trim().is_empty() {
            yield Ok(axum::response::sse::Event::default().data(buffer));
        }
    };

    Ok(axum::response::Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15)),
    ))
}

// ── Router ──────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/ollama/status", get(ollama_status))
        .route("/api/ollama/warm", post(ollama_warm))
        .route("/api/ollama/pull", post(ollama_pull))
        .route("/api/ollama/start", post(ollama_start))
        .with_state(state)
}
