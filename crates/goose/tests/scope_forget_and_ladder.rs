//! Integration tests for sovereign-offboarding Phase 1 (Part of #850).
//!
//! Sanctioned raw `spectral::Brain` usage — the test owns its data dir and
//! drives the `SafeBrain` async surface. Two claims are exercised against a
//! **real** Brain (memory.db + graph.sqlite + ontology):
//!
//! - Claim 1 — "hard-delete by scope": `SafeBrain::forget_scope(wing)`
//!   hard-deletes every memory in a wing, verified, and leaves other wings
//!   (and wingless memories) untouched. The graph-triple residual (Q2,
//!   Spectral-gated) is asserted explicitly so it can never regress silently.
//! - Claim 5 — "settable scope ladder": `SafeBrain::remember_scoped` writes at
//!   a chosen `MemoryScope`, the persisted visibility matches, and Spectral's
//!   read filter behaves (a Private memory is hidden from a Team-clearance
//!   recall floor).
//!
//! These tests set `PERMAGENT_PATH_ROOT` (so `Paths::brain_dir()` — which
//! `forget_scope` enumerates — points at the temp Brain) and therefore run
//! `#[serial]`.

use std::sync::Arc;

use permagent::brain_handle::{MemoryScope, SafeBrain};
use permagent::config::paths::Paths;
use serial_test::serial;
use spectral::core::entity_id::entity_id;
use spectral::graph::graph_store::{Entity, Triple};
use spectral::{Brain, DeviceId, RememberOpts, Visibility};

fn ontology_toml() -> &'static str {
    include_str!("../assets/ontology.toml")
}

/// Build a real Brain at `Paths::brain_dir()`. MUST be called inside
/// `spawn_blocking` — `Brain::builder().build()` spins its own runtime and
/// panics if constructed on an async worker.
fn build_brain() -> Arc<Brain> {
    let brain_dir = Paths::brain_dir();
    let ontology_path = brain_dir.join("ontology.toml");
    std::fs::create_dir_all(&brain_dir).expect("create brain dir");
    std::fs::write(&ontology_path, ontology_toml()).expect("write ontology");

    Arc::new(
        Brain::builder()
            .data_dir(&brain_dir)
            .ontology_path(&ontology_path)
            .device_id(DeviceId::from_descriptor("offboarding-phase1-test"))
            .build()
            .expect("brain open"),
    )
}

