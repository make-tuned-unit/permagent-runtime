//! S2 (#428) + S3 (#429), epic #399: the tee ingest seam for supervised
//! terminal sessions, now bridged into the Decision Inbox.
//!
//! The supervised Claude Code session lives in a Tauri-owned PTY (the visible
//! Build-tab terminal) in the APP process; the gate parser + session registry
//! (`permagent::agents::platform_extensions::terminal_supervision`) live in
//! the DAEMON. This route is the bridge: the Tauri PTY reader tees each raw
//! output chunk here (`ui/desktop/src-tauri/src/terminal.rs`), the registry
//! parses it and emits structured gate events to the bus, and (S3) every
//! detected gate is filed as a `session_gate` Decision-Inbox row — Tier 2
//! fail-closed until S4's classification — while gates that resolve outside
//! the inbox (hand-typed answer, session end) supersede their open card.
//!
//! Push, deterministic, zero-LLM: nothing polls — cost is one localhost POST
//! per output burst of a SUPERVISED session only (plain terminals never tee).
//! The inbox bridge is best-effort: a filing failure is reported in the
//! response (and logged), never a failed tee — the gate stays visible in the
//! session's tab and on the event bus regardless.
//!
//! Auth: mounted in the protected router — the tee holds the daemon bearer
//! token (same token the app already uses). Unknown sessions return 404 so a
//! tee outliving a restarted daemon stops instead of spamming.

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::agents::platform_extensions::terminal_supervision as registry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct OutputChunk {
    /// The loop session id (`sup-<uuid>`) from the `project_launch` payload.
    /// Optional so a future caller holding only the PTY id can still tee.
    #[serde(default)]
    pub supervised_session_id: Option<String>,
    /// The Tauri PTY id (`pty-<uuid>`) — recorded as the S5 relay address on
    /// first sight.
    #[serde(default)]
    pub pty_session_id: Option<String>,
    /// Raw PTY output chunk (may split lines/escapes anywhere; the scanner
    /// reassembles).
    #[serde(default)]
    pub data: String,
    /// True on the final frame, when the PTY closed. A session that dies
    /// without a `type:"result"` is failed through S1's completion seam.
    #[serde(default)]
    pub eof: bool,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// The ingest report plus what the S3 inbox bridge did with it — evidence,
/// not silence, for the tee side and tests. The tee only inspects the HTTP
/// status; the extra fields are additive.
#[derive(Serialize)]
struct IngestResponse {
    #[serde(flatten)]
    report: registry::IngestReport,
    /// `session_gate` decisions filed for gates detected by this chunk.
    decisions_filed: usize,
    /// Open `session_gate` decisions superseded (gate answered in the
    /// terminal / session ended).
    decisions_superseded: usize,
    /// Best-effort bridge failures (also logged). Never fails the tee.
    bridge_errors: Vec<String>,
}

async fn ingest_output(
    State(state): State<Arc<AppState>>,
    Json(chunk): Json<OutputChunk>,
) -> Result<Json<IngestResponse>, (StatusCode, Json<ErrorBody>)> {
    let session_id = registry::resolve_session_id(
        chunk.supervised_session_id.as_deref(),
        chunk.pty_session_id.as_deref(),
    )
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!(
                    "no supervised session for supervised_session_id={:?} / pty_session_id={:?}",
                    chunk.supervised_session_id, chunk.pty_session_id
                ),
            }),
        )
    })?;
    if let Some(pty) = chunk.pty_session_id.as_deref() {
        registry::attach_pty(&session_id, pty);
    }
    let report = registry::ingest_output(&session_id, &chunk.data, chunk.eof).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("supervised session '{session_id}' disappeared during ingest"),
            }),
        )
    })?;

    // S3 (#429): gate → Decision Inbox. Best-effort by design — the parse
    // already happened, the bus events are out, and the gate is visible in
    // the session's tab; a DB hiccup must not stop the tee (fail-stop there
    // would blind the whole loop, strictly worse than a missed card).
    let bridge = match state.session_manager().pool_clone().await {
        Ok(pool) => registry::bridge_report_to_inbox(&pool, &session_id, &report).await,
        Err(e) => {
            tracing::warn!(
                session_id = session_id.as_str(),
                "session-gate inbox bridge skipped: no DB pool ({e})"
            );
            registry::GateBridgeOutcome {
                errors: vec![format!("no DB pool: {e}")],
                ..Default::default()
            }
        }
    };

    Ok(Json(IngestResponse {
        report,
        decisions_filed: bridge.decisions_filed,
        decisions_superseded: bridge.decisions_superseded,
        bridge_errors: bridge.errors,
    }))
}

