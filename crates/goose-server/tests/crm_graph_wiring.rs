//! #595 integration wiring — associate→disassociate through the real HTTP
//! router (`routes::projects::routes`) against an AppState with a REAL spectral
//! Brain mounted in a throwaway PERMAGENT_PATH_ROOT.
//!
//! Proves, at the daemon seam, both halves of the issue:
//!   * POST /api/projects/{id}/people on a NON-ontology project mints the
//!     project's graph identity (runtime provenance + node + bridge column)
//!     and asserts the `works_on` triple;
//!   * DELETE /api/projects/{id}/people/{uuid} removes the association row AND
//!     the `works_on` triple — no graph residue — while both entity nodes
//!     (identity, not residue) survive.
//!
//! One test per integration binary (own process): PERMAGENT_PATH_ROOT is
//! process-wide, so this follows the decision_wiring precedent (superseding
//! the older #[serial] guidance in #695).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use permagent::identity::canonical::graph_entity_id_hex;
use permagent::people_provenance::Provenance;

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.unwrap()
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("DELETE")
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn associate_disassociate_roundtrip_cleans_graph_residue() {
    // Throwaway data root WITH a Brain: state.rs mounts one when brain_dir and
    // ontology.toml both exist at startup.
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
    let brain = state
        .brain
        .clone()
        .expect("AppState must mount the Brain when brain_dir + ontology exist");
    let app = permagent_daemon::routes::projects::routes(state.clone());
    let pool = state.session_manager().pool_clone().await.unwrap();

    // A person with graph identity (the #583 runtime-create path — there is no
    // HTTP create-person route; Henry/UI call this same function).
    let person = permagent::people_create::create_person(
        &pool,
        &brain,
        "Wiring Tester",
        Provenance::Runtime,
    )
    .await
    .unwrap();
    let person_gid = person.graph_entity_id.clone().expect("person graph id");

    // ── Create a NON-ontology project through the router ──
    let resp = send(
        &app,
        post_json(
            "/api/projects",
            serde_json::json!({"name": "Wireframe Skunk"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let project = body_json(resp).await;
    let project_id = project["id"].as_str().unwrap().to_string();

    // ── Associate: 201, identity minted, works_on asserted ──
    let resp = send(
        &app,
        post_json(
            &format!("/api/projects/{project_id}/people"),
            serde_json::json!({"entityUuid": person.entity_uuid}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let expected_gid = graph_entity_id_hex("project", "Wireframe Skunk");
    let stored_gid: Option<String> =
        sqlx::query_scalar("SELECT graph_entity_id FROM projects WHERE id = ?")
            .bind(&project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored_gid.as_deref(),
        Some(expected_gid.as_str()),
        "non-ontology project must get its graph identity on associate (#595)"
    );
    let provenance: Option<String> =
        sqlx::query_scalar("SELECT source FROM entity_provenance WHERE entity_id_hex = ?")
            .bind(&expected_gid)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        provenance.as_deref(),
        Some("runtime"),
        "minted project node must be reconciler-protected"
    );

    let person_eid: spectral::core::entity_id::EntityId = person_gid.parse().unwrap();
    let project_eid: spectral::core::entity_id::EntityId = expected_gid.parse().unwrap();
    {
        let store = spectral::graph::graph_store::GraphStore::open_read_only(
            &brain_dir.join("graph.sqlite"),
        )
        .unwrap();
        assert_eq!(
            store
                .find_triples(Some(&person_eid), Some(&project_eid), Some("works_on"))
                .unwrap()
                .len(),
            1,
            "associate must assert the works_on triple"
        );
    }

    // ── Disassociate: 200, row gone, triple gone, nodes kept ──
    let resp = send(
        &app,
        delete(&format!(
            "/api/projects/{project_id}/people/{}",
            person.entity_uuid
        )),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM project_people WHERE project_id = ? AND entity_uuid = ?",
    )
    .bind(&project_id)
    .bind(&person.entity_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "association row deleted");

    {
        let store = spectral::graph::graph_store::GraphStore::open_read_only(
            &brain_dir.join("graph.sqlite"),
        )
        .unwrap();
        assert!(
            store
                .find_triples(Some(&person_eid), Some(&project_eid), Some("works_on"))
                .unwrap()
                .is_empty(),
            "disassociate must delete the works_on triple (#595 half 1)"
        );
        // Identity survives: nodes are not residue.
        assert!(store.get_entity(&person_eid).unwrap().is_some());
        assert!(store.get_entity(&project_eid).unwrap().is_some());
    }

    // ── A second disassociate is a 404 (association already gone) ──
    let resp = send(
        &app,
        delete(&format!(
            "/api/projects/{project_id}/people/{}",
            person.entity_uuid
        )),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
