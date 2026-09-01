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
use std::sync::LazyLock;
use tower::ServiceExt;

/// Process-lifetime `PERMAGENT_PATH_ROOT` shared by every test in this binary.
///
/// The session-manager pool behind AppState is a process-global `LazyLock`
/// (`SESSION_STORAGE`) that pins its sqlite path — and `create_dir_all`s that
/// path's parent — to whatever `PERMAGENT_PATH_ROOT` resolves to the *first*
/// time any test builds an AppState. Each test used to mint its own
/// `tempfile::tempdir()` and set the root to it; that dir was deleted the moment
/// the first test returned, while the process-global pool kept pointing at it,
/// so the next `#[serial]` test's very first query opened a connection under a
/// vanished parent directory → "unable to open database file" (#840, the flake
/// that forced CI re-runs across the merge fleet). It only *flaked* rather than
/// always failing because the per-platform run order and lazy-connection reuse
/// decided whether a fresh connection had to be opened after the drop.
///
/// The fix: one tempdir, owned by this `static` so it lives for the whole
/// process (never dropped between tests), with the db/brain/inbox parents
/// created up front. Every test roots itself through [`test_root`] *before*
/// building an AppState, so the global pool is pinned to a directory that
/// outlives every test. HOME is pointed at the same root because the
/// project-documents write path resolves under `~/.permagent/…` via
/// `dirs::home_dir()` (an inherited #677 wart, not PERMAGENT_PATH_ROOT) — doing
/// it once here keeps every test's artifacts out of the real home.
static TEST_ROOT: LazyLock<tempfile::TempDir> = LazyLock::new(|| {
    let tmp = tempfile::tempdir().expect("create test PERMAGENT_PATH_ROOT");
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    std::env::set_var("HOME", tmp.path());
    // Pre-create every parent a lazily-opened db/brain/inbox handle might touch,
    // so no connection can ever race a missing directory.
    for dir in [
        permagent::config::paths::Paths::spectral_dir(),
        permagent::config::paths::Paths::brain_dir(),
        permagent::config::paths::Paths::inbox_dir(),
        permagent::config::paths::Paths::logs_dir(),
    ] {
        std::fs::create_dir_all(&dir).expect("create test root subdir");
    }
    tmp
});

/// Root this test in the shared, process-lifetime `PERMAGENT_PATH_ROOT`.
/// Call at the top of every test, before building an AppState.
fn test_root() {
    LazyLock::force(&TEST_ROOT);
}

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

/// Find one row by id in a `GET /api/inbox` array body.
fn row_by_id<'a>(listed: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    listed.as_array()?.iter().find(|r| r["id"] == id)
}

// #[serial]: mutates the process-global PERMAGENT_PATH_ROOT env var and builds a
// full AppState, which races other AppState-building tests under parallel cargo
// test (see the appstate-tests-must-be-serial regression).
//
// NOTE on assertions: the session-manager pool behind AppState is process-global,
// so every #[serial] test in this binary shares ONE accumulating database and
// the run order differs per platform. Assertions must therefore be id-scoped or
// baseline-relative — never absolute counts or positional indexing (that
// exact class of flake went red on CI: macOS ran the routing tests first and
// "starts empty" saw their rows).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn post_then_get_inbox_roundtrips_through_router() {
    test_root();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());

    // Baseline (the shared DB may already hold rows from sibling tests).
    let resp = app.clone().oneshot(get("/api/inbox")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let baseline = body_json(resp).await.as_array().map(|a| a.len()).unwrap();

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
    let created_id = created["id"].as_str().unwrap().to_string();

    // GET now lists it — this is the endpoint the inbox panel calls.
    let resp = app.oneshot(get("/api/inbox")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    assert_eq!(
        listed.as_array().map(|a| a.len()),
        Some(baseline + 1),
        "exactly the recorded file was added"
    );
    let row = row_by_id(&listed, &created_id).expect("the recorded file must be listed");
    assert_eq!(row["filename"], "invoice.pdf");
    assert_eq!(row["original_url"], "https://example.com/invoice.pdf");
    assert_eq!(row["size_bytes"], 2048);
}

// ── Routing (#395) ─────────────────────────────────────────────────────────

/// Happy paths through the real router: brain (Reader digest, status
/// `ingested`), scheduler (social_post card, status `routed` + project stamp),
/// project (document row via the shared docs write path). One test so the
/// expensive AppState build happens once.
///
/// The project-documents path writes under `~/.permagent/project-docs/` via
/// `dirs::home_dir()` (not PERMAGENT_PATH_ROOT — an inherited #677 wart);
/// [`test_root`] points HOME at the shared throwaway root so those artifacts
/// stay out of the real home.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn route_inbox_file_to_each_destination() {
    test_root();

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
    let row = row_by_id(&listed, &file_id).expect("routed file must still be listed");
    assert_eq!(row["status"], "routed");
}

