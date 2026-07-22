//! S2 (#428, epic #399): the tee ingest seam for supervised terminal sessions.
//!
//! The supervised Claude Code session lives in a Tauri-owned PTY (the visible
//! Build-tab terminal) in the APP process; the gate parser + session registry
//! (`permagent::agents::platform_extensions::terminal_supervision`) live in
//! the DAEMON. This route is the bridge: the Tauri PTY reader tees each raw
//! output chunk here (`ui/desktop/src-tauri/src/terminal.rs`), the registry
//! parses it and emits structured gate events to the bus.
//!
//! Push, deterministic, zero-LLM: nothing polls — cost is one localhost POST
//! per output burst of a SUPERVISED session only (plain terminals never tee).
//!
//! Auth: mounted in the protected router — the tee holds the daemon bearer
//! token (same token the app already uses). Unknown sessions return 404 so a
//! tee outliving a restarted daemon stops instead of spamming.

use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::agents::platform_extensions::terminal_supervision as registry;
use serde::{Deserialize, Serialize};

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

async fn ingest_output(
    Json(chunk): Json<OutputChunk>,
) -> Result<Json<registry::IngestReport>, (StatusCode, Json<ErrorBody>)> {
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
    Ok(Json(report))
}

async fn list_sessions() -> Json<Vec<registry::SessionSnapshot>> {
    Json(registry::list_sessions())
}

/// Stateless (the registry is process-wide, like the event bus) — mounted in
/// the PROTECTED router: the tee authenticates with the daemon bearer token.
pub fn routes() -> Router {
    Router::new()
        .route("/terminal/supervised/output", post(ingest_output))
        .route("/terminal/supervised/sessions", get(list_sessions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use permagent::agents::platform_extensions::terminal_supervision::{
        register_session, remove_session, session_snapshot, SupervisedSessionKind, SupervisedStatus,
    };
    use tower::ServiceExt;

    // Real captured gate line (from `providers/claude_code.rs` tests) and the
    // result line — the same fixtures the core parser tests pin.
    const GATE_LINE: &str = r#"{"type":"control_request","request_id":"perm_1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"path":"foo.txt","content":"hello"},"tool_use_id":"tu_1"}}"#;
    const RESULT_LINE: &str =
        r#"{"type":"result","result":"Done","usage":{"input_tokens":10,"output_tokens":5}}"#;

    fn unique_id(tag: &str) -> String {
        format!("sup-route-{tag}-{}", uuid::Uuid::new_v4())
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

    #[tokio::test]
    async fn output_chunk_attaches_pty_and_reports_gates() {
        let sid = unique_id("gates");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = routes();

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

        // The PTY was attached as the relay address on first sight.
        let snap = session_snapshot(&sid).unwrap();
        assert_eq!(snap.pty_session_id.as_deref(), Some("pty-route-1"));
        assert_eq!(snap.status, SupervisedStatus::Attached);
        assert_eq!(snap.pending_gates.len(), 1);

        // Follow-up chunks may address by PTY id alone.
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
        assert_eq!(
            session_snapshot(&sid).unwrap().status,
            SupervisedStatus::Completed
        );

        remove_session(&sid);
    }

    #[tokio::test]
    async fn unknown_session_is_404() {
        let app = routes();
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

    #[tokio::test]
    async fn eof_fails_an_unfinished_session() {
        let sid = unique_id("eof");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = routes();

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

    #[tokio::test]
    async fn sessions_listing_returns_registered_sessions() {
        let sid = unique_id("list");
        register_session(&sid, SupervisedSessionKind::Watched, "proj", "/tmp/p");
        let app = routes();

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
