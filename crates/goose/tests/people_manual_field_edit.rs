//! CRM slice 2b (#495) — the manual field-edit write path.
//!
//! Exercises the exact seam the `PATCH /api/people/{id}/fields` route uses:
//! `SafeBrain::set_entity_field(.., FieldSource::Manual, ..)` to write, and
//! `SafeBrain::entity_fields_for` to read back through the people↔graph bridge,
//! keyed by `graph_entity_id_hex("person", name)` — the same id derivation the
//! route takes from `people.graph_entity_id`.
//!
//! Proves the guarantees the route relies on:
//!   1. A manual write persists and round-trips with `Manual` provenance.
//!   2. The #495 vocabulary additions (`birthday`, `relationship_strength`,
//!      `how_met`) are accepted, round-trip, and surface on `Person` via the
//!      same `set_attribute` overlay the route applies.
//!   3. A later `Enriched` write for a manually-set field is SUPPRESSED — the
//!      "enrichment never clobbers manual" guarantee, now non-vacuous because
//!      this path is what finally writes `Manual`.
//!
//! Sanctioned raw `spectral::Brain` construction — the test owns its runtime,
//! mirroring `people_attr_latency.rs`.

use permagent::brain_handle::SafeBrain;
use permagent::identity::canonical::graph_entity_id_hex;
use permagent::people::{Person, PERSON_FIELD_NAMES};
use spectral::core::entity_id::EntityId;
use spectral::ingest::FieldSource;
use spectral::{Brain, DeviceId};
use tempfile::{tempdir, TempDir};

fn ontology_toml() -> &'static str {
    include_str!("../assets/ontology.toml")
}

/// Build a `SafeBrain` off the async executor, exactly as `state.rs` does. The
/// returned `TempDir` must be held for the test's lifetime — the brain's data
/// dir lives under it.
async fn test_brain() -> (SafeBrain, TempDir) {
    let temp = tempdir().expect("tempdir");
    let ontology_path = temp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ontology_toml()).expect("write ontology");
    let data_dir = temp.path().join("brain");
    let brain = tokio::task::spawn_blocking(move || {
        Brain::builder()
            .data_dir(data_dir)
            .ontology_path(&ontology_path)
            .device_id(DeviceId::from_descriptor("permagent-manual-edit-test"))
            .build()
            .expect("brain open")
    })
    .await
    .expect("brain build task");
    (SafeBrain::new(brain), temp)
}

/// The route's id derivation: `people.graph_entity_id` == hex(blake3 of the
/// graph-canonical person name), parsed to an `EntityId`.
fn person_id(name: &str) -> EntityId {
    graph_entity_id_hex("person", name)
        .parse::<EntityId>()
        .expect("valid 64-hex EntityId")
}

/// Apply the graph overlay to a fresh `Person` exactly as the route does:
/// clear column-sourced attributes, then set each `entity_fields` value by name.
async fn overlaid_person(brain: &SafeBrain, name: &str) -> Person {
    let id = person_id(name);
    let hex = id.to_string();
    let map = brain
        .entity_fields_for(vec![id])
        .await
        .expect("entity_fields_for");

    let mut p = Person {
        entity_uuid: "uuid".into(),
        canonical_id: format!("person:{}", name.to_lowercase().replace(' ', "-")),
        display_name: name.into(),
        role: None,
        company: None,
        email: None,
        phone: None,
        notes: None,
        last_contact_at: None,
        birthday: None,
        relationship_strength: None,
        how_met: None,
        linkedin: None,
        x_handle: None,
        personal_site: None,
        graph_entity_id: Some(hex.clone()),
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    p.clear_attributes();
    if let Some(fields) = map.get(&hex) {
        for f in fields {
            p.set_attribute(&f.field_name, f.value.clone());
        }
    }
    p
}

#[tokio::test]
async fn manual_write_persists_and_round_trips_including_new_vocab() {
    let (brain, _temp) = test_brain().await;
    let name = "Jane Doe";
    let id = person_id(name);

    // Sanity: the #495 vocabulary additions are in the shared allowlist the
    // route validates against — "the new field names are accepted".
    for field in ["birthday", "relationship_strength", "how_met"] {
        assert!(
            PERSON_FIELD_NAMES.contains(&field),
            "PERSON_FIELD_NAMES must include {field}"
        );
    }

    // Manual write across an existing field and all three new fields.
    let writes = [
        ("email", "jane@example.com"),
        ("birthday", "1990-04-01"),
        ("relationship_strength", "close"),
        ("how_met", "conference 2019"),
    ];
    for (field, value) in writes {
        let applied = brain
            .set_entity_field(id, field, value, FieldSource::Manual, None)
            .await
            .expect("manual write ok");
        assert!(applied, "first manual write of {field} must apply");
    }

    // Round-trip through the exact route read hop, with provenance intact.
    let map = brain.entity_fields_for(vec![id]).await.expect("read");
    let fields = map.get(&id.to_string()).expect("fields for person");
    assert_eq!(fields.len(), 4, "all four manual fields persisted");
    for f in fields {
        assert_eq!(f.source, FieldSource::Manual, "{} is Manual", f.field_name);
    }

    // And they surface on `Person` through the same overlay the route applies.
    let p = overlaid_person(&brain, name).await;
    assert_eq!(p.email.as_deref(), Some("jane@example.com"));
    assert_eq!(p.birthday.as_deref(), Some("1990-04-01"));
    assert_eq!(p.relationship_strength.as_deref(), Some("close"));
    assert_eq!(p.how_met.as_deref(), Some("conference 2019"));
}

#[tokio::test]
async fn enrichment_never_clobbers_a_manual_value() {
    let (brain, _temp) = test_brain().await;
    let name = "John Roe";
    let id = person_id(name);

    // User manually sets company.
    assert!(brain
        .set_entity_field(id, "company", "Acme", FieldSource::Manual, None)
        .await
        .expect("manual write"));

    // A later enrichment pass tries to overwrite the same field — must be
    // suppressed by the store and reported as not-applied (`false`).
    let applied = brain
        .set_entity_field(
            id,
            "company",
            "Globex (guessed)",
            FieldSource::Enriched,
            Some("https://example.com/guess"),
        )
        .await
        .expect("suppressed enriched write still returns Ok");
    assert!(
        !applied,
        "enriched write over a manual field must be suppressed"
    );

    // The manual value survives, still with Manual provenance, through the
    // overlay the route uses for its response.
    let p = overlaid_person(&brain, name).await;
    assert_eq!(
        p.company.as_deref(),
        Some("Acme"),
        "manual company value preserved against enrichment"
    );
    let map = brain.entity_fields_for(vec![id]).await.expect("read");
    let company = map
        .get(&id.to_string())
        .and_then(|fs| fs.iter().find(|f| f.field_name == "company"))
        .expect("company field");
    assert_eq!(company.source, FieldSource::Manual);
}
