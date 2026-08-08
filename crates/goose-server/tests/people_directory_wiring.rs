//! People directory wiring — `GET /api/people/directory` and `POST /api/people`
//! against an AppState with a REAL spectral Brain mounted in a throwaway
//! PERMAGENT_PATH_ROOT.
//!
//! This lives here rather than in `routes::people`'s unit tests for a specific
//! reason: in that harness no ontology is written, so `state.brain` is `None`,
//! and `overlay_graph_attributes` returns immediately after clearing every
//! attribute. An assertion that a field survived the overlay would pass there
//! for entirely the wrong reason — it would be asserting `None == None`. The
//! overlay is only observable with a Brain mounted.
//!
//! One test per integration binary (own process): PERMAGENT_PATH_ROOT is
//! process-wide, per the decision_wiring precedent.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_creates_a_person_and_serves_graph_attributes() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());
    let brain_dir = permagent::config::paths::Paths::brain_dir();
    std::fs::create_dir_all(&brain_dir).unwrap();
    std::fs::write(
        permagent::config::paths::Paths::brain_ontology(),
        ONTOLOGY_TOML,
    )
    .unwrap();

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    state
        .brain
        .clone()
        .expect("AppState must mount the Brain when brain_dir + ontology exist");
    let app = permagent_daemon::routes::people::routes(state.clone());

    // ── Create through the router, with a field, in one call ──
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/people")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "display_name": "Directory Tester",
                        "fields": { "company": "Atlas Atlantic" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    assert_eq!(created["created"], serde_json::Value::Bool(true));
    let uuid = created["person"]["entity_uuid"]
        .as_str()
        .unwrap()
        .to_string();

    // ── The directory serves that field back ──
    //
    // This is the real assertion: `company` has no people-table column value —
    // the create path writes it to graph `entity_fields`, and the overlay
    // clears the columns before refilling. Seeing it here proves the directory
    // ran the overlay rather than reading stale columns.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/people/directory")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = body_json(resp).await;
    let row = rows
        .as_array()
        .expect("directory returns an array")
        .iter()
        .find(|r| r["entity_uuid"] == serde_json::Value::String(uuid.clone()))
        .expect("the created person appears in the directory");
    assert_eq!(
        row["company"],
        serde_json::Value::String("Atlas Atlantic".into())
    );

    // Associated with no project — the cohort the directory exists to surface.
    assert_eq!(row["projects"], serde_json::json!([]));

    // ── Idempotent by name: the second create resolves to the same person ──
    //
    // `created: false` is the only signal a caller gets that their "new" person
    // was already in the directory. If this ever silently reported true, the UI
    // would show a success toast for a person it did not create.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/people")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "display_name": "Directory Tester" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let again = body_json(resp).await;
    assert_eq!(again["created"], serde_json::Value::Bool(false));
    assert_eq!(
        again["person"]["entity_uuid"],
        serde_json::Value::String(uuid.clone())
    );

    // ── The reverse query is mounted and empty for an unassociated person ──
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/people/{uuid}/projects"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await, serde_json::json!([]));
}
