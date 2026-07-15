//! Integration-wiring proof for the Downloads inbox (#392/#393), through the
//! real HTTP router (`routes::inbox::routes`) against an AppState rooted at a
//! throwaway PERMAGENT_PATH_ROOT.
//!
//! Proves the exact defect class the wiring audit flagged for #4 — a described
//! surface that could not populate: record a file via `POST /api/inbox`, then
//! confirm `GET /api/inbox` (the list endpoint the new inbox panel calls)
//! returns it. Before this thread there was no consumer of the list endpoint.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// #[serial]: mutates the process-global PERMAGENT_PATH_ROOT env var and builds a
// full AppState, which races other AppState-building tests under parallel cargo
// test (see the appstate-tests-must-be-serial regression).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn post_then_get_inbox_roundtrips_through_router() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());

    // Starts empty.
    let resp = app.clone().oneshot(get("/api/inbox")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    assert_eq!(
        listed.as_array().map(|a| a.len()),
        Some(0),
        "inbox starts empty"
    );

    // Record a download via POST (mirrors the desktop download bridge).
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({
                "filename": "invoice.pdf",
                "original_url": "https://example.com/invoice.pdf",
                "content_type": "application/pdf",
                "size_bytes": 2048,
                "disk_path": "invoice.pdf"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    assert_eq!(created["filename"], "invoice.pdf");
    assert_eq!(created["status"], "received");

    // GET now lists it — this is the endpoint the inbox panel calls.
    let resp = app.oneshot(get("/api/inbox")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    let arr = listed.as_array().expect("array body");
    assert_eq!(arr.len(), 1, "the recorded file must be listed");
    assert_eq!(arr[0]["filename"], "invoice.pdf");
    assert_eq!(arr[0]["original_url"], "https://example.com/invoice.pdf");
    assert_eq!(arr[0]["size_bytes"], 2048);
}
