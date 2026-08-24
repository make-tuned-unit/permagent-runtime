//! N2 audit blocker: project reindex must not duplicate or cross-delete Brain
//! memories.
//!
//! Two projects can each hold a `README.md`. The Reader keys project text as
//! `doc:{project_id}:{relative/path}` (`reader::doc_memory_key`), so:
//!
//!   * the SAME filename in two projects is two distinct memories — reindexing
//!     one must never retire the other's;
//!   * the SAME path reindexed after an edit is still ONE memory — the stable
//!     key must be replaced in place, not accumulate a second copy. (Spectral's
//!     stable-key write returns `WriteOutcome::NoOp` forever, so `ingest_bytes_as`
//!     has to forget-then-write; without that the edit is silently dropped.)
//!
//! Lives in its own integration binary rather than the lib-test module because
//! it needs a REAL Brain: `AppState` mounts one only when brain_dir AND
//! ontology.toml both exist, and the shared lib-test root deliberately writes
//! no ontology (~700 tests there are written against `state.brain == None`).
//! One test per binary — `PERMAGENT_PATH_ROOT` is process-wide (the
//! `crm_graph_wiring` precedent).

use permagent::reader;
use permagent_daemon::routes::projects::reindex_project_code;

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

#[tokio::test(flavor = "multi_thread")]
async fn project_reindex_scopes_same_named_files_and_replaces_edits() {
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
    let pool = state.session_manager().pool_clone().await.unwrap();
    let brain = state
        .brain
        .as_ref()
        .expect("AppState must mount the Brain when brain_dir + ontology exist");

    let root_a = tmp.path().join("project-reader-a");
    let root_b = tmp.path().join("project-reader-b");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    std::fs::write(root_a.join("README.md"), "Project A original knowledge").unwrap();
    std::fs::write(root_b.join("README.md"), "Project B independent knowledge").unwrap();

    let project_a = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Reader scope A".to_string(),
            root_path: Some(root_a.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let project_b = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Reader scope B".to_string(),
            root_path: Some(root_b.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let first_a = reindex_project_code(&pool, brain, &project_a)
        .await
        .unwrap();
    let first_b = reindex_project_code(&pool, brain, &project_b)
        .await
        .unwrap();
    let key_a = reader::doc_memory_key(&project_a.id, "README.md");
    let key_b = reader::doc_memory_key(&project_b.id, "README.md");
    assert_eq!(first_a.memory_key, key_a);
    assert_eq!(first_b.memory_key, key_b);
    assert_ne!(key_a, key_b, "same filename, different projects, two keys");
    assert_eq!(
        brain
            .get_memory_by_key(&key_a)
            .await
            .unwrap()
            .unwrap()
            .content,
        "Project A original knowledge"
    );
    assert_eq!(
        brain
            .get_memory_by_key(&key_b)
            .await
            .unwrap()
            .unwrap()
            .content,
        "Project B independent knowledge"
    );

    std::fs::write(root_a.join("README.md"), "Project A edited replacement").unwrap();
    let edited = reindex_project_code(&pool, brain, &project_a)
        .await
        .unwrap();
    assert_eq!(edited.memory_key, key_a, "path identity remains stable");
    assert_eq!(
        brain
            .get_memory_by_key(&key_a)
            .await
            .unwrap()
            .unwrap()
            .content,
        "Project A edited replacement",
        "edited content replaces the prior stable-key memory (no duplicate)"
    );
    assert_eq!(
        brain
            .get_memory_by_key(&key_b)
            .await
            .unwrap()
            .unwrap()
            .content,
        "Project B independent knowledge",
        "editing project A must not retire project B's same-named file"
    );
}
