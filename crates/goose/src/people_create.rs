//! The one runtime person-create path (people-in-graph v1, #583).
//!
//! Every runtime caller — the Henry `create_person` tool (#578), the UI
//! add-person route, and the v1.5 Librarian extraction — funnels through
//! [`create_person`], differing only in the [`Provenance`] they pass. The order
//! is the safety spec:
//!
//! 1. **Provenance first** (durable in `permagent.db`) — a graph node must never
//!    exist without protecting provenance, or the reconciler could prune it.
//! 2. **Graph node** — materialize the person in Kuzu (`create_person_entity`).
//! 3. **Directory row** — mint the identity-only `people` row via the bridge.
//!
//! If step 1 fails we abort before writing the node. If step 2 fails the orphan
//! provenance row is harmless (points at a nonexistent id, ignored by every
//! reader). So a runtime person can never end up unprotected.

use sqlx::{Pool, Sqlite};

use crate::brain_handle::SafeBrain;
use crate::identity::canonical::{self, EntityPrefix};
use crate::people::Person;
use crate::people_provenance::{self, Provenance};

/// Create a person at runtime: provenance → graph node → directory row. Returns
/// the minted (or pre-existing) [`Person`]. Idempotent by canonical name — the
/// content-addressed `EntityId` and the `people.canonical_id` UNIQUE key both
/// dedupe, so a repeat call returns the same person.
pub async fn create_person(
    pool: &Pool<Sqlite>,
    brain: &SafeBrain,
    display_name: &str,
    source: Provenance,
) -> Result<Person, String> {
    let canonical = canonical::graph_canonical(display_name);
    if canonical.is_empty() {
        return Err("cannot create person: name is empty after normalization".to_string());
    }
    let id_hex = canonical::graph_entity_id_hex("person", display_name);
    let canonical_id = canonical::canonicalize_entity_id(EntityPrefix::Person, display_name)
        .map_err(|e| format!("cannot canonicalize name: {e:?}"))?;

    // 1. Provenance FIRST (durable) — protects the node before it exists.
    people_provenance::record_provenance(pool, &id_hex, source).await?;

    // 2. Graph node — the person materializes in Kuzu.
    brain
        .create_person_entity(display_name, spectral::Visibility::Private)
        .await
        .map_err(|e| format!("create person graph node: {e}"))?;

    // 3. Directory row — identity-only; attributes stay graph-authoritative
    //    (Decision A, #255) and land later via the slice-2b write path.
    crate::people_bridge::upsert_identity_from_graph(pool, &canonical, &id_hex).await?;

    let person = crate::people::get_by_canonical_id(pool, &canonical_id)
        .await?
        .ok_or_else(|| "person row not found after mint".to_string())?;

    tracing::info!(
        target: "permagentd::people_create",
        display_name = %display_name,
        entity_id = %id_hex,
        source = source.as_str(),
        "Runtime person created (provenance + graph node + directory row)"
    );

    // Publish HERE, not in the callers. `peopleRev` is client-local and the
    // People surfaces have no poll backstop (unlike goals/decisions), so a
    // person that reaches the database without reaching the bus is invisible
    // in every open window — including the one that asked for it — until the
    // user reloads. The REST route used to emit and the `create_person` tool
    // did not, which is exactly how an agent-created person read as a no-op.
    // This is the one create path, so this is the one place that announces it.
    crate::events::emit(crate::events::person_changed(
        "",
        &person.entity_uuid,
        "created",
    ));

    Ok(person)
}
