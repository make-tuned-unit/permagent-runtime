//! Project code maps use the same project wing as Reader-ingested documents.

use permagent_daemon::routes::projects::{code_map_memory_key, reindex_project_code};

const ONTOLOGY_TOML: &str = include_str!("../../goose/assets/ontology.toml");

#[tokio::test(flavor = "multi_thread")]
async fn project_reindex_writes_code_map_to_project_wing() {
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

    let root = tmp.path().join("winged-project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn fixture() -> bool { true }").unwrap();

    let project = permagent::projects::create_project(
        &pool,
        permagent::projects::CreateProject {
            name: "Winged fixture".to_string(),
            root_path: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let outcome = reindex_project_code(&pool, brain, &project)
        .await
        .expect("fixture source should produce a code map");
    let key = code_map_memory_key(&project.id);
    assert_eq!(outcome.memory_key, key);

    let memory = brain
        .get_memory_by_key(&key)
        .await
        .unwrap()
        .expect("code map memory exists");
    assert_eq!(memory.wing.as_deref(), Some(project.slug.as_str()));
}
