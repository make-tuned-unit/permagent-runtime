//! Entity provenance — where a graph entity came from (people-in-graph v1, #583).
//!
//! People are graph-authoritative, but the reconciler
//! ([`crate`]'s `sync_graph_with_ontology`, daemon-side) is **prune-only**: it
//! deletes every Kuzu entity absent from the curated ontology. That would delete
//! any person created at runtime. Provenance is what lets the reconciler and
//! runtime writes coexist without conflict: the reconciler prunes ONLY
//! `ontology`-sourced entities (what the curator owns); `runtime`/`extracted`
//! entities are **structurally excluded** from the prune predicate and are never
//! deleted.
//!
//! ## Where it lives (Decision A/D, #583)
//!
//! A `permagent.db` side table keyed on the bare 64-hex `EntityId` — the same key
//! `people.graph_entity_id` and `entity_fields_for` already use. The pinned
//! Spectral dep stays frozen. The alternative — a `source` column on the Kuzu
//! `Entity` node — is the **deferred v2 migration**, worthwhile only if provenance
//! must survive independently of `permagent.db` (e.g. graph-only cross-device
//! sync). It is intentionally not built now.

use std::collections::HashSet;

use sqlx::{Pool, Sqlite};

/// Where a graph entity originated. Explicit three-valued provenance (Decision B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// From the curated `ontology.toml`. The reconciler may prune it when the
    /// curator removes it from the file.
    Ontology,
    /// Created by an explicit human/agent action (Henry `create_person`, UI
    /// add-person). Never pruned.
    Runtime,
    /// Created by automated extraction (v1.5 Librarian "person mentioned"). Never
    /// pruned.
    Extracted,
}

impl Provenance {
    /// The stored string form (matches the table's `CHECK` constraint).
    pub fn as_str(&self) -> &'static str {
        match self {
            Provenance::Ontology => "ontology",
            Provenance::Runtime => "runtime",
            Provenance::Extracted => "extracted",
        }
    }

    /// True for provenances the reconciler must **never** prune.
    pub fn is_protected(&self) -> bool {
        matches!(self, Provenance::Runtime | Provenance::Extracted)
    }
}

/// Record an entity's provenance, keyed on its bare 64-hex `EntityId`.
///
/// Idempotent and **first-source-wins** (`INSERT OR IGNORE`): a person created at
/// runtime keeps its `runtime` origin even if the curator later adds the same
/// name to `ontology.toml` (Decision G — no-op promotion; an in-ontology entity is
/// never pruned regardless of its recorded provenance).
pub async fn record_provenance(
    pool: &Pool<Sqlite>,
    entity_id_hex: &str,
    source: Provenance,
) -> Result<(), String> {
    sqlx::query("INSERT OR IGNORE INTO entity_provenance (entity_id_hex, source) VALUES (?, ?)")
        .bind(entity_id_hex)
        .bind(source.as_str())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The set of entity ids the reconciler must never prune — those with `runtime`
/// or `extracted` provenance — as raw 32-byte ids (the Kuzu node key), so
/// `sync_graph_with_ontology` can compare directly against its ontology id set.
///
/// Deliberately **tolerant**: returns an empty set on any error (e.g. the table
/// not yet created on an older DB) so it can never break daemon startup. An empty
/// protected set degrades to today's prune-all-not-in-ontology behaviour — safe,
/// never a spurious extra prune.
pub async fn protected_entity_ids(pool: &Pool<Sqlite>) -> HashSet<[u8; 32]> {
    let mut set = HashSet::new();
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT entity_id_hex FROM entity_provenance WHERE source IN ('runtime','extracted')",
    )
    .fetch_all(pool)
    .await;
    if let Ok(hexes) = rows {
        for h in hexes {
            if let Ok(bytes) = hex::decode(&h) {
                if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                    set.insert(arr);
                }
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        spectral_schema::init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[test]
    fn protected_is_runtime_and_extracted_only() {
        assert!(!Provenance::Ontology.is_protected());
        assert!(Provenance::Runtime.is_protected());
        assert!(Provenance::Extracted.is_protected());
    }

    #[tokio::test]
    async fn protected_set_contains_only_runtime_and_extracted() {
        let pool = fresh_pool().await;
        // 64-hex ids (32 bytes).
        let ont = "a".repeat(64);
        let run = "b".repeat(64);
        let ext = "c".repeat(64);
        record_provenance(&pool, &ont, Provenance::Ontology)
            .await
            .unwrap();
        record_provenance(&pool, &run, Provenance::Runtime)
            .await
            .unwrap();
        record_provenance(&pool, &ext, Provenance::Extracted)
            .await
            .unwrap();

        let protected = protected_entity_ids(&pool).await;
        assert_eq!(protected.len(), 2, "only runtime + extracted are protected");
        let run_bytes: [u8; 32] = hex::decode(&run).unwrap().try_into().unwrap();
        let ont_bytes: [u8; 32] = hex::decode(&ont).unwrap().try_into().unwrap();
        assert!(protected.contains(&run_bytes));
        assert!(!protected.contains(&ont_bytes), "ontology is NOT protected");
    }

    #[tokio::test]
    async fn record_is_first_source_wins() {
        let pool = fresh_pool().await;
        let id = "d".repeat(64);
        // Runtime first, then an ontology stamp: origin must stay runtime.
        record_provenance(&pool, &id, Provenance::Runtime)
            .await
            .unwrap();
        record_provenance(&pool, &id, Provenance::Ontology)
            .await
            .unwrap();
        let src: String =
            sqlx::query_scalar("SELECT source FROM entity_provenance WHERE entity_id_hex = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(src, "runtime", "first-source-wins keeps the runtime origin");
        // And it remains protected.
        assert_eq!(protected_entity_ids(&pool).await.len(), 1);
    }
}
