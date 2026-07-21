//! #595 — project graph identity + `works_on` residue, proven against a real
//! spectral Brain (this crate is the sanctioned home for Brain-dependent
//! integration tests; see src/lib.rs).
//!
//! Covers the two halves of the issue at the function level:
//!  1. a NON-ontology project (Projects-tab create) gets a runtime-minted graph
//!     identity — provenance-first, bridge column backfilled — and the
//!     associate→disassociate roundtrip leaves NO `works_on` residue;
//!  2. an ontology project resolves alias-aware to its curated identity (no
//!     runtime provenance), and the minted identity is immutable across a
//!     project rename.

use std::sync::Arc;

use permagent::brain_handle::SafeBrain;
use permagent::identity::canonical::graph_entity_id_hex;
use permagent::people_create;
use permagent::people_provenance::Provenance;
use permagent::project_graph;
use permagent::projects::{self, CreateProject, UpdateProject};
use permagent::session::spectral_schema;
use spectral::Brain;
use sqlx::{Pool, Sqlite};

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

/// A fresh Brain in its own tempdir (kept alive for the test), plus the raw
/// Arc for store-level verification and the graph.sqlite path for the
/// delete-side checks.
fn fresh_brain(descriptor: &str) -> (SafeBrain, Arc<Brain>, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY_TOML).unwrap();
    let _ = Box::leak(Box::new(temp)); // keep the dir for the whole test binary
    let raw = Arc::new(
        Brain::builder()
            .data_dir(&brain_path)
            .ontology_path(&ontology_path)
            .device_id(spectral::DeviceId::from_descriptor(descriptor))
            .build()
            .expect("test brain"),
    );
    let graph_db = brain_path.join("graph.sqlite");
    (SafeBrain::from_arc(raw.clone()), raw, graph_db)
}

async fn fresh_pool() -> Pool<Sqlite> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    spectral_schema::init_spectral_db(&pool).await.unwrap();
    pool
}

async fn provenance_of(pool: &Pool<Sqlite>, id_hex: &str) -> Option<String> {
    sqlx::query_scalar("SELECT source FROM entity_provenance WHERE entity_id_hex = ?")
        .bind(id_hex)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// Half 2 then half 1 of #595, end to end: a Projects-tab (non-ontology)
/// project gets a graph identity at associate time, the `works_on` edge is
/// asserted, and disassociate deletes exactly that edge — nodes survive, and
/// the pair can re-associate afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn non_ontology_roundtrip_leaves_no_residue() {
    let (brain, raw, graph_db) = fresh_brain("test-595-roundtrip");
    let pool = fresh_pool().await;

    // A person with a graph identity (the #583 runtime-create path).
    let person = people_create::create_person(&pool, &brain, "Zara Quorra", Provenance::Runtime)
        .await
        .unwrap();
    let person_gid = person.graph_entity_id.clone().expect("person graph id");

    // A project that is NOT in ontology.toml.
    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "Nebular Skunkworks".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        project.graph_entity_id.is_none(),
        "no graph identity until first needed"
    );

    // ── Ensure: runtime mint (provenance → node → bridge column) ──
    let gid = project_graph::ensure_project_graph_identity(&pool, &brain, &project)
        .await
        .unwrap()
        .expect("non-ontology project must mint a graph identity");
    assert_eq!(gid, graph_entity_id_hex("project", "Nebular Skunkworks"));
    assert_eq!(
        provenance_of(&pool, &gid).await.as_deref(),
        Some("runtime"),
        "runtime provenance must protect the node from the reconciler"
    );
    let reloaded = projects::get_project(&pool, &project.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.graph_entity_id.as_deref(), Some(gid.as_str()));
    let node_id: spectral::core::entity_id::EntityId = gid.parse().unwrap();
    let node = raw
        .store()
        .get_entity(&node_id)
        .unwrap()
        .expect("node materialized");
    assert_eq!(node.entity_type, "project");
    assert_eq!(node.canonical, "nebular skunkworks");

    // Idempotent: a second ensure returns the same id, mints nothing new.
    let gid2 = project_graph::ensure_project_graph_identity(&pool, &brain, &reloaded)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gid2, gid);

    // ── Associate: the works_on edge lands ──
    assert!(brain.assert_works_on_edge(&person_gid, &gid).await.unwrap());
    // Idempotent: re-assert is a no-op.
    assert!(!brain.assert_works_on_edge(&person_gid, &gid).await.unwrap());
    let person_id: spectral::core::entity_id::EntityId = person_gid.parse().unwrap();
    assert_eq!(
        raw.store()
            .find_triples(Some(&person_id), Some(&node_id), Some("works_on"))
            .unwrap()
            .len(),
        1
    );

    // ── Disassociate: residue deleted, identity (nodes) kept ──
    let candidates = project_graph::project_graph_id_candidates(&brain, &reloaded).await;
    assert!(candidates.contains(&gid));
    let deleted =
        project_graph::delete_works_on_triples(&graph_db, &person_gid, &candidates).unwrap();
    assert_eq!(deleted, 1, "exactly the works_on triple is removed");
    assert!(
        raw.store()
            .find_triples(Some(&person_id), Some(&node_id), Some("works_on"))
            .unwrap()
            .is_empty(),
        "no works_on residue after disassociate"
    );
    assert!(raw.store().get_entity(&person_id).unwrap().is_some());
    assert!(raw.store().get_entity(&node_id).unwrap().is_some());

    // ── Re-associate works after the roundtrip ──
    assert!(brain.assert_works_on_edge(&person_gid, &gid).await.unwrap());
}

