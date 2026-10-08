//! Dedicated process: verify product bootstrap, pinned ontology parser and actual
//! AppState mount together. No fixture ontology, personal data or worker dispatch.
use permagent_daemon::brain_bootstrap::{
    initialize_fresh_brain, BootstrapOutcome, BASELINE_ONTOLOGY,
};

#[tokio::test(flavor = "multi_thread")]
async fn fresh_product_ontology_mounts_a_brain_without_seeded_people_or_projects() {
    let root = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", root.path());
    std::env::set_var("PERMAGENT_DISABLE_KEYRING", "1");
    let ontology = spectral::graph::ontology::Ontology::from_toml(BASELINE_ONTOLOGY)
        .expect("product baseline must parse with the pinned Spectral version");
    assert_eq!(ontology.version, 1);
    assert!(ontology.entities.is_empty());
    assert!(ontology.predicates.is_empty());
    assert_eq!(
        initialize_fresh_brain(root.path()).unwrap(),
        BootstrapOutcome::Created
    );
    let bytes = std::fs::read(root.path().join("brain/ontology.toml")).unwrap();
    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let brain = state
        .brain
        .as_ref()
        .expect("fresh production bootstrap must mount a real Brain");
    assert!(brain
        .get_memory_by_key("fresh-install/absent")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        initialize_fresh_brain(root.path()).unwrap(),
        BootstrapOutcome::ExistingOntology
    );
    assert_eq!(
        std::fs::read(root.path().join("brain/ontology.toml")).unwrap(),
        bytes
    );
}
