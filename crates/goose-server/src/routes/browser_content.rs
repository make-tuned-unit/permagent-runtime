use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

/// Shared bridge for pending browser content requests.
///
/// Each request gets its own UUID-keyed oneshot channel, so concurrent
/// `read_browser_content` calls are fully independent — they don't share
/// a single slot or interfere with each other's timeouts.
pub struct BrowserContentBridge {
    pending: Mutex<HashMap<String, oneshot::Sender<PageContent>>>,
}

impl BrowserContentBridge {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub async fn request(&self) -> (String, oneshot::Receiver<PageContent>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        (id, rx)
    }

    pub async fn fulfill(&self, request_id: &str, content: PageContent) -> bool {
        if let Some(tx) = self.pending.lock().await.remove(request_id) {
            tx.send(content).is_ok()
        } else {
            false
        }
    }
}

/// Status distinguishes three failure modes for agent reasoning:
/// - "ok": content extracted successfully
/// - "no_tab": no browser tab is open
/// - "error": tab exists but extraction failed (blank page, JS error, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub title: String,
    pub url: String,
    pub content: String,
    /// "ok", "no_tab", or "error"
    /// Note: status "ok" with empty content can occur on about:blank, loading
    /// pages, or pages with content entirely in cross-origin iframes. The
    /// frontend bridge detects empty content and returns status "error" with
    /// an explanatory message, so this edge case is handled at the bridge layer.
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub truncated: bool,
}

fn default_status() -> String {
    "ok".to_string()
}

/// POST /api/browser/content/read — MCP tool calls this to request page content.
///
/// Emits a BrowserContentRequested event on the global bus, then blocks until
/// the frontend extracts content and POSTs it to the fulfill endpoint.
///
/// Auth: unauthenticated (localhost-only, called by in-process MCP tool).
/// This is deliberate — the tool runs inside the daemon process and doesn't
/// have access to the daemon token. Acceptable because the endpoint only
/// triggers a content read of the user's own browser tab.
///
/// TODO(mesh): If read_browser_content moves out-of-process (Mesh skill,
/// remote agent), this assumption breaks — add Bearer token auth.
async fn read_content(State(state): State<Arc<AppState>>) -> Result<Json<PageContent>, StatusCode> {
    let (request_id, rx) = state.browser_content_bridge.request().await;

    permagent::events::emit(permagent::events::PermagentEvent::new(
        permagent::events::PermagentEventType::BrowserContentRequested,
        serde_json::json!({ "request_id": request_id }),
    ));

    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(content)) => {
            // Persist successful reads to Spectral as a background task.
            // Key: browser:read:<sha256(url)> — deduplicates by URL.
            if content.status == "ok" && !content.content.is_empty() {
                if let Some(brain) = state.brain.as_ref() {
                    let brain = brain.clone();
                    let title = content.title.clone();
                    let url = content.url.clone();
                    // Truncate content for memory — store a readable summary, not the full page
                    let mem_content = {
                        let max = 2000;
                        let text = &content.content;
                        if text.len() > max {
                            let cut = text[..max]
                                .rfind('\n')
                                .unwrap_or(max);
                            format!("{}\n[truncated]", &text[..cut])
                        } else {
                            text.clone()
                        }
                    };
                    let remember_content =
                        format!("Page: {title}\nURL: {url}\n\n{mem_content}");
                    tokio::spawn(async move {
                        let url_for_key = url.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            url_for_key.hash(&mut hasher);
                            let hash = format!("{:x}", hasher.finish());
                            let key = format!("browser:read:{}", &hash[..12]);
                            brain.remember_with(
                                &key,
                                &remember_content,
                                spectral::RememberOpts {
                                    source: Some("browser".into()),
                                    visibility: spectral::Visibility::Private,
                                    ..Default::default()
                                },
                            )
                        })
                        .await;
                        match result {
                            Ok(Ok(r)) => {
                                tracing::info!(
                                    target: "permagentd::brain",
                                    memory_id = r.memory_id,
                                    url,
                                    "Browser page read persisted to Spectral"
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(
                                    target: "permagentd::brain",
                                    error = %e,
                                    url,
                                    "Failed to persist browser page read"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "permagentd::brain",
                                    error = %e,
                                    "Browser page persist panicked"
                                );
                            }
                        }
                    });
                }
            }
            Ok(Json(content))
        }
        Ok(Err(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(_) => {
            state
                .browser_content_bridge
                .pending
                .lock()
                .await
                .remove(&request_id);
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
    }
}

/// POST /api/browser/content/:request_id — frontend delivers extracted page content.
async fn fulfill_content(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(content): Json<PageContent>,
) -> StatusCode {
    if state
        .browser_content_bridge
        .fulfill(&request_id, content)
        .await
    {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/browser/content/read", post(read_content))
        .route("/api/browser/content/{request_id}", post(fulfill_content))
        .with_state(state)
}
