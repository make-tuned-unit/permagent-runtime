//! Project graph identity + `works_on` residue cleanup (#595, CRM slice 3
//! follow-up).
//!
//! Two halves, both scoped to the person→project association flow:
//!
//! 1. **Identity** — [`ensure_project_graph_identity`] gives every project a
//!    graph identity at associate time. An ontology project resolves (alias +
//!    case aware) and materializes as before; a **non-ontology** project — one
//!    created in the Projects tab — is runtime-minted through the same
//!    provenance-first discipline people got in #583: durable `runtime`
//!    provenance in `permagent.db` *before* the node exists, so the prune-only
//!    reconciler can never delete it. The minted id lands in the
//!    `projects.graph_entity_id` bridge column (fill-if-NULL, immutable).
//!    This widens PEOPLE_GRAPH_V1 Decision C's person-only-create rule to
//!    projects, exactly as that ruling anticipated.
//!
//! 2. **Deletion** — [`delete_works_on_triples`] removes the `works_on`
//!    triple(s) on disassociate, so the Brain view stops drawing the line once
//!    the authoritative `project_people` row is gone.
//!
//! ## Why deletion is direct SQL (STOP-and-flag, #595 option (a) vs (b))
//!
//! The pinned `spectral` rev exposes **no** triple deletion — checked at pin
//! `fb1038db` and again at spectral main HEAD (`05c5065`): `GraphStore` has
//! only insert/find/neighborhood. Option (a) (add `delete_triple` upstream)
//! is cross-repo and must ride a deliberate #497-style pin bump once it lands
//! in spectral main. Meanwhile the store is no longer Kuzu — spectral main
//! collapsed the graph onto SQLite (`brain/graph.sqlite`) — so the permagent
//! side can follow its own established direct-SQL precedent (the Librarian
//! pruning path DELETEs rows straight out of the Brain's `memory.db`).
//! [`delete_works_on_triples`] is that stopgap: a scoped
//! `DELETE FROM triple WHERE from_id = ? AND to_id = ? AND predicate =
//! 'works_on'` against `graph.sqlite`. Replace it with
//! `store.delete_triple(...)` when spectral grows the API. The unit test below
//! round-trips through spectral's own `GraphStore`, so a pin bump that changes
//! the triple schema fails CI here instead of silently orphaning the DELETE.

use std::path::Path;

use sqlx::{Pool, Sqlite};

use crate::brain_handle::SafeBrain;
use crate::identity::canonical::{graph_canonical, graph_entity_id_hex};
use crate::people_provenance::{self, Provenance};
use crate::projects::{self, Project};

/// Resolve — or mint — the graph identity for a project at associate time.
///
/// Resolution order:
/// 1. **Ontology** (alias + case aware): the curated identity wins; its node is
///    materialized if missing (the ontology is not eager-seeded). No provenance
///    row — the reconciler owns ontology entities via the ontology id set.
/// 2. **Stored bridge column**: a previously minted identity is immutable. If
///    the stored id no longer matches the name-derived id (the project was
///    renamed after minting), the stored identity still wins — no re-mint under
///    a new name (mirrors `people.graph_entity_id` immutability).
/// 3. **Runtime mint** (#595, widened Decision C): durable `runtime` provenance
///    FIRST, then the node, then the bridge column backfill — the #583 ordering
///    that guarantees a runtime node can never exist unprotected.
///
/// Returns `Ok(None)` only when the project name is empty after normalization
/// (no graph identity is possible). Best-effort caller contract: the
/// association row is the source of truth; an `Err` here is logged upstream,
/// never a request failure.
pub async fn ensure_project_graph_identity(
    pool: &Pool<Sqlite>,
    brain: &SafeBrain,
    project: &Project,
) -> Result<Option<String>, String> {
    // 1. Ontology identity (materializes the node if missing).
    if let Some(id_hex) = brain
        .materialize_ontology_project(&project.name)
        .await
        .map_err(|e| format!("ontology project resolve: {e}"))?
    {
        projects::set_graph_entity_id(pool, &project.id, &id_hex).await?;
        return Ok(Some(id_hex));
    }

    // 2. / 3. Runtime identity.
    if graph_canonical(&project.name).is_empty() {
        return Ok(None);
    }
    let computed = graph_entity_id_hex("project", &project.name);
    if let Some(stored) = project.graph_entity_id.as_deref() {
        if stored != computed {
            // Renamed since minting: the stored identity is immutable and wins.
            // Its node either exists (normal case) or lags (graph wiped) — the
            // edge assert skips gracefully in the latter case.
            return Ok(Some(stored.to_string()));
        }
    }

    // Provenance FIRST (durable) — protects the node before it exists (#583).
    people_provenance::record_provenance(pool, &computed, Provenance::Runtime).await?;
    let id_hex = brain
        .create_project_entity(&project.name, spectral::Visibility::Private)
        .await
        .map_err(|e| format!("create project graph node: {e}"))?;
    debug_assert_eq!(id_hex, computed, "content-addressed ids must agree");
    projects::set_graph_entity_id(pool, &project.id, &id_hex).await?;

    tracing::info!(
        target: "permagentd::project_graph",
        project = %project.id,
        name = %project.name,
        entity_id = %id_hex,
        "Runtime project graph identity minted (provenance + node + bridge column)"
    );
    Ok(Some(id_hex))
}

