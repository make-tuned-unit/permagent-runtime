//! Phase 1 smoke test for Spectral integration.
//!
//! Proves that Spectral's Brain API compiles cleanly inside Permagent's
//! Cargo workspace and round-trips memories + triples correctly through
//! the full ontology validation path.
//!
//! This test does NOT wire Spectral into Permagent's production code
//! paths. That's Phase 4 of the integration plan. See
//! docs/architecture/SPECTRAL_INTEGRATION.md for the full migration plan.

use spectral::{Brain, DeviceId, RememberOpts, Visibility};
use tempfile::tempdir;

fn ontology_toml() -> &'static str {
    include_str!("../assets/ontology.toml")
}

#[test]
fn spectral_round_trips_chat_memory_with_provenance() {
    let temp = tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ontology_toml()).expect("write ontology");

    let brain = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-smoke-test"))
        .build()
        .expect("brain open");

    // Write a memory with full provenance fields
    brain
        .remember_with(
            "smoke-test-memory-1",
            "Phase 1 integration: Spectral compiles and writes inside Permagent.",
            RememberOpts {
                source: Some("permagent_smoke_test".into()),
                device_id: Some(brain.device_id().clone()),
                confidence: Some(0.95),
                visibility: Visibility::Private,
            },
        )
        .expect("remember_with");

    // Write a graph triple using a v1.0 ontology predicate.
    // "Jesse" resolves to canonical "jesse-sharratt" (person entity),
    // "Permagent" resolves to canonical "permagent" (project entity).
    brain
        .assert(
            "Jesse",
            "worked_on",
            "Permagent",
            0.95,
            Visibility::Private,
        )
        .expect("assert triple");

    // Recall using a query that matches the stored content's fingerprints.
    // TACT retrieval matches on shared terms, so use words from the content.
    let recall = brain
        .recall("Spectral compiles inside Permagent", Visibility::Private)
        .expect("recall");

    assert!(
        !recall.memory_hits.is_empty(),
        "expected at least one memory hit from TACT recall"
    );

    let memory = &recall.memory_hits[0];
    assert_eq!(memory.source.as_deref(), Some("permagent_smoke_test"));
    assert!(memory.confidence > 0.9);

    // Verify the graph triple via the hybrid recall's graph component
    let triples = &recall.graph.triples;
    assert!(
        triples.iter().any(|t| t.predicate == "worked_on"),
        "expected worked_on triple, got {:?}",
        triples
    );

    drop(brain);

    // Reopen verifies schema persistence + automatic migration on existing brain
    let brain_reopened = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-smoke-test"))
        .build()
        .expect("reopen brain");

    let recall_after_reopen = brain_reopened
        .recall("Spectral compiles inside Permagent", Visibility::Private)
        .expect("recall after reopen");

    assert!(
        !recall_after_reopen.memory_hits.is_empty(),
        "memory should persist across brain reopen"
    );
}

#[test]
fn ontology_loads_without_specific_entities() {
    // Verify the schema-only ontology design works.
    // Ontology declares entity types + predicates with no specific entities.
    // Brain should open cleanly even though no [[entity]] entries exist.
    let temp = tempdir().expect("tempdir");
    let brain_path = temp.path().join("brain");
    let ontology_path = temp.path().join("ontology.toml");

    // Write a schema-only ontology: version header only, no entities or predicates
    std::fs::write(&ontology_path, "version = 1\n").expect("write ontology");

    let brain = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-smoke-test"))
        .build()
        .expect("schema-only ontology should load cleanly");

    // Smoke test: brain accepts memory writes even with no entities declared
    brain
        .remember_with(
            "schema-test-1",
            "Schema-only ontology test memory.",
            RememberOpts {
                source: Some("test".into()),
                device_id: Some(brain.device_id().clone()),
                confidence: Some(1.0),
                visibility: Visibility::Private,
            },
        )
        .expect("memory write should succeed without entities");
}
