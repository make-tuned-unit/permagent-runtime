//! Act-on-page routes (#649 / #622): a11y-ref snapshot + click/type/select.
//!
//! Extends the read-page seam (`browser_content.rs`) with two round-trips the
//! agent grounds actions on. Each mirrors the read path exactly: the in-process
//! MCP tool POSTs here, we mint a UUID-keyed request, emit an event the frontend
//! bridge listens for, then block on a per-request oneshot until the frontend
//! injects the grounding script and POSTs the result back. Reusing
//! [`PendingBridge`] keeps the correlation machinery identical to the read path.
//!
//! Same rails as the read path: loopback-only (`require_loopback`), transient
//! (results are handed to the agent for the turn, never persisted). The web-scheme
//! guard is enforced in-page by the injected grounding script — the agent cannot
//! snapshot or act on a `file://` / custom-scheme page.

use crate::routes::browser_content::PendingBridge;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One interactive element, grounded on a stable `data-permagent-ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotElement {
    #[serde(rename = "ref")]
    pub ref_id: u32,
    pub role: String,
    pub name: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// The compact interactive-element list for the open page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub url: String,
    /// Concrete native webview that produced these refs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webview_id: Option<String>,
    pub elements: Vec<SnapshotElement>,
    #[serde(default)]
    pub truncated: bool,
    /// "ok", "no_tab", "refused_scheme" (non-http(s) page), or "error".
    #[serde(default = "default_status")]
    pub status: String,
    /// The snapshot generation these refs were stamped in. An act presents it
    /// back so refs superseded by ANOTHER session's snapshot of the same shared
    /// webview fail loudly instead of resolving to a restamped element (#939).
    #[serde(default)]
    pub generation: String,
}

/// Result of an act. On success carries a FRESH snapshot so the caller re-grounds
/// on post-action refs and never acts on a stale one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<PageSnapshot>,
}

fn default_status() -> String {
    "ok".to_string()
}

/// Correlates `get_page_snapshot` round-trips.
pub type SnapshotBridge = PendingBridge<PageSnapshot>;
/// Correlates `act_on_page` round-trips.
pub type ActBridge = PendingBridge<ActResult>;

/// Incoming act request from the MCP tool.
#[derive(Debug, Deserialize)]
pub struct ActRequest {
    #[serde(rename = "ref")]
    pub ref_id: u32,
    pub action: String,
    #[serde(default)]
    pub value: Option<String>,
    pub webview_id: String,
    pub page_url: String,
    /// Generation the caller's refs came from (#939). Absent from an older
    /// caller means "no generation check" — the page-URL guard still applies.
    #[serde(default)]
    pub generation: Option<String>,
}

/// Validate an act: `click` takes no value; `type` requires a value present
/// (empty is allowed — clearing a field); `select` requires a non-empty value.
/// Any other action is rejected. This is the fail-closed enforcement point.
pub fn validate_act(action: &str, value: Option<&str>) -> Result<(), String> {
    match action {
        "click" => Ok(()),
        "type" => value
            .map(|_| ())
            .ok_or_else(|| "action 'type' requires a value".to_string()),
        "select" => match value {
            Some(v) if !v.is_empty() => Ok(()),
            _ => Err("action 'select' requires a non-empty value".to_string()),
        },
        other => Err(format!(
            "unknown action '{other}' (use click, type, or select)"
        )),
    }
}

/// POST /api/browser/snapshot/read — the MCP tool asks for the interactive-element
/// snapshot of the open page. Emits BrowserSnapshotRequested, blocks until the
/// frontend POSTs the stamped refs back.
async fn read_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PageSnapshot>, StatusCode> {
    // `slot` owns the pending entry — see PendingSlot: an abandoned request
    // releases it on drop instead of leaking the map entry.
    let (slot, rx) = state.browser_snapshot_bridge.request();

    permagent::events::emit(permagent::events::PermagentEvent::new(
        permagent::events::PermagentEventType::BrowserSnapshotRequested,
        serde_json::json!({ "request_id": slot.id() }),
    ));

    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(snapshot)) => Ok(Json(snapshot)),
        Ok(Err(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(_) => Err(StatusCode::GATEWAY_TIMEOUT),
    }
}

/// POST /api/browser/snapshot/:request_id — frontend delivers the snapshot.
async fn fulfill_snapshot(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(snapshot): Json<PageSnapshot>,
) -> StatusCode {
    if state.browser_snapshot_bridge.fulfill(&request_id, snapshot) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

/// POST /api/browser/act — the MCP tool asks to act on a ref. The event payload
/// carries ref/action/value so the frontend can perform it; the fresh snapshot
/// comes back through the oneshot.
async fn act(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ActRequest>,
) -> Result<Json<ActResult>, StatusCode> {
    if validate_act(&req.action, req.value.as_deref()).is_err() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let (slot, rx) = state.browser_act_bridge.request();

    permagent::events::emit(permagent::events::PermagentEvent::new(
        permagent::events::PermagentEventType::BrowserActRequested,
        serde_json::json!({
            "request_id": slot.id(),
            "ref": req.ref_id,
            "action": req.action,
            "value": req.value,
            "webview_id": req.webview_id,
            "page_url": req.page_url,
            "generation": req.generation,
        }),
    ));

    match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
        Ok(Ok(result)) => Ok(Json(result)),
        Ok(Err(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(_) => Err(StatusCode::GATEWAY_TIMEOUT),
    }
}

/// POST /api/browser/act/:request_id — frontend delivers the act result.
async fn fulfill_act(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(result): Json<ActResult>,
) -> StatusCode {
    if state.browser_act_bridge.fulfill(&request_id, result) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/browser/snapshot/read", post(read_snapshot))
        .route("/api/browser/snapshot/{request_id}", post(fulfill_snapshot))
        .route("/api/browser/act", post(act))
        .route("/api/browser/act/{request_id}", post(fulfill_act))
        // Loopback-only (#630), same rail as the read path: unauthenticated by
        // design (in-process MCP tool, no token), so never reachable once the
        // daemon binds a routable address for multi-device/tailnet.
        .layer(axum::middleware::from_fn(
            crate::middleware::loopback::require_loopback,
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_act_enforces_the_action_set_and_value_rules() {
        assert!(validate_act("click", None).is_ok());
        assert!(validate_act("click", Some("ignored")).is_ok());
        // type requires a value present; empty is allowed (clears the field).
        assert!(validate_act("type", Some("hello")).is_ok());
        assert!(validate_act("type", Some("")).is_ok());
        assert!(validate_act("type", None).is_err());
        // select requires a non-empty value.
        assert!(validate_act("select", Some("Option A")).is_ok());
        assert!(validate_act("select", Some("")).is_err());
        assert!(validate_act("select", None).is_err());
        // anything else is fail-closed.
        assert!(validate_act("submit", None).is_err());
        assert!(validate_act("navigate", Some("https://x")).is_err());
    }

    #[test]
    fn snapshot_element_serializes_ref_key_and_omits_absent_value() {
        let el = SnapshotElement {
            ref_id: 3,
            role: "link".into(),
            name: "Home".into(),
            tag: "a".into(),
            value: None,
        };
        let v = serde_json::to_value(&el).unwrap();
        assert_eq!(v["ref"], 3);
        assert_eq!(v["role"], "link");
        assert!(v.get("value").is_none(), "absent value must be omitted");
    }

    #[test]
    fn page_snapshot_round_trips_and_defaults_status() {
        // The frontend may omit status/truncated; they default.
        let json = r#"{"url":"https://e.com","elements":[{"ref":0,"role":"button","name":"Go","tag":"button","value":"unchecked"}]}"#;
        let snap: PageSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.status, "ok");
        assert!(!snap.truncated);
        assert_eq!(snap.elements.len(), 1);
        assert_eq!(snap.elements[0].ref_id, 0);
        assert_eq!(snap.elements[0].value.as_deref(), Some("unchecked"));
        // Round-trip preserves the ref key.
        let back = serde_json::to_string(&snap).unwrap();
        assert!(back.contains("\"ref\":0"));
    }

    #[test]
    fn act_request_reads_ref_key() {
        let req: ActRequest =
            serde_json::from_str(
                r#"{"ref":5,"action":"type","value":"hi","webview_id":"browser-1","page_url":"https://example.com/"}"#,
            )
            .unwrap();
        assert_eq!(req.ref_id, 5);
        assert_eq!(req.action, "type");
        assert_eq!(req.value.as_deref(), Some("hi"));
        assert_eq!(req.webview_id, "browser-1");
        assert_eq!(req.page_url, "https://example.com/");
        // Absent generation is tolerated — the URL guard still applies.
        assert_eq!(req.generation, None);
    }

    /// #939: the generation rides the act so a superseded ref is refused.
    #[test]
    fn act_request_carries_the_snapshot_generation() {
        let req: ActRequest = serde_json::from_str(
            r#"{"ref":3,"action":"click","webview_id":"browser-1","page_url":"https://example.com/","generation":"gen-7"}"#,
        )
        .unwrap();
        assert_eq!(req.generation.as_deref(), Some("gen-7"));
    }

    #[test]
    fn page_snapshot_round_trips_the_generation() {
        let json = r#"{"url":"https://e.com","elements":[],"generation":"gen-9"}"#;
        let snap: PageSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.generation, "gen-9");
        // A frontend that omits it degrades to the empty (unchecked) generation.
        let bare: PageSnapshot = serde_json::from_str(r#"{"url":"x","elements":[]}"#).unwrap();
        assert_eq!(bare.generation, "");
    }

    #[test]
    fn act_result_omits_none_fields() {
        let ok = ActResult {
            ok: true,
            error: None,
            snapshot: None,
        };
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v.get("error").is_none());
        assert!(v.get("snapshot").is_none());
    }
}