/// Guard rails: unknown file, unknown destination, project-scoped destination
/// without a project, bytes gone from disk, and disk_path traversal.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn route_inbox_file_guard_rails() {
    test_root();

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
    let ghost = row_by_id(&listed, &ghost_id).expect("ghost row listed");
    assert_eq!(ghost["status"], "received", "failed route must not mark");

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

// ── #2: JSON errors from inbox routes ───────────────────────────────────────

/// FAILS BEFORE: `routes/inbox.rs` returned axum's bare `(StatusCode, String)`
/// on every rejection, which serializes as `text/plain` with the string as
/// the whole body. `apiFetch` (`ui/command-center/src/lib/api.ts`) only ever
/// calls `response.json()` on a non-2xx, so that body failed to parse and
/// every inbox failure surfaced in the UI as the generic "Unknown error" — the
/// real reason, and the filename it was about, never reached the user. Now
/// every handler returns `ErrorResponse`, the same JSON `{ "message": … }`
/// shape the rest of the daemon's routes use.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rejected_route_answers_json_naming_the_file() {
    test_root();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());

    // A row whose bytes never landed on disk — routing it 409s.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({
                "filename": "vanished-report.pdf",
                "disk_path": "vanished-report.pdf"
            }),
        ))
        .await
        .unwrap();
    let file_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(post_json(
            &format!("/api/inbox/{file_id}/route"),
            serde_json::json!({ "destination": "brain" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "expected a JSON content-type, got {content_type:?}"
    );
    let body = body_json(resp).await;
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("vanished-report.pdf"),
        "the error message must name the file that failed to route, got {body}"
    );
}

// ── #5: the one writer announces on every caller ────────────────────────────

/// FAILS BEFORE: `save_project_document` (the write path both the multipart
/// upload route and this inbox `project` route share) never emitted
/// `project_changed`; only the two HTTP handlers around it did — the upload
/// handler after a successful multipart POST, and the delete handler. The
/// inbox route called the shared function directly and announced nothing, so
/// a project detail view already open in a second client never refreshed its
/// Documents panel after a file was routed to it from the inbox.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn routing_to_a_project_announces_documents_changed() {
    test_root();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    let project = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Doc drop".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let inbox_dir = permagent::config::paths::Paths::inbox_dir();
    std::fs::create_dir_all(&inbox_dir).unwrap();
    std::fs::write(inbox_dir.join("brief.txt"), "brief contents").unwrap();
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({
                "filename": "brief.txt",
                "content_type": "text/plain",
                "disk_path": "brief.txt"
            }),
        ))
        .await
        .unwrap();
    let file_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    let mut rx = permagent::events::subscribe();
    let resp = app
        .oneshot(post_json(
            &format!("/api/inbox/{file_id}/route"),
            serde_json::json!({ "destination": "project", "project_id": project.id }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let mut saw = false;
    while let Ok(evt) = rx.try_recv() {
        if evt.event_type == permagent::events::PermagentEventType::ProjectChanged
            && evt.payload["project_id"] == project.id.as_str()
            && evt.payload["change"] == "documents"
        {
            saw = true;
        }
    }
    assert!(
        saw,
        "routing an inbox file to a project must emit project_changed(id, \"documents\")"
    );
}

// ── #6: download feedback ───────────────────────────────────────────────────

/// FAILS BEFORE: `create_inbox_handler` emitted nothing at all — a file
/// landing in the Downloads inbox produced no bus frame, so the client had no
/// way to toast "you got a file" short of polling `GET /api/inbox`.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn creating_an_inbox_row_announces_arrival() {
    test_root();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let app = permagent_daemon::routes::inbox::routes(state.clone());

    let mut rx = permagent::events::subscribe();
    let resp = app
        .oneshot(post_json(
            "/api/inbox",
            serde_json::json!({
                "filename": "landed.pdf",
                "disk_path": "landed.pdf",
                "size_bytes": 99
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let mut saw = false;
    while let Ok(evt) = rx.try_recv() {
        if evt.event_type == permagent::events::PermagentEventType::InboxFileReceived
            && evt.payload["filename"] == "landed.pdf"
        {
            saw = true;
        }
    }
    assert!(
        saw,
        "POST /api/inbox must emit inbox_file_received naming the file"
    );
}