async fn list_sessions() -> Json<Vec<registry::SessionSnapshot>> {
    Json(registry::list_sessions())
}

/// Mounted in the PROTECTED router: the tee authenticates with the daemon
/// bearer token. Stateful since S3 — the inbox bridge needs the decisions DB
/// (the registry itself stays process-wide, like the event bus).
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/terminal/supervised/output", post(ingest_output))
        .route("/terminal/supervised/sessions", get(list_sessions))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use permagent::agents::platform_extensions::terminal_supervision::{
        register_session, remove_session, session_snapshot, SupervisedSessionKind, SupervisedStatus,
    };
    use serial_test::serial;
    use tower::ServiceExt;

    // Real captured gate line (from `providers/claude_code.rs` tests) and the
    // result line — the same fixtures the core parser tests pin.
    const GATE_LINE: &str = r#"{"type":"control_request","request_id":"perm_1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"path":"foo.txt","content":"hello"},"tool_use_id":"tu_1"}}"#;
    const RESULT_LINE: &str =
        r#"{"type":"result","result":"Done","usage":{"input_tokens":10,"output_tokens":5}}"#;

    fn unique_id(tag: &str) -> String {
        format!("sup-route-{tag}-{}", uuid::Uuid::new_v4())
    }

    async fn test_app() -> Router {
        // Builds AppState → #[serial] on every test using this (env_lock /
        // process-singleton session-store race, see #695).
        let state = AppState::new(true).await.unwrap();
        routes(state)
    }

    async fn post_chunk(app: &Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/terminal/supervised/output")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn output_chunk_attaches_pty_reports_gates_and_files_decisions() {
        let sid = unique_id("gates");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = test_app().await;

        let (status, body) = post_chunk(
            &app,
            serde_json::json!({
                "supervised_session_id": sid,
                "pty_session_id": "pty-route-1",
                "data": format!("{GATE_LINE}\r\n"),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["gates_detected"], 1);
        assert_eq!(body["completed"], false);
        // S3: the gate reached the Decision Inbox as a session_gate row.
        assert_eq!(
            body["bridge_errors"],
            serde_json::json!([]),
            "bridge must not error: {body}"
        );
        assert_eq!(body["decisions_filed"], 1);

        // The PTY was attached as the relay address on first sight.
        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.pty_session_id.as_deref(), Some("pty-route-1"));
        assert_eq!(snap.status, SupervisedStatus::Attached);
        assert_eq!(snap.pending_gates.len(), 1);

        // Follow-up chunks may address by PTY id alone. The result ends the
        // session with the gate unanswered → its inbox card is superseded.
        let (status, body) = post_chunk(
            &app,
            serde_json::json!({
                "pty_session_id": "pty-route-1",
                "data": format!("{RESULT_LINE}\n"),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["completed"], true);
        assert_eq!(body["decisions_superseded"], 1);
        assert_eq!(
            session_snapshot(&sid).unwrap().status,
            SupervisedStatus::Completed
        );

        remove_session(&sid);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn unknown_session_is_404() {
        let app = test_app().await;
        let (status, body) = post_chunk(
            &app,
            serde_json::json!({
                "supervised_session_id": "sup-route-unknown",
                "data": "x\n",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("sup-route-unknown"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn eof_fails_an_unfinished_session() {
        let sid = unique_id("eof");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = test_app().await;

        let (status, body) = post_chunk(
            &app,
            serde_json::json!({
                "supervised_session_id": sid,
                "data": "",
                "eof": true,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["failed"], true);
        assert_eq!(
            session_snapshot(&sid).unwrap().status,
            SupervisedStatus::Failed
        );
        remove_session(&sid);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn sessions_listing_returns_registered_sessions() {
        let sid = unique_id("list");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = test_app().await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/terminal/supervised/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let listed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["session_id"] == sid.as_str()));
        remove_session(&sid);
    }
}
