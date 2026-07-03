//! Ontology → Kuzu graph reconciliation.
//!
//! On daemon startup, if ontology.toml has changed since the last sync,
//! removes entities from the Kuzu graph that no longer exist in the
//! ontology. This ensures that pruning ontology.toml actually takes
//! effect in the live graph.

use std::collections::HashSet;
use std::path::Path;

/// Reconcile the Kuzu graph with the current ontology.
///
/// Runs *before* Brain opens — opens the Kuzu DB directly, deletes stale
/// entities, and closes the connection so Brain can open normally.
///
/// Gated on a hash of ontology.toml: only runs when the ontology has
/// changed since the last successful sync.
///
/// `protected_ids` is the set of entity ids the reconciler must **never** prune —
/// those with `runtime` / `extracted` provenance (people-in-graph v1, #583),
/// loaded from `permagent.db` by
/// [`permagent::people_provenance::protected_entity_ids`]. It is the whole reason
/// runtime person-creation is durable: without it, a person created at runtime
/// (absent from the curated ontology) would be deleted here on the next ontology
/// change. An empty set reproduces the original prune-all-not-in-ontology
/// behaviour.
pub fn sync_graph_with_ontology(
    brain_dir: &Path,
    ontology_path: &Path,
    protected_ids: &HashSet<[u8; 32]>,
) {
    let graph_path = brain_dir.join("graph.kz");
    if !graph_path.exists() {
        tracing::debug!(target: "permagentd::brain_sync", "No graph.kz — skipping sync");
        return;
    }

    // Hash current ontology
    let ontology_bytes = match std::fs::read(ontology_path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(target: "permagentd::brain_sync", "Cannot read ontology: {e}");
            return;
        }
    };
    let current_hash = format!("{:x}", md5_hash(&ontology_bytes));

    // Check stored hash
    let hash_path = brain_dir.join("ontology_sync_hash");
    if let Ok(stored) = std::fs::read_to_string(&hash_path) {
        if stored.trim() == current_hash {
            tracing::debug!(target: "permagentd::brain_sync", "Ontology unchanged — skipping sync");
            return;
        }
    }

    tracing::info!(
        target: "permagentd::brain_sync",
        "Ontology changed — syncing graph with pruned ontology"
    );

    // Load ontology and compute valid entity IDs
    let ontology = match spectral::graph::ontology::Ontology::load(ontology_path) {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(target: "permagentd::brain_sync", "Failed to load ontology: {e}");
            return;
        }
    };

    let valid_ids: HashSet<[u8; 32]> = ontology
        .entities
        .iter()
        .map(|e| *ontology.entity_id_for(e).as_bytes())
        .collect();

    tracing::info!(
        target: "permagentd::brain_sync",
        "Ontology has {} entities",
        valid_ids.len()
    );

    // Back up graph.kz before modifying
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let backup_path = brain_dir.join(format!("graph.kz.bak.{timestamp}"));
    if let Err(e) = std::fs::copy(&graph_path, &backup_path) {
        tracing::error!(
            target: "permagentd::brain_sync",
            "Failed to back up graph.kz to {}: {e}",
            backup_path.display()
        );
        return;
    }
    tracing::info!(
        target: "permagentd::brain_sync",
        "Backed up graph.kz to {}",
        backup_path.display()
    );

    // Open Kuzu DB directly
    let db = match kuzu::Database::new(&graph_path, kuzu::SystemConfig::default()) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(target: "permagentd::brain_sync", "Failed to open Kuzu DB: {e}");
            return;
        }
    };
    let conn = match kuzu::Connection::new(&db) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(target: "permagentd::brain_sync", "Failed to create Kuzu connection: {e}");
            return;
        }
    };

    // Query all entities
    let all_entities = match conn.query("MATCH (e:Entity) RETURN e.id, e.canonical, e.entity_type")
    {
        Ok(result) => {
            let mut entities = Vec::new();
            for row in result {
                if let (
                    kuzu::Value::Blob(id_bytes),
                    kuzu::Value::String(canonical),
                    kuzu::Value::String(entity_type),
                ) = (&row[0], &row[1], &row[2])
                {
                    entities.push((id_bytes.clone(), canonical.clone(), entity_type.clone()));
                }
            }
            entities
        }
        Err(e) => {
            tracing::error!(target: "permagentd::brain_sync", "Failed to query entities: {e}");
            return;
        }
    };

    tracing::info!(
        target: "permagentd::brain_sync",
        "Found {} entities in Kuzu graph",
        all_entities.len()
    );

    // Find stale entities (in Kuzu but not in ontology)
    let mut removed = 0;
    let mut kept = 0;
    let mut protected = 0;
    for (id_bytes, canonical, entity_type) in &all_entities {
        let id_array: [u8; 32] = match id_bytes.as_slice().try_into() {
            Ok(a) => a,
            Err(_) => continue,
        };

        if valid_ids.contains(&id_array) {
            kept += 1;
            continue;
        }

        // Provenance guard (people-in-graph v1 #583): a runtime/extracted entity
        // is absent from the curated ontology by design — it must NOT be pruned.
        // Deterministic (explicit set membership, no heuristics) and observable
        // (logs what + why), so a runtime person is structurally safe from the
        // reconciler.
        if protected_ids.contains(&id_array) {
            protected += 1;
            tracing::info!(
                target: "permagentd::brain_sync",
                "Kept runtime/extracted entity '{}' ({}) — provenance-protected, absent from ontology",
                canonical,
                entity_type
            );
            continue;
        }

        // Delete stale entity and all its relationships
        match conn.prepare("MATCH (e:Entity) WHERE e.id = $id DETACH DELETE e") {
            Ok(mut stmt) => {
                if let Err(e) =
                    conn.execute(&mut stmt, vec![("id", kuzu::Value::Blob(id_bytes.clone()))])
                {
                    tracing::warn!(
                        target: "permagentd::brain_sync",
                        "Failed to delete entity '{}' ({}): {e}",
                        canonical,
                        entity_type
                    );
                } else {
                    tracing::info!(
                        target: "permagentd::brain_sync",
                        "Removed stale entity: '{}' ({})",
                        canonical,
                        entity_type
                    );
                    removed += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::brain_sync",
                    "Failed to prepare delete for '{}': {e}",
                    canonical
                );
            }
        }
    }

    tracing::info!(
        target: "permagentd::brain_sync",
        "Sync complete: kept {kept}, protected {protected} runtime/extracted, removed {removed} stale entities"
    );

    // Drop DB connection before writing hash (ensures Kuzu is closed)
    drop(conn);
    drop(db);

    // Store the hash so we skip on next startup
    if let Err(e) = std::fs::write(&hash_path, &current_hash) {
        tracing::warn!(
            target: "permagentd::brain_sync",
            "Failed to write sync hash: {e}"
        );
    }
}

