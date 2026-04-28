//! Phase 1 smoke test for Spectral integration.
//!
//! Proves Spectral's Brain API compiles cleanly inside Permagent's Cargo
//! workspace and round-trips memories with full provenance fields.
//!
//! Phase 1 exercises the API surface that works against a schema-only
//! ontology: remember_with, recall, brain reopen. Graph layer assertions
//! (brain.assert) require runtime entity creation and are gated on
//! Spectral's AutoCreateWithCanonicalizer (Phase 2 Track A). Once that
//! lands, this file gains a graph round-trip test.
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

    // Write a chat-turn memory with full provenance fields.
    // Permagent's chat layer will call this on every turn in Phase 4.
    brain
        .remember_with(
            "chat-turn-smoke-1",
            "User asked about Spectral integration. Agent confirmed v1.0 ships chat memory with the graph layer foundation in place.",
            RememberOpts {
                source: Some("chat".into()),
                device_id: Some(brain.device_id().clone()),
                confidence: Some(1.0),
                visibility: Visibility::Private,
            },
        )
        .expect("remember_with");

    // Recall via fingerprint matching. Wings/halls inferred from content.
    let recall = brain
        .recall("Spectral integration v1.0 chat memory", Visibility::Private)
        .expect("recall");

    assert!(
        !recall.memory_hits.is_empty(),
        "expected at least one memory hit from fingerprint recall"
    );

    let memory = &recall.memory_hits[0];
    assert_eq!(memory.source.as_deref(), Some("chat"));
    assert!(memory.confidence > 0.9);

    drop(brain);

    // Reopen verifies persistence + automatic schema migration on existing brain
    let brain_reopened = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-smoke-test"))
        .build()
        .expect("reopen brain");

    let recall_after_reopen = brain_reopened
        .recall("Spectral integration v1.0 chat memory", Visibility::Private)
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
    std::fs::write(&ontology_path, ontology_toml()).expect("write ontology");

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

#[test]
#[ignore] // Run explicitly with: cargo test -p permagent --test spectral_smoke -- --ignored --nocapture
fn opens_migrated_brain_and_recalls() {
    let brain_path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".permagent/brain"))
        .expect("HOME");

    if !brain_path.exists() {
        eprintln!("skipping: {} does not exist", brain_path.display());
        return;
    }

    let ontology_path = brain_path.join("ontology.toml");

    let brain = Brain::builder()
        .data_dir(&brain_path)
        .ontology_path(&ontology_path)
        .device_id(DeviceId::from_descriptor("permagent-live-brain-test"))
        .build()
        .expect("open migrated brain");

    let recall = brain
        .recall("permagent runtime", Visibility::Private)
        .expect("recall");

    eprintln!("Recall hits: {}", recall.memory_hits.len());
    for (i, hit) in recall.memory_hits.iter().take(3).enumerate() {
        let preview: String = hit
            .content
            .chars()
            .take(80)
            .collect::<String>()
            .replace('\n', " ");
        eprintln!("  {}. score={:.2} {}", i + 1, hit.signal_score, preview);
    }

    assert!(
        !recall.memory_hits.is_empty(),
        "migrated brain returned zero recall hits — schema or version mismatch?"
    );
}
