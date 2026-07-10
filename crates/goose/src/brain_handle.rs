//! `SafeBrain` — a newtype around `Arc<spectral::Brain>` that enforces
//! `spawn_blocking` at compile time.
//!
//! Every public method is `async` and internally moves the blocking Brain
//! call into `tokio::task::spawn_blocking`. This makes it impossible to
//! accidentally call a Brain method on the async executor — the compiler
//! is the reviewer.
//!
//! ## Sanctioned raw-Brain locations
//!
//! The only code permitted to touch `spectral::Brain` directly:
//! - This module (wraps the raw handle)
//! - `state.rs` construction block (builds Brain, wraps into SafeBrain)
//! - Functions using `raw_blocking_handle()` (must already be inside spawn_blocking)
//! - Test crates (`permagent-brain-tests`, `spectral_smoke`)

use std::sync::Arc;

/// Self-knowledge descriptor for the Brain surface (the memory view). Added in
/// Phase 2 — Phase 1 had no Brain descriptor. Static for brief rendering (the
/// surface section is editorial), but its lesson confirms via the queryable
/// `MemoryRecallable` proxy (`search_memory`). Co-located here; aggregated by
/// `crate::agents::self_knowledge`.
pub const BRAIN_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "brain",
        display_name: "Brain",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Your persistent memory — durable facts, conversations, and ingested content that survive across every session",
        why_it_matters:
            "It is what makes you continuous rather than a fresh chatbot each time; recall it before assuming you do not know something",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Open your Brain",
                body: "Show them where their memories live — this is the heart of what makes you persistent.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Brain",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Prove it remembers",
                body: "Ask them to tell you one durable fact about themselves, save it to memory, then recall it back — unlike an ordinary chatbot, you will still know it tomorrow and in every future session.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::MemoryRecallable(
                    "the fact they just told you about themselves",
                )),
            },
        ],
    };

/// Simplified cascade result carrying only the merged hits.
///
/// Wraps the fields callers actually use from the full CascadeResult
/// (which lives in the `spectral_cascade` crate and is not directly
/// nameable from this crate without adding a transitive dependency).
pub struct CascadeHits {
    pub merged_hits: Vec<spectral::ingest::MemoryHit>,
}

/// A thread-safe handle to `spectral::Brain` that enforces all operations
/// run off the async executor via `spawn_blocking`.
///
/// `Clone` is cheap (Arc clone). The inner `Brain` is never publicly accessible
/// except through the deterrent-named [`raw_blocking_handle()`](SafeBrain::raw_blocking_handle).
#[derive(Clone)]
pub struct SafeBrain {
    inner: Arc<spectral::Brain>,
}

impl SafeBrain {
    /// Wrap a freshly-built Brain. Call this inside the `spawn_blocking`
    /// construction block in `state.rs` — what leaves that block is a SafeBrain.
    pub fn new(brain: spectral::Brain) -> Self {
        Self {
            inner: Arc::new(brain),
        }
    }

    /// Wrap an already-Arc'd Brain. Used by test crates that construct
    /// `Arc<Brain>` directly (sanctioned raw-Brain users).
    pub fn from_arc(brain: Arc<spectral::Brain>) -> Self {
        Self { inner: brain }
    }

    /// Escape hatch: borrow the raw `spectral::Brain` for use inside an
    /// **already-entered** `spawn_blocking` context.
    ///
    /// # Contract
    ///
    /// The caller MUST already be on a blocking thread (inside
    /// `tokio::task::spawn_blocking`). Calling Brain methods on the async
    /// executor will stall it. This method exists only for functions that
    /// are themselves called from spawn_blocking and need the raw Brain
    /// reference. Callers of this method must carry a `_blocking` suffix
    /// (e.g. `run_consolidation_scan_blocking`, `load_entity_names_blocking`,
    /// `current_digest_blocking`).
    pub fn raw_blocking_handle(&self) -> &spectral::Brain {
        &self.inner
    }

    /// The device ID of the underlying Brain (non-blocking read).
    pub fn device_id(&self) -> &spectral::DeviceId {
        self.inner.device_id()
    }

    /// Check whether this SafeBrain wraps the same underlying Brain as another.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    // ── Async methods (each moves work into spawn_blocking) ──────────

