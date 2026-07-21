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

// ── Routing (#395) ─────────────────────────────────────────────────────────

/// Happy paths through the real router: brain (Reader digest, status
/// `ingested`), scheduler (social_post card, status `routed` + project stamp),
/// project (document row via the shared docs write path). One test so the
/// expensive AppState build happens once.
///
/// HOME is pointed at the tempdir because the project-documents path writes
/// under `~/.permagent/project-docs/` via `dirs::home_dir()` (not
/// PERMAGENT_PATH_ROOT — an inherited #677 wart); this keeps test artifacts out
/// of the real home. Safe here: #[serial] and this binary's tests set their own
/// env.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn route_inbox_file_to_each_destination() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    std::env::set_var("HOME", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    // A real project target (create_project seeds default board columns, which
    // the scheduler card needs).
    let project = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Launch assets".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // A real file on disk in the (path-root-scoped) inbox dir + its row.
    let inbox_dir = permagent::config::paths::Paths::inbox_dir();
    std::fs::create_dir_all(&inbox_dir).unwrap();
    std::fs::write(
        inbox_dir.join("flyer.txt"),
        "Spring launch flyer copy: our new release lands Friday.",
    )
    .unwrap();
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({
                "filename": "flyer.txt",
                "content_type": "text/plain",
                "disk_path": "flyer.txt"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let file_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // 1) → Brain: Reader extracts the text and returns a digest; status flips
    //    to `ingested`. (No Brain/Ollama in tests: the digest summary falls
    //    back to truncation and persistence is a logged warn — the wire
    //    contract is what this pins.)
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/inbox/{file_id}/route"),
            serde_json::json!({ "destination": "brain" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["destination"], "brain");
    assert_eq!(body["file"]["status"], "ingested");
    assert!(
        body["summary"].as_str().is_some_and(|s| !s.is_empty()),
        "text file must yield a non-empty digest summary, got {body}"
    );

    // 2) → Scheduler: a social_post card lands on the project board with the
    //    attachment reference; status `routed`, project stamped.
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/inbox/{file_id}/route"),
            serde_json::json!({ "destination": "scheduler", "project_id": project.id }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["file"]["status"], "routed");
    assert_eq!(body["file"]["project_id"], project.id.as_str());
    let card_id = body["card_id"].as_str().expect("card id").to_string();
    let posts = permagent::cards::list_cards(&pool, &project.id, Some("social_post"), None)
        .await
        .unwrap();
    assert_eq!(posts.len(), 1, "the social_post card must exist");
    assert_eq!(posts[0].id, card_id);
    assert_eq!(posts[0].title, "flyer.txt");
    assert_eq!(
        posts[0].metadata_json["attachment"]["inbox_file_id"],
        file_id.as_str(),
        "card must reference the inbox file"
    );

    // 3) → Project: the file becomes a project document through the shared
    //    docs write path.
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/inbox/{file_id}/route"),
            serde_json::json!({ "destination": "project", "project_id": project.id }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let doc_id = body["document_id"].as_str().expect("document id");
    let docs = permagent::project_documents::list_documents(&pool, &project.id)
        .await
        .unwrap();
    assert_eq!(docs.len(), 1, "the project document row must exist");
    assert_eq!(docs[0].id, doc_id);
    assert_eq!(docs[0].filename, "flyer.txt");
    assert!(
        std::path::Path::new(&docs[0].path).exists(),
        "document bytes must be on disk at {}",
        docs[0].path
    );

    // The routed file is still listed — routing never deletes from the inbox.
    let resp = app.oneshot(get("/api/inbox")).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed.as_array().map(|a| a.len()), Some(1));
}

/// Guard rails: unknown file, unknown destination, project-scoped destination
/// without a project, bytes gone from disk, and disk_path traversal.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn route_inbox_file_guard_rails() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());

    // Unknown inbox id → 404.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox/nope/route",
            serde_json::json!({ "destination": "brain" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // A row whose bytes never landed on disk.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({ "filename": "ghost.pdf", "disk_path": "ghost.pdf" }),
        ))
        .await
        .unwrap();
    let ghost_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Unknown destination → 400.
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/inbox/{ghost_id}/route"),
            serde_json::json!({ "destination": "finder" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Project-scoped destinations without a project_id → 400.
    for dest in ["project", "scheduler"] {
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/inbox/{ghost_id}/route"),
                serde_json::json!({ "destination": dest }),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{dest} without project_id must 400"
        );
    }

    // Bytes missing on disk → 409, and the row's status is untouched.
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/inbox/{ghost_id}/route"),
            serde_json::json!({ "destination": "brain" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let resp = app.clone().oneshot(get("/api/inbox")).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(
        listed[0]["status"], "received",
        "failed route must not mark"
    );

    // A traversal disk_path must be refused before touching the filesystem.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({ "filename": "evil", "disk_path": "../evil.txt" }),
        ))
        .await
        .unwrap();
    let evil_id = body_json(resp).await["id"].as_str().unwrap().to_string();
    let resp = app
        .oneshot(post_json(
            &format!("/api/inbox/{evil_id}/route"),
            serde_json::json!({ "destination": "brain" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