/// The candidate graph ids a project's `works_on` residue could have been
/// asserted under — read-only (never creates nodes; safe on delete paths):
///
/// * the stored `projects.graph_entity_id` bridge column,
/// * the ontology-resolved id (alias + case aware) — covers pre-#595 residue,
///   which was always asserted under the ontology identity,
/// * the name-derived runtime id.
///
/// Deduplicated; usually these collapse to a single id.
pub async fn project_graph_id_candidates(brain: &SafeBrain, project: &Project) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(stored) = project.graph_entity_id.as_deref() {
        out.push(stored.to_string());
    }
    match brain.resolve_ontology_project_id(&project.name).await {
        Ok(Some(id)) => {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(
            target: "permagentd::project_graph",
            project = %project.id, error = %e,
            "ontology resolve failed while collecting residue candidates"
        ),
    }
    if !graph_canonical(&project.name).is_empty() {
        let computed = graph_entity_id_hex("project", &project.name);
        if !out.contains(&computed) {
            out.push(computed);
        }
    }
    out
}

/// Delete the `works_on(person, project)` triple(s) from the Brain's graph
/// store — the disassociate-side cleanup of #495 slice 3's populate (#595).
///
/// Direct SQL against `graph.sqlite` (see the module docs for why the pinned
/// spectral API cannot do this yet). Scoped to exactly the `works_on`
/// predicate between the given person and the candidate project ids — other
/// predicates, other pairs, and both entity nodes are untouched (nodes are
/// identity, not residue). A missing database is a clean no-op (`Ok(0)`) —
/// the graph never materialized, so there is nothing to clean.
///
/// Synchronous (rusqlite): call from `spawn_blocking` on async paths.
pub fn delete_works_on_triples(
    graph_db_path: &Path,
    person_id_hex: &str,
    project_id_hexes: &[String],
) -> Result<usize, String> {
    if !graph_db_path.exists() {
        return Ok(0);
    }
    let from = hex::decode(person_id_hex).map_err(|e| format!("bad person id hex: {e}"))?;
    if from.len() != 32 {
        return Err(format!("person id must be 32 bytes, got {}", from.len()));
    }
    let conn =
        rusqlite::Connection::open(graph_db_path).map_err(|e| format!("open graph db: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("busy_timeout: {e}"))?;
    let mut deleted = 0usize;
    for project_hex in project_id_hexes {
        let to = hex::decode(project_hex).map_err(|e| format!("bad project id hex: {e}"))?;
        if to.len() != 32 {
            return Err(format!("project id must be 32 bytes, got {}", to.len()));
        }
        let n = conn
            .execute(
                "DELETE FROM triple
                 WHERE from_id = ?1 AND to_id = ?2 AND predicate = 'works_on'",
                rusqlite::params![from, to],
            )
            .map_err(|e| format!("delete works_on triple: {e}"))?;
        deleted += n;
    }
    Ok(deleted)
}

/// Delete exactly one typed directed graph edge. This is the same temporary
/// direct-SQL bridge as `delete_works_on_triples`; Spectral has no delete API.
pub fn delete_graph_triple(
    graph_db_path: &Path,
    from_hex: &str,
    to_hex: &str,
    predicate: &str,
) -> Result<usize, String> {
    if !graph_db_path.exists() {
        return Ok(0);
    }
    let from = hex::decode(from_hex).map_err(|e| format!("bad source id hex: {e}"))?;
    let to = hex::decode(to_hex).map_err(|e| format!("bad target id hex: {e}"))?;
    if from.len() != 32 || to.len() != 32 {
        return Err("graph ids must be 32 bytes".into());
    }
    let conn =
        rusqlite::Connection::open(graph_db_path).map_err(|e| format!("open graph db: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM triple WHERE from_id = ?1 AND to_id = ?2 AND predicate = ?3",
        rusqlite::params![from, to, predicate],
    )
    .map_err(|e| format!("delete graph triple: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::graph::graph_store::{Entity, GraphStore, Triple};

    fn eid(entity_type: &str, canonical: &str) -> spectral::core::entity_id::EntityId {
        spectral::core::entity_id::entity_id(entity_type, canonical)
    }

    fn entity(entity_type: &str, canonical: &str) -> Entity {
        let now = chrono::Utc::now();
        Entity {
            id: eid(entity_type, canonical),
            entity_type: entity_type.to_string(),
            canonical: canonical.to_string(),
            visibility: spectral::Visibility::Private,
            created_at: now,
            updated_at: now,
            weight: 1.0,
            description: None,
        }
    }

    fn triple(
        from: spectral::core::entity_id::EntityId,
        to: spectral::core::entity_id::EntityId,
        predicate: &str,
    ) -> Triple {
        Triple {
            from,
            to,
            predicate: predicate.to_string(),
            confidence: 1.0,
            source_doc_id: None,
            source_brain_id: spectral::core::identity::BrainId::from_bytes([7u8; 32]),
            asserted_at: chrono::Utc::now(),
            visibility: spectral::Visibility::Private,
            weight: 1.0,
        }
    }

    /// Round-trip through spectral's own `GraphStore`: insert a `works_on`
    /// triple, delete it via the direct-SQL path, and verify exactly it is
    /// gone — the other predicate on the same pair, the same predicate on a
    /// different pair, and both entity nodes all survive. This test is the
    /// schema-coupling tripwire: a spectral pin bump that changes the triple
    /// table breaks here, not silently in production.
    #[test]
    fn delete_removes_exactly_the_works_on_triple() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("graph.sqlite");
        let store = GraphStore::open(&db).unwrap();

        let alice = eid("person", "alice");
        let acme = eid("project", "acme");
        let other = eid("project", "otherproj");
        store.upsert_entity(&entity("person", "alice")).unwrap();
        store.upsert_entity(&entity("project", "acme")).unwrap();
        store
            .upsert_entity(&entity("project", "otherproj"))
            .unwrap();
        store
            .insert_triple(&triple(alice, acme, "works_on"))
            .unwrap();
        store.insert_triple(&triple(alice, acme, "leads")).unwrap();
        store
            .insert_triple(&triple(alice, other, "works_on"))
            .unwrap();

        let n = delete_works_on_triples(
            &db,
            &hex::encode(alice.as_bytes()),
            &[hex::encode(acme.as_bytes())],
        )
        .unwrap();
        assert_eq!(n, 1, "exactly the alice→acme works_on triple is deleted");

        // The targeted triple is gone…
        assert!(store
            .find_triples(Some(&alice), Some(&acme), Some("works_on"))
            .unwrap()
            .is_empty());
        // …its neighbors survive: other predicate, other pair, both nodes.
        assert_eq!(
            store
                .find_triples(Some(&alice), Some(&acme), Some("leads"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .find_triples(Some(&alice), Some(&other), Some("works_on"))
                .unwrap()
                .len(),
            1
        );
        assert!(store.get_entity(&alice).unwrap().is_some());
        assert!(store.get_entity(&acme).unwrap().is_some());
    }

    #[test]
    fn typed_delete_removes_only_the_requested_person_edge() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("graph.sqlite");
        let store = GraphStore::open(&db).unwrap();
        let alice = eid("person", "alice");
        let bob = eid("person", "bob");
        store.upsert_entity(&entity("person", "alice")).unwrap();
        store.upsert_entity(&entity("person", "bob")).unwrap();
        store
            .insert_triple(&triple(alice, bob, "colleague"))
            .unwrap();
        store.insert_triple(&triple(alice, bob, "manager")).unwrap();

        assert_eq!(
            delete_graph_triple(&db, &alice.to_string(), &bob.to_string(), "colleague").unwrap(),
            1
        );
        assert!(store
            .find_triples(Some(&alice), Some(&bob), Some("colleague"))
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .find_triples(Some(&alice), Some(&bob), Some("manager"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn delete_is_a_noop_without_a_triple_or_a_database() {
        let tmp = tempfile::tempdir().unwrap();

        // No database at all: clean no-op.
        let missing = tmp.path().join("nope").join("graph.sqlite");
        let a = hex::encode(eid("person", "alice").as_bytes());
        let b = vec![hex::encode(eid("project", "acme").as_bytes())];
        assert_eq!(delete_works_on_triples(&missing, &a, &b).unwrap(), 0);

        // Database exists but the triple does not: 0 rows, no error.
        let db = tmp.path().join("graph.sqlite");
        let _store = GraphStore::open(&db).unwrap();
        assert_eq!(delete_works_on_triples(&db, &a, &b).unwrap(), 0);
    }

    #[test]
    fn delete_rejects_malformed_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("graph.sqlite");
        let _store = GraphStore::open(&db).unwrap();
        let good = hex::encode(eid("person", "alice").as_bytes());
        assert!(delete_works_on_triples(&db, "zz-not-hex", std::slice::from_ref(&good)).is_err());
        assert!(delete_works_on_triples(&db, &good, &["abcd".to_string()]).is_err());
    }
}
