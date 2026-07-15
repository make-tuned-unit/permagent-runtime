//! People ↔ Spectral graph bridge population (#255 Decision B).
//!
//! Mints/backfills identity-only `people` rows from the Spectral graph's person
//! entities so the CRM directory shows everyone the Brain knows — e.g. "mel
//! schembri", who exists in the graph but had no `people` row (the live gap this
//! bridge closes). Each row's immutable `graph_entity_id` is the graph's REAL
//! `EntityId` (read from the ontology via `entity_id_for`), never derived from the
//! mutable `canonical_id` (deriving it would mis-join — see
//! [`crate::identity::canonical::graph_canonical`]).
//!
//! The ontology is the authoritative person set: the live graph is seeded from it,
//! so its person ids correspond to graph nodes. Reading the ids here (rather than
//! querying the graph store) avoids opening a second connection while the Brain
//! holds the graph open.

use std::path::Path;

use sqlx::{Pool, Sqlite};

use crate::identity::canonical::{self, EntityPrefix};

/// Title-case a lowercased graph canonical into a display name
/// (`"mel schembri"` → `"Mel Schembri"`). An identity-only default; the person
/// can rename it later (slice 2b).
fn title_case(canonical: &str) -> String {
    canonical
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Mirror one graph person entity into an identity-only `people` row.
///
/// `canonical` is the graph's canonical form (lowercased, e.g. `"mel schembri"`);
/// `graph_entity_id` is its bare 64-hex `EntityId`. A new person gets a freshly
/// minted `entity_uuid`, a title-cased `display_name`, and the bridge key. An
/// existing row keeps its identity and only has a NULL `graph_entity_id` filled in
/// — never clobbered (the key is immutable). Returns `true` if a row was minted or
/// backfilled, `false` if it was already bridged.
pub async fn upsert_identity_from_graph(
    pool: &Pool<Sqlite>,
    canonical: &str,
    graph_entity_id: &str,
) -> Result<bool, String> {
    let canonical_id = match canonical::canonicalize_entity_id(EntityPrefix::Person, canonical) {
        Ok(c) => c,
        // Empty-after-normalization (e.g. punctuation-only canonical): skip.
        Err(_) => return Ok(false),
    };
    let display_name = title_case(canonical);
    let uuid = uuid::Uuid::now_v7().to_string();

    let affected = sqlx::query(
        "INSERT INTO people (entity_uuid, canonical_id, display_name, graph_entity_id)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(canonical_id) DO UPDATE SET
             graph_entity_id = excluded.graph_entity_id,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE people.graph_entity_id IS NULL",
    )
    .bind(&uuid)
    .bind(&canonical_id)
    .bind(&display_name)
    .bind(graph_entity_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    Ok(affected > 0)
}

/// Mirror all graph person entities into identity-only `people` rows.
///
/// Idempotent: mints rows for graph persons absent from the table and backfills
/// `graph_entity_id` on rows that predate the bridge, leaving already-bridged rows
/// untouched. Returns the number of rows minted or backfilled this pass.
pub async fn sync_people_from_ontology(
    pool: &Pool<Sqlite>,
    ontology_path: &Path,
) -> Result<usize, String> {
    let ontology = spectral::graph::ontology::Ontology::load(ontology_path)
        .map_err(|e| format!("load ontology: {e}"))?;

    let mut touched = 0usize;
    for entity in ontology
        .entities
        .iter()
        .filter(|e| e.entity_type == "person")
    {
        let graph_entity_id = hex::encode(ontology.entity_id_for(entity).as_bytes());
        // Stamp provenance=ontology (people-in-graph v1 #583). First-source-wins,
        // so a person created at runtime and later curated keeps its runtime
        // origin (Decision G). Non-fatal — the mint proceeds regardless.
        let _ = crate::people_provenance::record_provenance(
            pool,
            &graph_entity_id,
            crate::people_provenance::Provenance::Ontology,
        )
        .await;
        if upsert_identity_from_graph(pool, &entity.canonical, &graph_entity_id).await? {
            touched += 1;
        }
    }
    Ok(touched)
}

/// Mirror **runtime / extracted** graph persons into identity-only `people` rows —
/// the graph-side counterpart to [`sync_people_from_ontology`] (people-in-graph
/// v1, #583).
///
/// Together the two are the *union mint*: curated persons (from the ontology) plus
/// emergent persons (created at runtime by the Henry tool / UI, or extracted by
/// the v1.5 Librarian). Ontology persons keep minting through the other function;
/// this one handles only the provenance-protected ids so the two never contend.
///
/// Idempotent: skips ids that already have a `people` row, and resolves each
/// remaining id's canonical from the **live** Brain (no second Kuzu connection).
/// A protected id whose graph node is gone is silently skipped. Returns the number
/// of rows minted this pass.
pub async fn sync_people_from_graph(
    pool: &Pool<Sqlite>,
    brain: &crate::brain_handle::SafeBrain,
) -> Result<usize, String> {
    let protected = crate::people_provenance::protected_entity_ids(pool).await;
    let mut touched = 0usize;
    for id_bytes in protected {
        let id_hex = hex::encode(id_bytes);
        // Already minted? (the people row carries graph_entity_id) — skip without
        // touching the graph.
        let existing: Option<String> =
            sqlx::query_scalar("SELECT entity_uuid FROM people WHERE graph_entity_id = ?")
                .bind(&id_hex)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?;
        if existing.is_some() {
            continue;
        }
        // Resolve the canonical from the live graph; skip if the node is gone.
        let Some(canonical) = brain
            .entity_canonical(&id_hex)
            .await
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if upsert_identity_from_graph(pool, &canonical, &id_hex).await? {
            touched += 1;
        }
    }
    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::{self, PersonAttrs};
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
    fn title_case_capitalizes_each_word() {
        assert_eq!(title_case("mel schembri"), "Mel Schembri");
        assert_eq!(title_case("jesse  sharratt"), "Jesse Sharratt");
        assert_eq!(title_case(""), "");
    }

    #[tokio::test]
    async fn mints_identity_row_for_graph_person() {
        let pool = fresh_pool().await;
        // "mel schembri" lives in the graph but has no people row — the live gap.
        let gid = canonical::graph_entity_id_hex("person", "mel schembri");
        let touched = upsert_identity_from_graph(&pool, "mel schembri", &gid)
            .await
            .unwrap();
        assert!(touched);

        let row = people::get_by_canonical_id(&pool, "person:mel-schembri")
            .await
            .unwrap()
            .expect("row minted");
        assert_eq!(row.display_name, "Mel Schembri");
        assert_eq!(row.graph_entity_id.as_deref(), Some(gid.as_str()));
        assert_eq!(gid.len(), 64); // bare blake3 hex, no "e:" prefix
    }

    #[tokio::test]
    async fn second_pass_is_idempotent() {
        let pool = fresh_pool().await;
        let gid = canonical::graph_entity_id_hex("person", "mel schembri");
        assert!(upsert_identity_from_graph(&pool, "mel schembri", &gid)
            .await
            .unwrap());
        // Already bridged — no-op (WHERE graph_entity_id IS NULL guards it).
        assert!(!upsert_identity_from_graph(&pool, "mel schembri", &gid)
            .await
            .unwrap());

        let all = people::list_people(&pool, &Default::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn backfills_preexisting_row_without_clobbering_identity() {
        let pool = fresh_pool().await;
        // A CRM-created person renamed to a slug that no longer matches the graph
        // canonical: simulate a pre-bridge row by nulling its graph_entity_id.
        let p = people::upsert_person(
            &pool,
            "person:mel-schembri",
            "Mr Mel",
            &PersonAttrs::default(),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE people SET graph_entity_id = NULL WHERE entity_uuid = ?")
            .bind(&p.entity_uuid)
            .execute(&pool)
            .await
            .unwrap();

        let gid = canonical::graph_entity_id_hex("person", "mel schembri");
        assert!(upsert_identity_from_graph(&pool, "mel schembri", &gid)
            .await
            .unwrap());

        let after = people::get_by_uuid(&pool, &p.entity_uuid)
            .await
            .unwrap()
            .unwrap();
        // graph_entity_id filled, identity (uuid + user's display_name) preserved.
        assert_eq!(after.entity_uuid, p.entity_uuid);
        assert_eq!(after.display_name, "Mr Mel");
        assert_eq!(after.graph_entity_id.as_deref(), Some(gid.as_str()));
    }
}