fn remember(brain: &Brain, key: &str, content: &str, wing: Option<&str>) {
    brain
        .remember_with(
            key,
            content,
            RememberOpts {
                source: Some("reader".into()),
                device_id: Some(*brain.device_id()),
                confidence: Some(1.0),
                visibility: Visibility::Private,
                wing: wing.map(|w| w.to_string()),
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("remember_with {key}: {e}"));
}

/// Seed a company-derived graph triple directly through the public graph store
/// (deterministic; independent of ontology/entity-policy). Returns the triple
/// count for a pre-sweep sanity assert.
fn seed_triple(brain: &Brain) -> usize {
    let now = spectral::Utc::now();
    let org = entity_id("organization", "acme corp");
    let concept = entity_id("concept", "acme secret roadmap");
    let store = brain.store();
    for (id, ty, canonical) in [
        (org, "organization", "acme corp"),
        (concept, "concept", "acme secret roadmap"),
    ] {
        store
            .upsert_entity(&Entity {
                id,
                entity_type: ty.into(),
                canonical: canonical.into(),
                visibility: Visibility::Private,
                created_at: now,
                updated_at: now,
                weight: 1.0,
                description: None,
            })
            .expect("upsert entity");
    }
    store
        .insert_triple(&Triple {
            from: org,
            to: concept,
            predicate: "related_to".into(),
            confidence: 0.9,
            source_doc_id: None,
            source_brain_id: *brain.brain_id(),
            asserted_at: now,
            visibility: Visibility::Private,
            weight: 1.0,
        })
        .expect("insert triple");
    store.find_triples(None, None, None).expect("find").len()
}

#[tokio::test]
#[serial]
async fn forget_scope_hard_deletes_wing_and_spares_others() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("PERMAGENT_PATH_ROOT", temp.path());

    // Build + seed on a blocking thread (Brain owns a runtime).
    let brain = tokio::task::spawn_blocking(|| {
        let brain = build_brain();
        // Two "company" memories in the acme wing.
        remember(
            &brain,
            "acme-1",
            "Acme internal architecture: the billing service shards by tenant id.",
            Some("acme"),
        );
        remember(
            &brain,
            "acme-2",
            "Acme roadmap Q3: migrate the ledger to the new settlement pipeline.",
            Some("acme"),
        );
        // A personal memory in a different wing.
        remember(
            &brain,
            "personal-1",
            "I prefer to review PRs in the morning before standup.",
            Some("personal"),
        );
        // A wingless memory (chat turns write wing = NULL).
        remember(
            &brain,
            "chat-1",
            "User: hello. Assistant: hi there, how can I help?",
            None,
        );
        let triple_count = seed_triple(&brain);
        assert!(triple_count >= 1, "expected a seeded graph triple");
        brain
    })
    .await
    .expect("build task");

    let safe = SafeBrain::from_arc(brain.clone());

    // Sweep the acme wing.
    let report = safe.forget_scope("acme").await.expect("forget_scope");
    assert_eq!(report.wing.as_deref(), Some("acme"));
    assert_eq!(report.keys_swept, 2, "two acme memories enumerated");
    assert_eq!(report.existed, 2, "both existed");
    assert_eq!(report.fully_forgotten, 2, "both verified gone");
    assert_eq!(
        report.forgotten_keys.len(),
        2,
        "audit receipt lists both keys"
    );
    assert!(
        report.residual_keys.is_empty(),
        "scope receipt must be clean"
    );
    // Q2 residual: graph triples are Spectral-gated and NOT deleted. This
    // assertion documents the gap and guards against a silent regression.
    assert_eq!(
        report.graph_triples_deleted, 0,
        "graph-triple deletion is Spectral-gated at pin fb1038db (Q2)"
    );

    // Acme memories are gone.
    assert!(
        safe.get_memory_by_key("acme-1")
            .await
            .expect("get acme-1")
            .is_none(),
        "acme-1 hard-deleted"
    );
    assert!(
        safe.get_memory_by_key("acme-2")
            .await
            .expect("get acme-2")
            .is_none(),
        "acme-2 hard-deleted"
    );

    // Other-wing and wingless memories survive untouched.
    assert!(
        safe.get_memory_by_key("personal-1")
            .await
            .expect("get personal-1")
            .is_some(),
        "personal wing untouched"
    );
    assert!(
        safe.get_memory_by_key("chat-1")
            .await
            .expect("get chat-1")
            .is_some(),
        "wingless memory untouched"
    );

    // The company-derived graph triple SURVIVES the memory sweep (Q2 residual).
    let triples_after = tokio::task::spawn_blocking({
        let b = brain.clone();
        move || {
            b.store()
                .find_triples(None, None, None)
                .expect("find")
                .len()
        }
    })
    .await
    .expect("triple-count task");
    assert!(
        triples_after >= 1,
        "graph triples survive forget (documented Spectral-gated Q2 residual)"
    );

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}

#[tokio::test]
#[serial]
async fn forget_scope_on_empty_wing_is_a_noop() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("PERMAGENT_PATH_ROOT", temp.path());

    let brain = tokio::task::spawn_blocking(|| {
        let brain = build_brain();
        remember(&brain, "keep-1", "A memory in the kept wing.", Some("kept"));
        brain
    })
    .await
    .expect("build task");

    let safe = SafeBrain::from_arc(brain);
    let report = safe
        .forget_scope("nonexistent-wing")
        .await
        .expect("forget_scope");
    assert_eq!(report.keys_swept, 0, "no members to sweep");
    assert_eq!(report.fully_forgotten, 0);
    assert!(report.residual_keys.is_empty());
    assert!(
        safe.get_memory_by_key("keep-1")
            .await
            .expect("get keep-1")
            .is_some(),
        "unrelated wing untouched by empty sweep"
    );

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}

