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

impl Default for BrowserContentBridge {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl BrowserContentBridge {
    pub fn new() -> Self {
        Self::default()
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
/// Auth: unauthenticated but loopback-only — the tool runs inside the daemon
/// process and has no daemon token, so instead of a bearer check the whole
/// bridge is gated by `require_loopback` (#630): only same-box peers reach it,
/// regardless of bind host. A network-bound daemon (multi-device/tailnet) can't
/// be driven here from off-box.
///
/// TODO(mesh): If read_browser_content moves out-of-process (Mesh skill, remote
/// agent), the loopback assumption breaks — add Bearer token auth alongside.
async fn read_content(State(state): State<Arc<AppState>>) -> Result<Json<PageContent>, StatusCode> {
    let (request_id, rx) = state.browser_content_bridge.request().await;

    permagent::events::emit(permagent::events::PermagentEvent::new(
        permagent::events::PermagentEventType::BrowserContentRequested,
        serde_json::json!({ "request_id": request_id }),
    ));

    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(content)) => {
            // Transient only: page content is returned to the agent for the
            // current turn and is NOT persisted to Brain/Spectral. Reading a
            // private email must not silently write its contents anywhere.
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

#[derive(serde::Deserialize)]
struct NavigateRequest {
    url: String,
}

/// POST /api/browser/navigate — the agent asks the in-app browser to open a
/// URL (#567). Fire-and-forget: emits BrowserNavigateRequested; the frontend
/// bridge opens it in the Build tab. Only http(s) URLs are accepted — the
/// agent must not be able to drive file:// or custom schemes into the webview.
async fn navigate(Json(req): Json<NavigateRequest>) -> StatusCode {
    let url = req.url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    tracing::info!(target: "permagentd::browser", %url, "agent navigate request");
    permagent::events::emit(permagent::events::PermagentEvent::new(
        permagent::events::PermagentEventType::BrowserNavigateRequested,
        serde_json::json!({ "url": url }),
    ));
    StatusCode::ACCEPTED
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/browser/navigate", axum::routing::post(navigate))
        .route("/api/browser/content/read", post(read_content))
        .route("/api/browser/content/{request_id}", post(fulfill_content))
        // Loopback-only (#630): these routes are unauthenticated by design
        // (in-process MCP tool, no token), so they must never be reachable once
        // the daemon binds a routable address for multi-device/tailnet.
        .layer(axum::middleware::from_fn(
            crate::middleware::loopback::require_loopback,
        ))
        .with_state(state)
}