/// Simple hash for change detection (not cryptographic).
fn md5_hash(data: &[u8]) -> u128 {
    // Use a simple FNV-like hash — we just need change detection, not security.
    // We avoid adding a dep by computing a 128-bit hash manually.
    let mut h: u128 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u128;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::core::entity_id::entity_id;
    use spectral::graph::kuzu_store::{Entity, KuzuStore};

    fn person(canonical: &str) -> Entity {
        let now = chrono::Utc::now();
        Entity {
            id: entity_id("person", canonical),
            entity_type: "person".to_string(),
            canonical: canonical.to_string(),
            visibility: spectral::Visibility::Private,
            created_at: now,
            updated_at: now,
            weight: 1.0,
            description: None,
        }
    }

    /// THE acceptance guarantee (people-in-graph v1 #583): a runtime person
    /// (provenance-protected, absent from the ontology) SURVIVES a reconcile,
    /// while a genuinely stale ontology entity is still pruned. This is the exact
    /// no-prune property that makes runtime create durable across restarts.
    #[test]
    fn runtime_person_survives_reconcile_stale_ontology_entity_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let brain_dir = dir.path();
        let graph_path = brain_dir.join("graph.kz");

        // Two person nodes in the graph, NEITHER in the ontology.
        let sabaa = person("sabaa quao"); // runtime — must survive
        let stale = person("stale person"); // curated-then-removed — must prune
        {
            let store = KuzuStore::open(&graph_path).unwrap();
            store.upsert_entity(&sabaa).unwrap();
            store.upsert_entity(&stale).unwrap();
        } // drop the store so the reconciler can open the DB

        // An ontology that contains neither (a lone unrelated curated person).
        let ontology_path = brain_dir.join("ontology.toml");
        std::fs::write(
            &ontology_path,
            "version = 1\n\n[[entity]]\ntype = \"person\"\ncanonical = \"someone else\"\naliases = []\nvisibility = \"private\"\n",
        )
        .unwrap();

        // Only Sabaa is provenance-protected.
        let mut protected: HashSet<[u8; 32]> = HashSet::new();
        protected.insert(*sabaa.id.as_bytes());

        sync_graph_with_ontology(brain_dir, &ontology_path, &protected);

        // Sabaa survives; the unprotected stale entity is gone.
        let store = KuzuStore::open(&graph_path).unwrap();
        assert!(
            store.get_entity(&sabaa.id).unwrap().is_some(),
            "runtime person MUST survive the reconciler (the #583 no-prune guarantee)"
        );
        assert!(
            store.get_entity(&stale.id).unwrap().is_none(),
            "an unprotected stale ontology entity must still be pruned (curated pruning preserved)"
        );
    }
}