/// An ontology project resolves alias-aware to its curated identity: no
/// runtime provenance is written, the curated node is materialized, and the
/// bridge column is backfilled with the ONTOLOGY id (not the name-derived one).
#[tokio::test(flavor = "multi_thread")]
async fn ontology_project_resolves_alias_aware() {
    let (brain, raw, _graph_db) = fresh_brain("test-595-ontology");
    let pool = fresh_pool().await;

    // "Permagent.AI" is an alias of ontology project "permagent".
    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "Permagent.AI".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let gid = project_graph::ensure_project_graph_identity(&pool, &brain, &project)
        .await
        .unwrap()
        .expect("ontology project must resolve");
    let curated = graph_entity_id_hex("project", "permagent");
    assert_eq!(gid, curated, "alias resolves to the curated identity");
    assert_ne!(
        gid,
        graph_entity_id_hex("project", "Permagent.AI"),
        "must NOT be the name-derived id"
    );
    assert_eq!(
        provenance_of(&pool, &gid).await,
        None,
        "curated entities carry no runtime provenance (reconciler owns them)"
    );
    let node_id: spectral::core::entity_id::EntityId = gid.parse().unwrap();
    let node = raw
        .store()
        .get_entity(&node_id)
        .unwrap()
        .expect("materialized");
    assert_eq!(node.canonical, "permagent");
    let reloaded = projects::get_project(&pool, &project.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.graph_entity_id.as_deref(), Some(curated.as_str()));
}

/// A minted identity is immutable: renaming the project does not re-mint, and
/// the residue-candidate set still covers the original identity so
/// disassociate-after-rename deletes the old edge.
#[tokio::test(flavor = "multi_thread")]
async fn minted_identity_survives_rename() {
    let (brain, _raw, _graph_db) = fresh_brain("test-595-rename");
    let pool = fresh_pool().await;

    let project = projects::create_project(
        &pool,
        CreateProject {
            name: "Skunkworks Alpha".to_string(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let gid = project_graph::ensure_project_graph_identity(&pool, &brain, &project)
        .await
        .unwrap()
        .unwrap();

    projects::update_project(
        &pool,
        &project.id,
        UpdateProject {
            name: Some("Skunkworks Omega".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let renamed = projects::get_project(&pool, &project.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.name, "Skunkworks Omega");
    assert_eq!(
        renamed.graph_entity_id.as_deref(),
        Some(gid.as_str()),
        "bridge column is immutable across rename"
    );

    // Ensure after rename: the stored identity wins; no re-mint under the
    // new name.
    let gid_after = project_graph::ensure_project_graph_identity(&pool, &brain, &renamed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gid_after, gid);
    assert_eq!(
        provenance_of(&pool, &graph_entity_id_hex("project", "Skunkworks Omega")).await,
        None,
        "no second identity minted for the new name"
    );

    // The delete-side candidate set still targets the original identity.
    let candidates = project_graph::project_graph_id_candidates(&brain, &renamed).await;
    assert!(candidates.contains(&gid));
}
