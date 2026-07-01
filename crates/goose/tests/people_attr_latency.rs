//! Decision E (measure-before-cache, #255) — read-through latency for the
//! people↔graph attribute overlay.
//!
//! Times `SafeBrain::entity_fields_for` (the exact batched hop the people/project
//! routes use) for a realistic N-person panel, in two states:
//!   - empty:     no entity_fields written yet (the Step-2 reality until 2b)
//!   - populated: each person has the 6 person attributes written (post-2b)
//!
//! On-demand bench (ignored in normal runs). Run with:
//!   cargo test -p permagent --test people_attr_latency -- --ignored --nocapture
//!
//! Sanctioned raw `spectral::Brain` usage — the test owns its runtime.

use std::time::Instant;

use permagent::brain_handle::SafeBrain;
use permagent::identity::canonical::graph_entity_id_hex;
use permagent::people::PERSON_FIELD_NAMES;
use spectral::ingest::FieldSource;
use spectral::{Brain, DeviceId};
use tempfile::tempdir;

fn ontology_toml() -> &'static str {
    include_str!("../assets/ontology.toml")
}

#[tokio::test]
#[ignore = "on-demand latency bench; run with --ignored --nocapture"]
async fn read_through_latency_for_realistic_panel() {
    const N: usize = 100; // realistic CRM directory / project-people panel size

    let temp = tempdir().expect("tempdir");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ontology_toml()).expect("write ontology");
    // Brain::builder().build() spins its own runtime — build off the async
    // executor via spawn_blocking, exactly as state.rs does.
    let data_dir = temp.path().join("brain");
    let brain = tokio::task::spawn_blocking(move || {
        Brain::builder()
            .data_dir(data_dir)
            .ontology_path(&ontology_path)
            .device_id(DeviceId::from_descriptor("permagent-attr-latency"))
            .build()
            .expect("brain open")
    })
    .await
    .expect("brain build task");
    let brain = SafeBrain::new(brain);

    // N person ids, exactly as the route derives them from people.graph_entity_id.
    let names: Vec<String> = (0..N).map(|i| format!("Person Number {i}")).collect();
    let ids: Vec<_> = names
        .iter()
        .map(|n| {
            graph_entity_id_hex("person", n)
                .parse::<spectral::core::entity_id::EntityId>()
                .unwrap()
        })
        .collect();

    // ── State 1: empty (no fields written) — the Step-2 reality until 2b ──
    let started = Instant::now();
    let map = brain.entity_fields_for(ids.clone()).await.unwrap();
    let empty_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(map.is_empty());

    // ── Populate all 6 attributes per person (post-2b state) ──
    for (id, _name) in ids.iter().zip(names.iter()) {
        for field in PERSON_FIELD_NAMES {
            brain
                .set_entity_field(*id, field, "value", FieldSource::Manual, None)
                .await
                .unwrap();
        }
    }

    // ── State 2: populated read-through ──
    let started = Instant::now();
    let map = brain.entity_fields_for(ids.clone()).await.unwrap();
    let full_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(map.len(), N);

    eprintln!("───────── Decision E read-through latency (N={N}) ─────────");
    eprintln!(
        "  empty     (0 fields):   {empty_ms:.2} ms  ({:.3} ms/person)",
        empty_ms / N as f64
    );
    eprintln!(
        "  populated (6 fields/p): {full_ms:.2} ms  ({:.3} ms/person)",
        full_ms / N as f64
    );
    eprintln!("──────────────────────────────────────────────────────────");

    // The Brain holds an internal runtime; drop it off the async executor (same
    // reason it is built there) so teardown doesn't panic.
    tokio::task::spawn_blocking(move || drop(brain))
        .await
        .expect("brain drop task");
}
