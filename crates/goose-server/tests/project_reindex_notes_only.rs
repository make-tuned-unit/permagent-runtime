//! A notes-only project is meaningfully indexable.
//!
//! `reindex_project_code` used to return `NoSourceFiles` whenever tree-sitter
//! found no parseable language, which threw away the markdown that IS the
//! knowledge in a research or planning project. Text ingest now runs BEFORE
//! that decision, and its memories are project-scoped and associated, so the
//! Brain can recall them and the project can list them.
//!
//! Own integration binary: needs a REAL Brain (see the sibling
//! `project_reindex_scoping.rs` header for why the lib-test binary cannot
//! provide one) and `PERMAGENT_PATH_ROOT` is process-wide.

use permagent::reader;
use permagent_daemon::routes::projects::reindex_project_code;

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

#[tokio::test(flavor = "multi_thread")]
async fn note_only_project_reindex_succeeds_before_no_source_files() {
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

    let root = tmp.path().join("project-notes-only");
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(
        root.join("notes").join("plan.md"),
        "A durable planning note with no source code in this project.",
    )
    .unwrap();

    let project = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Notes only".to_string(),
            root_path: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let outcome = reindex_project_code(&pool, brain, &project)
        .await
        .expect("a text-only project is meaningfully indexable");
    let key = reader::doc_memory_key(&project.id, "notes/plan.md");
    assert_eq!(outcome.files, 1);
    assert_eq!(outcome.memory_key, key);
    assert_eq!(
        brain
            .get_memory_by_key(&key)
            .await
            .unwrap()
            .unwrap()
            .content,
        "A durable planning note with no source code in this project."
    );

    let associations =
        permagent::project_association::list_project_memory_associations(&pool, &project.id)
            .await
            .unwrap();
    let memory_id = brain.get_memory_by_key(&key).await.unwrap().unwrap().id;
    assert!(
        associations.iter().any(|a| a.memory_id == memory_id),
        "the ingested note must be associated with its project"
    );
}