    pub async fn recall(
        &self,
        query: &str,
        visibility: spectral::Visibility,
    ) -> anyhow::Result<spectral::HybridRecallResult> {
        let brain = self.inner.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || brain.recall(&query, visibility))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: recall: {e}"))?
            .map_err(Into::into)
    }

    /// Recall via the integrated cascade pipeline with default config.
    ///
    /// All existing callers pass `Default::default()` for the cascade config,
    /// so this wrapper hardcodes it. Returns merged hits with signal scores.
    pub async fn recall_cascade(
        &self,
        query: &str,
        context: &spectral::graph::RecognitionContext,
    ) -> anyhow::Result<CascadeHits> {
        let brain = self.inner.clone();
        let query = query.to_string();
        let context = context.clone();
        let result = tokio::task::spawn_blocking(move || {
            brain.recall_cascade(&query, &context, &Default::default())
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: recall_cascade: {e}"))?
        .map_err(anyhow::Error::from)?;
        Ok(CascadeHits {
            merged_hits: result.merged_hits,
        })
    }

    pub async fn remember_with(
        &self,
        key: &str,
        content: &str,
        opts: spectral::RememberOpts,
    ) -> anyhow::Result<spectral::RememberResult> {
        let brain = self.inner.clone();
        let key = key.to_string();
        let content = content.to_string();
        tokio::task::spawn_blocking(move || brain.remember_with(&key, &content, opts))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: remember_with: {e}"))?
            .map_err(Into::into)
    }

    pub async fn probe_recent(
        &self,
        window: spectral::ProbeWindow,
        opts: spectral::ProbeOpts,
    ) -> anyhow::Result<Vec<spectral::RecognizedMemory>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || brain.probe_recent(window, opts))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: probe_recent: {e}"))?
            .map_err(Into::into)
    }

    pub async fn list_undescribed(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<spectral::ingest::Memory>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || brain.list_undescribed(limit))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: list_undescribed: {e}"))?
            .map_err(Into::into)
    }

    /// Look up a memory by its logical **key** (not the derived id).
    ///
    /// Mirrors spectral's id derivation (`blake3(key)[..8]` as 16-hex, pinned
    /// rev 2c1f6bf). Used by the Reader for idempotency pre-checks so a re-drop
    /// of the same file skips OCR + summarization. Correctness does NOT depend
    /// on this — `remember_with` is itself idempotent via `WriteOutcome::NoOp`
    /// on a stable key — only the skip-redundant-work optimization does.
    pub async fn get_memory_by_key(
        &self,
        key: &str,
    ) -> anyhow::Result<Option<spectral::ingest::Memory>> {
        let id = format!(
            "{:016x}",
            u64::from_be_bytes(
                blake3::hash(key.as_bytes()).as_bytes()[..8]
                    .try_into()
                    .expect("blake3 digest is 32 bytes")
            )
        );
        self.get_memory(&id).await
    }

    pub async fn get_memory(&self, id: &str) -> anyhow::Result<Option<spectral::ingest::Memory>> {
        let brain = self.inner.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || brain.get_memory(&id))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: get_memory: {e}"))?
            .map_err(Into::into)
    }

    pub async fn set_description(&self, id: &str, description: &str) -> anyhow::Result<()> {
        let brain = self.inner.clone();
        let id = id.to_string();
        let description = description.to_string();
        tokio::task::spawn_blocking(move || brain.set_description(&id, &description))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: set_description: {e}"))?
            .map_err(Into::into)
    }

    /// Create (or idempotently return) a **person** node in the graph, returning
    /// its bare 64-hex `EntityId` — the people-bridge key.
    ///
    /// This is the *only* safe runtime graph-write for a person (people-in-graph
    /// v1, #583). It is narrow and validated — person-only `entity_type`, a
    /// canonicalized name, and it rejects a name that is empty after
    /// normalization — so it can never write a malformed node.
    ///
    /// The node materializes directly via `KuzuStore::upsert_entity`: a bare node
    /// needs no triple, `Brain::open` does not eager-seed the ontology, and
    /// `assert()` under the default `Strict` entity policy *rejects* entities
    /// absent from the ontology — so a triple-assert cannot create a novel person
    /// (the #583 no-eager-seed finding). Idempotent: re-creating an existing
    /// person returns its id and preserves the original `created_at`.
    ///
    /// Provenance is written *before* this call by [`crate::people_create`], so a
    /// graph node can never exist without protecting provenance.
    pub async fn create_person_entity(
        &self,
        display_name: &str,
        visibility: spectral::Visibility,
    ) -> anyhow::Result<String> {
        let canonical = crate::identity::canonical::graph_canonical(display_name);
        if canonical.is_empty() {
            anyhow::bail!("cannot create person: name is empty after normalization");
        }
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            use spectral::core::entity_id::entity_id;
            use spectral::graph::kuzu_store::Entity;

            let id = entity_id("person", &canonical);
            let store = brain.store();
            // Idempotent: preserve an existing node (and its created_at).
            if let Ok(Some(existing)) = store.get_entity(&id) {
                return Ok(hex::encode(existing.id.as_bytes()));
            }
            let now = chrono::Utc::now();
            let entity = Entity {
                id,
                entity_type: "person".to_string(),
                canonical,
                visibility,
                created_at: now,
                updated_at: now,
                weight: 1.0,
                description: None,
            };
            store
                .upsert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("upsert person entity: {e}"))?;
            Ok(hex::encode(entity.id.as_bytes()))
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: create_person_entity: {e}"))?
    }

    /// Look up a graph entity's canonical name by its bare 64-hex `EntityId`.
    /// `None` if no such node exists. Used by the graph-side people bridge
    /// ([`crate::people_bridge::sync_people_from_graph`]) to resolve runtime /
    /// extracted persons for minting — reads the live Brain, no second Kuzu
    /// connection.
    pub async fn entity_canonical(&self, id_hex: &str) -> anyhow::Result<Option<String>> {
        let brain = self.inner.clone();
        let id_hex = id_hex.to_string();
        tokio::task::spawn_blocking(move || {
            let id: spectral::core::entity_id::EntityId = id_hex
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid entity id hex: {e:?}"))?;
            match brain.store().get_entity(&id) {
                Ok(Some(e)) => Ok(Some(e.canonical)),
                Ok(None) => Ok(None),
                Err(e) => Err(anyhow::anyhow!("get_entity: {e}")),
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: entity_canonical: {e}"))?
    }

    /// Best-effort graph edge for a person→project association (#495 slice 3):
    /// assert `works_on(person, project)` directly against the store when both
    /// endpoints are known. The association row (permagent.db `project_people`)
    /// is the source of truth and the graph is allowed to lag behind it
    /// (people-in-graph v1, #583) — so an endpoint the graph can't name yet is
    /// a skip (`Ok(false)`), never a failure of the association itself.
    ///
    /// Resolution is deliberate: the person comes from their stored node (the
    /// `people.graph_entity_id` bridge — mention-resolution would miss
    /// runtime-created people, whose nodes bypass the in-process
    /// `runtime_entities` list), and the project from ontology
    /// canonicalization (alias + case aware). A project absent from the
    /// ontology has no graph identity yet and is skipped. An ontology-resolved
    /// project node is materialized on first edge (the ontology is not
    /// eager-seeded) — a curated entity, so this stays within the
    /// person-only-create rule for *novel* entities.
    ///
    /// Idempotent: an existing `works_on(person, project)` triple short-circuits
    /// to `Ok(false)`. Returns `Ok(true)` only when a triple was written.
    pub async fn assert_person_project_edge(
        &self,
        person_id_hex: &str,
        project_name: &str,
    ) -> anyhow::Result<bool> {
        let brain = self.inner.clone();
        let person_id_hex = person_id_hex.to_string();
        let project_name = project_name.to_string();
        tokio::task::spawn_blocking(move || {
            use spectral::graph::canonicalize::Canonicalizer;
            use spectral::graph::kuzu_store::{Entity, Triple};

            let person_id: spectral::core::entity_id::EntityId = person_id_hex
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid person entity id hex: {e:?}"))?;
            let store = brain.store();
            if store
                .get_entity(&person_id)
                .map_err(|e| anyhow::anyhow!("get_entity(person): {e}"))?
                .is_none()
            {
                return Ok(false); // person node lags; nothing to hang the edge on
            }

            let project = match Canonicalizer::new(brain.ontology()).resolve_one(&project_name) {
                Some(m) if m.entity_type == "project" => m,
                _ => return Ok(false), // not an ontology project (yet) — graph lags
            };

            let existing = store
                .find_triples(Some(&person_id), Some(&project.entity_id), Some("works_on"))
                .map_err(|e| anyhow::anyhow!("find_triples: {e}"))?;
            if !existing.is_empty() {
                return Ok(false);
            }

            let now = chrono::Utc::now();
            if store
                .get_entity(&project.entity_id)
                .map_err(|e| anyhow::anyhow!("get_entity(project): {e}"))?
                .is_none()
            {
                store
                    .upsert_entity(&Entity {
                        id: project.entity_id,
                        entity_type: "project".to_string(),
                        canonical: project.canonical.clone(),
                        visibility: spectral::Visibility::Private,
                        created_at: now,
                        updated_at: now,
                        weight: 1.0,
                        description: None,
                    })
                    .map_err(|e| anyhow::anyhow!("upsert project entity: {e}"))?;
            }

            brain
                .ontology()
                .validate_triple("works_on", "person", "project")
                .map_err(|e| anyhow::anyhow!("validate works_on: {e}"))?;
            store
                .insert_triple(&Triple {
                    from: person_id,
                    to: project.entity_id,
                    predicate: "works_on".to_string(),
                    confidence: 1.0,
                    source_doc_id: None,
                    source_brain_id: *brain.brain_id(),
                    asserted_at: now,
                    visibility: spectral::Visibility::Private,
                    weight: 1.0,
                })
                .map_err(|e| anyhow::anyhow!("insert works_on triple: {e}"))?;
            Ok(true)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: assert_person_project_edge: {e}"))?
    }

    /// Write a typed field on a graph entity, with provenance. The
    /// manual-not-clobbered rule is enforced in Spectral's store: an
    /// `Enriched` write never overwrites a field whose stored source is
    /// `Manual`. Returns `false` when the write was suppressed by that rule,
    /// `true` when applied.
    pub async fn set_entity_field(
        &self,
        entity_id: spectral::core::entity_id::EntityId,
        field_name: &str,
        value: &str,
        source: spectral::ingest::FieldSource,
        source_url: Option<&str>,
    ) -> anyhow::Result<bool> {
        let brain = self.inner.clone();
        let field_name = field_name.to_string();
        let value = value.to_string();
        let source_url = source_url.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            brain.set_entity_field(
                &entity_id,
                &field_name,
                &value,
                source,
                source_url.as_deref(),
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: set_entity_field: {e}"))?
        .map_err(Into::into)
    }

    /// Read all typed fields for a graph entity (provenance included).
    pub async fn get_entity_fields(
        &self,
        entity_id: spectral::core::entity_id::EntityId,
    ) -> anyhow::Result<Vec<spectral::ingest::EntityField>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || brain.get_entity_fields(&entity_id))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: get_entity_fields: {e}"))?
            .map_err(Into::into)
    }

    /// Batch-load typed fields for many entities in a single blocking hop
    /// (avoids one task dispatch per entity on the Brain-graph read path).
    /// Returns a map keyed by the entity's 64-hex id; entities with no fields
    /// are omitted.
    pub async fn entity_fields_for(
        &self,
        entity_ids: Vec<spectral::core::entity_id::EntityId>,
    ) -> anyhow::Result<std::collections::HashMap<String, Vec<spectral::ingest::EntityField>>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut out = std::collections::HashMap::new();
            for id in entity_ids {
                let fields = brain.get_entity_fields(&id)?;
                if !fields.is_empty() {
                    out.insert(id.to_string(), fields);
                }
            }
            Ok::<_, anyhow::Error>(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: entity_fields_for: {e}"))?
    }

    pub async fn consolidate_into(
        &self,
        source_keys: &[String],
        target_key: &str,
        opts: &spectral::ingest::ConsolidateOpts,
    ) -> anyhow::Result<spectral::ingest::ConsolidationResult> {
        let brain = self.inner.clone();
        let source_keys = source_keys.to_vec();
        let target_key = target_key.to_string();
        let opts = opts.clone();
        tokio::task::spawn_blocking(move || {
            brain.consolidate_into(&source_keys, &target_key, &opts)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: consolidate_into: {e}"))?
        .map_err(Into::into)
    }

    pub async fn rebuild_co_retrieval_index(&self) -> anyhow::Result<usize> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || brain.rebuild_co_retrieval_index())
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: rebuild_co_retrieval_index: {e}"))?
            .map_err(Into::into)
    }

    pub async fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> anyhow::Result<Vec<spectral::ingest::ConsolidationEdge>> {
        let brain = self.inner.clone();
        let target_key = target_key.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || brain.list_consolidated(target_key.as_deref()))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: list_consolidated: {e}"))?
            .map_err(Into::into)
    }
}

impl std::fmt::Debug for SafeBrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafeBrain").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that Clone shares the same underlying Arc (cheap clone).
    #[test]
    fn clone_shares_arc() {
        // We can't easily construct a real Brain in unit tests without a data dir,
        // but we can verify the Clone impl at the type level by confirming SafeBrain
        // is Clone. The actual sharing is guaranteed by Arc semantics.
        fn assert_clone<T: Clone>() {}
        assert_clone::<SafeBrain>();
    }
}