#[tokio::test]
#[serial]
async fn forget_scope_reenumerates_keys_that_appear_during_the_sweep() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("PERMAGENT_PATH_ROOT", temp.path());

    let brain = tokio::task::spawn_blocking(|| {
        let brain = build_brain();
        remember(
            &brain,
            "first",
            "Initially in the target wing.",
            Some("acme"),
        );
        remember(
            &brain,
            "late",
            "Moves into the target wing after enumeration.",
            Some("pending"),
        );
        let db_path = Paths::brain_dir().join("memory.db");
        let conn = rusqlite::Connection::open(db_path).expect("trigger connection");
        // Deterministically model a concurrent committed write after the first
        // key snapshot: deleting `first` makes `late` enter the swept scope.
        conn.execute_batch(
            "CREATE TRIGGER add_late_scope_member AFTER DELETE ON memories
             WHEN OLD.key = 'first'
             BEGIN UPDATE memories SET wing = 'acme' WHERE key = 'late'; END;",
        )
        .expect("create trigger");
        brain
    })
    .await
    .expect("build task");

    let safe = SafeBrain::from_arc(brain);
    let report = safe.forget_scope("acme").await.expect("forget scope");
    assert_eq!(report.keys_swept, 2, "the second pass must sweep late");
    assert!(report.forgotten_keys.contains(&"late".to_string()));
    assert!(report.residual_keys.is_empty());
    assert!(safe
        .get_memory_by_key("late")
        .await
        .expect("get late")
        .is_none());

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}

#[tokio::test]
#[serial]
async fn visibility_ladder_is_settable_and_filtered() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("PERMAGENT_PATH_ROOT", temp.path());

    let brain = tokio::task::spawn_blocking(build_brain)
        .await
        .expect("build task");
    let safe = SafeBrain::from_arc(brain);

    // Write one memory at each ladder level via the settable path.
    for (key, scope, expected) in [
        ("vis-private", MemoryScope::Private, "private"),
        ("vis-team", MemoryScope::Team, "team"),
        ("vis-org", MemoryScope::Org, "org"),
        ("vis-public", MemoryScope::Public, "public"),
    ] {
        safe.remember_scoped(
            key,
            &format!("Distinctive ladder content for the {expected} level."),
            scope,
            RememberOpts {
                source: Some("test".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("remember_scoped {key}: {e}"));

        // Deterministic proof the chosen level was persisted.
        let mem = safe
            .get_memory_by_key(key)
            .await
            .unwrap_or_else(|e| panic!("get {key}: {e}"))
            .unwrap_or_else(|| panic!("{key} should exist"));
        assert_eq!(
            mem.visibility, expected,
            "{key} persisted at the chosen visibility"
        );
    }

    // Mapping round-trips losslessly.
    for scope in [
        MemoryScope::Private,
        MemoryScope::Team,
        MemoryScope::Org,
        MemoryScope::Public,
    ] {
        assert_eq!(MemoryScope::from_visibility(scope.to_visibility()), scope);
    }

    // Real read filter: recall clearance is a publicness floor
    // (`content_vis >= clearance`). A Private memory surfaces at Private
    // clearance but is filtered out at Team clearance.
    let q = "Distinctive ladder content for the private level.";
    let at_private: Vec<String> = safe
        .recall(q, Visibility::Private)
        .await
        .expect("recall private")
        .memory_hits
        .iter()
        .map(|h| h.key.clone())
        .collect();
    assert!(
        at_private.iter().any(|k| k == "vis-private"),
        "Private memory is visible at Private clearance (floor); got {at_private:?}"
    );

    let at_team: Vec<String> = safe
        .recall(q, Visibility::Team)
        .await
        .expect("recall team")
        .memory_hits
        .iter()
        .map(|h| h.key.clone())
        .collect();
    assert!(
        !at_team.iter().any(|k| k == "vis-private"),
        "Private memory is filtered out at Team clearance; got {at_team:?}"
    );

    std::env::remove_var("PERMAGENT_PATH_ROOT");
}
