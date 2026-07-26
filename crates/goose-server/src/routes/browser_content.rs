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

/// Shared bridge for pending browser round-trip requests.
///
/// Each request gets its own UUID-keyed oneshot channel, so concurrent calls are
/// fully independent — they don't share a slot or interfere with each other's
/// timeouts. Generic over the payload `T` so the read path (`PageContent`) and
/// the act-on-page paths (snapshot / act results, see `browser_act.rs`) reuse the
/// same correlation machinery.
pub struct PendingBridge<T> {
    pending: Mutex<HashMap<String, oneshot::Sender<T>>>,
}

impl<T> Default for PendingBridge<T> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> PendingBridge<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending request; returns its id and the receiver to await.
    pub async fn request(&self) -> (String, oneshot::Receiver<T>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        (id, rx)
    }

    /// Deliver a result to the awaiting request. Returns false when the id is
    /// unknown (already fulfilled, or the request timed out and was cancelled).
    pub async fn fulfill(&self, request_id: &str, value: T) -> bool {
        if let Some(tx) = self.pending.lock().await.remove(request_id) {
            tx.send(value).is_ok()
        } else {
            false
        }
    }

    /// Drop a pending request (e.g. after a timeout) so the slot doesn't leak.
    pub async fn cancel(&self, request_id: &str) {
        self.pending.lock().await.remove(request_id);
    }
}

/// The read-page bridge (#612): correlates `read_browser_content` round-trips.
pub type BrowserContentBridge = PendingBridge<PageContent>;

/// The web-scheme rail shared by every agent → webview entry point: only http(s)
/// may be driven into the in-app browser — never `file://` or a custom scheme.
/// The act/snapshot paths enforce the same rail in-page (`__permagentIsWebScheme`
/// in `browser_grounding.js`); this is its canonical, unit-tested expression.
pub fn is_allowed_web_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
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
            state.browser_content_bridge.cancel(&request_id).await;
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
    if !is_allowed_web_url(url) {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    // Enforce the control-plane guard at the trust boundary (the route), not only
    // in the MCP-tool caller — a direct POST must not be able to open the
    // Permagent control-plane origin in the in-app webview.
    if permagent::agents::platform_extensions::browser::reject_control_plane(url).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_scheme_guard_allows_only_http_s() {
        assert!(is_allowed_web_url("https://example.com"));
        assert!(is_allowed_web_url("http://example.com/path?q=1"));
        assert!(is_allowed_web_url("  https://trimmed.example  "));
        // The class the guard exists to reject: no local files, no custom schemes.
        assert!(!is_allowed_web_url("file:///etc/passwd"));
        assert!(!is_allowed_web_url("about:blank"));
        assert!(!is_allowed_web_url("javascript:alert(1)"));
        assert!(!is_allowed_web_url("data:text/html,<h1>x"));
        assert!(!is_allowed_web_url("permagent://build"));
        assert!(!is_allowed_web_url("ftp://example.com"));
        assert!(!is_allowed_web_url(""));
        assert!(!is_allowed_web_url("example.com"));
    }

    #[tokio::test]
    async fn bridge_fulfills_the_matching_request() {
        let bridge: PendingBridge<u32> = PendingBridge::new();
        let (id, rx) = bridge.request().await;
        assert!(bridge.fulfill(&id, 42).await, "known id must fulfill");
        assert_eq!(rx.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn bridge_reports_unknown_and_cancelled_ids() {
        let bridge: PendingBridge<u32> = PendingBridge::new();
        assert!(!bridge.fulfill("nope", 1).await, "unknown id must be false");

        let (id, rx) = bridge.request().await;
        bridge.cancel(&id).await;
        // After cancel the slot is gone: fulfill is a no-op and the receiver drops.
        assert!(!bridge.fulfill(&id, 7).await, "cancelled id must be false");
        assert!(rx.await.is_err(), "sender dropped -> receiver errors");
    }
}
