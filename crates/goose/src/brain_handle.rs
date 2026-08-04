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
            "Your persistent memory — durable facts, conversations, and ingested content that survive across every session. The graph draws real connection lines between entities — person-to-project working relationships included",
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
                body: "Ask them to tell you one durable fact about themselves; it is written to your Brain automatically — memory is captured by ingestion, the Reader, notes, and consolidation, never by a save tool you call — then recall it back with search_memory: unlike an ordinary chatbot, you will still know it tomorrow and in every future session.",
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

/// A graph entity's identity + live description state, as read from the Kuzu
/// store. The unit the #387-v2 Librarian entity pass plans work over: `id` is
/// the content-addressed key `entity_id(entity_type, canonical)` exactly as the
/// store minted it, and `description` is the current card text (`None` when
/// unset or empty).
#[derive(Debug, Clone)]
pub struct GraphEntitySnapshot {
    pub id: spectral::core::entity_id::EntityId,
    pub entity_type: String,
    pub canonical: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersonGraphEdge {
    pub from_id: String,
    pub to_id: String,
    pub predicate: String,
}

/// Outcome of resolving a free-text name against the ontology + graph store
/// (see [`SafeBrain::resolve_ontology_entities_exact`]).
#[derive(Debug, Clone)]
pub enum OntologyEntityResolution {
    /// Not an ontology entity — no graph identity exists for this name.
    NoIdentity,
    /// The ontology declares it, but the node was never materialized in Kuzu
    /// (the ontology is not eager-seeded) — describing it would MERGE a
    /// half-formed node, so callers must skip it.
    NotInGraph,
    /// A live graph node, with the ontology's alias list for mention matching.
    InGraph {
        snapshot: GraphEntitySnapshot,
        aliases: Vec<String>,
    },
}

/// Permagent's name for Spectral's visibility ladder — the four-level total
/// order `Private < Team < Org < Public` (`spectral_core::visibility::Visibility`).
///
/// This is a **1:1 typed alias** over the shipped Spectral enum, not a new
/// level: it exists so Permagent (and the product surface / website) can name
/// the ladder in Permagent's own vocabulary without leaking the Spectral type,
/// and so the mapping is documented in exactly one place. Round-trips losslessly
/// via [`to_visibility`](MemoryScope::to_visibility) /
/// [`from_visibility`](MemoryScope::from_visibility).
///
/// ## Read semantics (what the ladder actually does)
///
/// Recall takes a *clearance floor*: a memory at visibility `V` surfaces in a
/// recall with clearance `C` iff `V >= C` (`Visibility::allows`,
/// `spectral-core/visibility.rs`). So a recall at `Private` clearance sees
/// **everything**; a recall at `Team` clearance hides `Private` memories; a
/// recall at `Public` clearance sees only `Public`. This is the honest,
/// shipped read filter — **not** an export boundary and **not** a
/// confidentiality barrier against a hostile local process (see the federation
/// security spec, "the overclaim to avoid").
///
/// ## The default is unchanged
///
/// [`MemoryScope::default()`] is `Private`, matching
/// [`Visibility::default()`]. Nothing in Permagent flips the global default —
/// non-`Private` levels are *expressible* (via
/// [`SafeBrain::remember_scoped`]), never *imposed*.
///
/// ## Not to be confused with a *wing* (the offboarding axis)
///
/// A `MemoryScope`/`Visibility` is a **read filter** (design-doc Axis B). A
/// **wing** — the `wing` string on a memory (`RememberOpts.wing`), what
/// [`SafeBrain::forget_scope`] sweeps — is a separate topical/scope axis
/// (Axis A-adjacent). A permagent "Project" or "Company" scope maps onto a
/// **wing** (a `String` slug), *not* onto a visibility level; do not invent a
/// fake ladder rung for it. The two axes are orthogonal and share no value:
/// a memory can be `Team`-visible yet in the `"acme"` wing, or `Private` and
/// wingless. (This is the design doc's Q3 terminology-collision hazard, called
/// out here so callers keep the axes distinct.)
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// Most restrictive. The default; local, personal memory.
    #[default]
    Private,
    /// Shared with a team clearance and broader.
    Team,
    /// Shared with an org clearance and broader.
    Org,
    /// Least restrictive; visible at every clearance.
    Public,
}

impl MemoryScope {
    /// Map to the shipped Spectral visibility level (lossless).
    pub fn to_visibility(self) -> spectral::Visibility {
        match self {
            MemoryScope::Private => spectral::Visibility::Private,
            MemoryScope::Team => spectral::Visibility::Team,
            MemoryScope::Org => spectral::Visibility::Org,
            MemoryScope::Public => spectral::Visibility::Public,
        }
    }

    /// Map back from a Spectral visibility level (lossless).
    pub fn from_visibility(v: spectral::Visibility) -> Self {
        match v {
            spectral::Visibility::Private => MemoryScope::Private,
            spectral::Visibility::Team => MemoryScope::Team,
            spectral::Visibility::Org => MemoryScope::Org,
            spectral::Visibility::Public => MemoryScope::Public,
        }
    }

    /// Canonical lowercase slug (matches the persisted `memories.visibility`
    /// string and the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryScope::Private => "private",
            MemoryScope::Team => "team",
            MemoryScope::Org => "org",
            MemoryScope::Public => "public",
        }
    }
}

impl From<MemoryScope> for spectral::Visibility {
    fn from(s: MemoryScope) -> Self {
        s.to_visibility()
    }
}

impl From<spectral::Visibility> for MemoryScope {
    fn from(v: spectral::Visibility) -> Self {
        MemoryScope::from_visibility(v)
    }
}

/// Aggregate outcome of a scope sweep ([`SafeBrain::forget_scope`] /
/// [`SafeBrain::forget_keys`]): a per-key roll-up of the verified
/// [`ForgetReport`](spectral::graph::brain::ForgetReport)s produced by hard-
/// deleting every memory in a scope.
///
/// This is the audit substrate for design-doc Step 4 ("local scope-forget"):
/// it records how many memories were swept, how many were *verified* gone
/// (recall + recognition both clear), and — critically — how many associated
/// **graph triples** were removed.
///
/// ### Graph-triple residual (Spectral-gated — see `graph_triples_deleted`)
///
/// `graph_triples_deleted` is **always 0 at Spectral pin `fb1038db`**: Spectral
/// exposes no triple/entity delete API and its `triple` rows carry no scope
/// key, so company-derived *graph* facts survive the memory sweep. This is the
/// design doc's **Q2** gap and is flagged, not silently dropped. See
/// `docs/design/sovereign-offboarding-phase1-notes.md` for the exact missing
/// Spectral surface. Until it lands, a scope sweep is honestly described as
/// "memories hard-deleted and verified; graph triples pending Spectral API".
#[derive(Debug, Clone, Default)]
pub struct ScopeForgetReport {
    /// The wing (scope) that was swept, if this came from [`SafeBrain::forget_scope`].
    pub wing: Option<String>,
    /// Number of memory keys the sweep attempted to forget.
    pub keys_swept: usize,
    /// Of those, how many actually existed in the store (`store.existed`).
    pub existed: usize,
    /// Of those, how many are *verified* gone — every substrate deleted and
    /// both the recall and recognition probes clear
    /// ([`ForgetReport::fully_forgotten`](spectral::graph::brain::ForgetReport::fully_forgotten)).
    pub fully_forgotten: usize,
    /// Graph triples deleted by the sweep. **Always 0 at pin `fb1038db`** — the
    /// Spectral-gated Q2 residual (no triple-delete API). Non-zero only once
    /// Spectral ships a scoped triple delete.
    pub graph_triples_deleted: usize,
    /// The forgotten keys, in sweep order (for the audit receipt).
    pub forgotten_keys: Vec<String>,
    /// Keys still present after the bounded sweep. A non-empty list means the
    /// scope was not completely forgotten and prevents callers from treating
    /// this receipt as a clean sweep.
    pub residual_keys: Vec<String>,
}

impl ScopeForgetReport {
    /// Fold one per-key [`ForgetReport`](spectral::graph::brain::ForgetReport)
    /// into the aggregate.
    fn absorb(&mut self, key: &str, report: &spectral::graph::brain::ForgetReport) {
        self.keys_swept += 1;
        if report.store.existed {
            self.existed += 1;
        }
        if report.fully_forgotten() {
            self.fully_forgotten += 1;
        }
        self.forgotten_keys.push(key.to_string());
    }
}

fn write_manual_entity_fields(
    db_path: &std::path::Path,
    entity_id: &str,
    fields: Vec<(String, String)>,
) -> anyhow::Result<()> {
    let mut conn = rusqlite::Connection::open(db_path)
        .map_err(|e| anyhow::anyhow!("manual field batch: open {}: {e}", db_path.display()))?;
    let tx = conn
        .transaction()
        .map_err(|e| anyhow::anyhow!("manual field batch: begin: {e}"))?;
    for (name, value) in fields {
        tx.execute(
            "INSERT INTO entity_fields \
                 (entity_id, field_name, value, source, source_url, updated_at) \
             VALUES (?1, ?2, ?3, 'manual', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) \
             ON CONFLICT(entity_id, field_name) DO UPDATE SET \
                 value = excluded.value, source = excluded.source, \
                 source_url = excluded.source_url, updated_at = excluded.updated_at",
            rusqlite::params![entity_id, name, value],
        )
        .map_err(|e| anyhow::anyhow!("manual field batch ({name}): {e}"))?;
    }
    tx.commit()
        .map_err(|e| anyhow::anyhow!("manual field batch: commit: {e}"))
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

    /// Recall via the integrated cascade pipeline.
    ///
    /// Uses `CascadePipelineConfig::default()` for every layer except `spread`
    /// (associative recall), which is resolved from the `PERMAGENT_ACR_MODE`
    /// toggle via [`acr_spread_config`]. ACR is experimental and **OFF by
    /// default**, so unless the A/B toggle is set this behaves exactly as before.
    /// Returns merged hits with signal scores.
    pub async fn recall_cascade(
        &self,
        query: &str,
        context: &spectral::graph::RecognitionContext,
    ) -> anyhow::Result<CascadeHits> {
        let brain = self.inner.clone();
        let query = query.to_string();
        let context = context.clone();
        let config = spectral::graph::cascade_layers::CascadePipelineConfig {
            spread: acr_spread_config(),
            ..Default::default()
        };
        let result =
            tokio::task::spawn_blocking(move || brain.recall_cascade(&query, &context, &config))
                .await
                .map_err(|e| anyhow::anyhow!("brain task panicked: recall_cascade: {e}"))?
                .map_err(anyhow::Error::from)?;
        Ok(CascadeHits {
            merged_hits: result.merged_hits,
        })
    }

    /// Read-only retrieval that returns a receipt (`Brain::turn`).
    ///
    /// Unlike [`recall_cascade`](Self::recall_cascade), this reinforces NOTHING
    /// at retrieval time — the whole point. `recall_*` auto-reinforces every
    /// hit, which credits exposure rather than usefulness; `turn` writes only a
    /// delivery record and waits to be told what was actually used, via
    /// [`record_turn_outcome`](Self::record_turn_outcome).
    ///
    /// **Not the default recall path.** Spectral's own preregistered latency
    /// gate FAILED on it: recall-only p95 regressed +87–100% against a +5% kill
    /// line, caused by the synchronous delivery-write commit (p50 actually
    /// improved ~19%). Call this on a SAMPLED fraction of turns until the
    /// deferred delivery write lands upstream — this repo already has a
    /// latency incident from slow pre-stream recall on the voice path, and
    /// that surface must not regress.
    pub async fn turn(
        &self,
        query: &str,
        visibility: spectral::Visibility,
        context: spectral::graph::RecognitionContext,
    ) -> anyhow::Result<spectral::TurnResult> {
        let brain = self.inner.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || {
            let request = spectral::TurnRequest::query(&query, visibility).with_context(context);
            brain.turn(&request)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: turn: {e}"))?
        .map_err(anyhow::Error::from)
    }

    /// Report which delivered memories were actually used, closing the loop a
    /// [`turn`](Self::turn) opened.
    ///
    /// A turn that is never reported leaves memory state completely unchanged
    /// and produces NO learning signal — the retrieval was then pure overhead.
    /// Treat this as mandatory, not optional.
    pub async fn record_turn_outcome(
        &self,
        receipt: spectral::TurnReceipt,
        outcomes: Vec<(String, spectral::MemoryOutcome)>,
    ) -> anyhow::Result<spectral::OutcomeReceipt> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let borrowed: Vec<(&str, spectral::MemoryOutcome)> = outcomes
                .iter()
                .map(|(key, outcome)| (key.as_str(), *outcome))
                .collect();
            brain.record_turn_outcome(&receipt, &borrowed)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: record_turn_outcome: {e}"))?
        .map_err(anyhow::Error::from)
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
        let result = tokio::task::spawn_blocking({
            let key = key.clone();
            let content = content.clone();
            move || brain.remember_with(&key, &content, opts)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: remember_with: {e}"))?
        .map_err(anyhow::Error::from)?;

        // Real-time brain growth (#24): every genuinely new memory is the
        // single choke point for the "feel the brain grow" contract — emit the
        // growth event and link the memory to the entities it mentions.
        // Best-effort: a linking failure must never fail the write.
        if matches!(
            result.write_outcome,
            spectral::ingest::WriteOutcome::Inserted
        ) {
            crate::events::emit(crate::events::memory_added(
                &result.memory_id,
                &key,
                "core",
                result.wing.as_deref(),
            ));
            let memory_id = result.memory_id.clone();
            tokio::task::spawn_blocking(move || {
                let brain_dir = crate::config::paths::Paths::brain_dir();
                match crate::brain_enrichment::link_new_memory(
                    &brain_dir.join("graph.sqlite"),
                    &brain_dir.join("memory.db"),
                    &memory_id,
                    &content,
                ) {
                    Ok(linked) => {
                        for l in &linked {
                            let event = if l.first_mention {
                                crate::events::entity_added(&l.hex, &l.entity_type)
                            } else {
                                crate::events::entity_updated(&l.hex, &l.entity_type)
                            };
                            crate::events::emit(event);
                        }
                        if !linked.is_empty() {
                            tracing::debug!(
                                memory_id,
                                entities = linked.len(),
                                "memory linked to graph entities"
                            );
                        }
                    }
                    Err(e) => tracing::debug!("memory mention linking skipped: {e}"),
                }
            });
        }
        Ok(result)
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

    /// Hard-delete a memory by its logical **key** across every substrate
    /// (memories + all FK children + recognition sidecar), returning a
    /// verified `ForgetReport`.
    ///
    /// Unlike `consolidate_into` — which is a *soft* filter that hides a memory
    /// from recall while the row persists — `forget` makes the content
    /// unrecoverable (#339). `report.store.existed == false` means no memory
    /// was found for the key (a no-op, not an error). Used by the Reader to
    /// retire a stale prior-version document on re-ingest.
    pub async fn forget(&self, key: &str) -> anyhow::Result<spectral::graph::brain::ForgetReport> {
        let brain = self.inner.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || brain.forget(&key))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: forget: {e}"))?
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
        self.create_typed_entity("person", display_name, visibility)
            .await
    }

    /// Create (or idempotently return) a **project** node in the graph,
    /// returning its bare 64-hex `EntityId` — the `projects.graph_entity_id`
    /// bridge key (#595).
    ///
    /// Projects are the second entity type to need runtime creation, widening
    /// PEOPLE_GRAPH_V1 Decision C's person-only-create rule exactly as that
    /// ruling anticipated ("generalize when a second entity type needs runtime
    /// creation"). The surface stays narrow and validated — project-only
    /// `entity_type`, canonicalized name, empty-after-normalization rejected —
    /// no arbitrary node injection.
    ///
    /// Provenance must be written *before* this call (done by
    /// [`crate::project_graph::ensure_project_graph_identity`]), so a runtime
    /// project node can never exist without reconciler-protecting provenance.
    pub async fn create_project_entity(
        &self,
        project_name: &str,
        visibility: spectral::Visibility,
    ) -> anyhow::Result<String> {
        self.create_typed_entity("project", project_name, visibility)
            .await
    }

    /// Shared narrow-create body for the two sanctioned runtime entity types
    /// (person, #583; project, #595). Private — callers go through the typed
    /// wrappers, which are the whole validated public surface.
    async fn create_typed_entity(
        &self,
        entity_type: &'static str,
        display_name: &str,
        visibility: spectral::Visibility,
    ) -> anyhow::Result<String> {
        let canonical = crate::identity::canonical::graph_canonical(display_name);
        if canonical.is_empty() {
            anyhow::bail!("cannot create {entity_type}: name is empty after normalization");
        }
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            use spectral::core::entity_id::entity_id;
            use spectral::graph::graph_store::Entity;

            let id = entity_id(entity_type, &canonical);
            let store = brain.store();
            // Idempotent: preserve an existing node (and its created_at).
            if let Ok(Some(existing)) = store.get_entity(&id) {
                return Ok(hex::encode(existing.id.as_bytes()));
            }
            let now = chrono::Utc::now();
            let entity = Entity {
                id,
                entity_type: entity_type.to_string(),
                canonical,
                visibility,
                created_at: now,
                updated_at: now,
                weight: 1.0,
                description: None,
            };
            store
                .upsert_entity(&entity)
                .map_err(|e| anyhow::anyhow!("upsert {entity_type} entity: {e}"))?;
            Ok(hex::encode(entity.id.as_bytes()))
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: create_typed_entity: {e}"))?
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

    /// Resolve a project name against the curated ontology (alias + case
    /// aware), returning the bare 64-hex `EntityId` — WITHOUT writing anything.
    /// `None` when the name does not resolve to an ontology `project` entity.
    /// The pure-read twin of [`Self::materialize_ontology_project`]; used on
    /// delete paths (#595 disassociate cleanup) that must not create nodes.
    pub async fn resolve_ontology_project_id(
        &self,
        project_name: &str,
    ) -> anyhow::Result<Option<String>> {
        let brain = self.inner.clone();
        let project_name = project_name.to_string();
        tokio::task::spawn_blocking(move || {
            use spectral::graph::canonicalize::Canonicalizer;
            Ok(
                match Canonicalizer::new(brain.ontology()).resolve_one(&project_name) {
                    Some(m) if m.entity_type == "project" => {
                        Some(hex::encode(m.entity_id.as_bytes()))
                    }
                    _ => None,
                },
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: resolve_ontology_project_id: {e}"))?
    }

    /// Materialize EVERY curated ontology entity as a graph node, skipping any
    /// that already exists. Returns the number of nodes minted.
    ///
    /// Why this exists: the ontology has never been eager-seeded (#495 slice 3)
    /// — nodes were materialized lazily, one at a time, as something referenced
    /// them. That was survivable while the graph store was long-lived, but when
    /// the store moved from Kuzu (`brain/graph.kz`) to SQLite
    /// (`brain/graph.sqlite`) the new store started EMPTY and nothing ever
    /// backfilled it: the Brain view showed zero entities and zero edges on
    /// installs with years of curated ontology behind them. Lazy materialization
    /// alone can never recover that, because nothing re-references an entity
    /// just because its node went missing.
    ///
    /// Idempotent and additive — it only ever inserts absent nodes, never
    /// updates or deletes — so it is safe to run on every boot; a steady state
    /// mints nothing. Curated entities stay outside the runtime-create /
    /// provenance machinery (the reconciler owns their lifecycle via the
    /// ontology id set), exactly as `materialize_ontology_project` does.
    pub async fn materialize_ontology_entities(&self) -> anyhow::Result<usize> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            use spectral::graph::graph_store::Entity;

            let ontology = brain.ontology();
            let store = brain.store();
            let now = chrono::Utc::now();
            let mut minted = 0usize;

            for entity in &ontology.entities {
                let id = ontology.entity_id_for(entity);
                // Preserve any existing node (and its created_at / weight).
                if store
                    .get_entity(&id)
                    .map_err(|e| anyhow::anyhow!("get_entity({}): {e}", entity.canonical))?
                    .is_some()
                {
                    continue;
                }
                store
                    .upsert_entity(&Entity {
                        id,
                        entity_type: entity.entity_type.clone(),
                        canonical: entity.canonical.clone(),
                        visibility: entity.visibility,
                        created_at: now,
                        updated_at: now,
                        weight: 1.0,
                        description: None,
                    })
                    .map_err(|e| {
                        anyhow::anyhow!("upsert ontology entity {}: {e}", entity.canonical)
                    })?;
                minted += 1;
            }
            Ok(minted)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: materialize_ontology_entities: {e}"))?
    }

    /// List every curated ontology entity that has a materialized graph node,
    /// as `(id_hex, entity_type, canonical, description)`.
    ///
    /// Exists for the Brain view: `neighborhood()` traverses TRIPLES only, so
    /// after the Kuzu→SQLite move (which lost the extraction-era triples) the
    /// graph route saw almost nothing even though the nodes were restored.
    /// The curated set IS real knowledge — the view unions it in.
    pub async fn list_ontology_graph_entities(
        &self,
    ) -> anyhow::Result<Vec<(String, String, String, Option<String>)>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let ontology = brain.ontology();
            let store = brain.store();
            let mut out = Vec::new();
            for entity in &ontology.entities {
                let id = ontology.entity_id_for(entity);
                if let Ok(Some(node)) = store.get_entity(&id) {
                    out.push((
                        hex::encode(node.id.as_bytes()),
                        node.entity_type,
                        node.canonical,
                        node.description,
                    ));
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: list_ontology_graph_entities: {e}"))?
    }

    /// Resolve a project name against the curated ontology and materialize its
    /// graph node if missing (the ontology is not eager-seeded — #495 slice 3).
    /// Returns the bare 64-hex `EntityId`, or `None` when the name is not an
    /// ontology project. A curated entity, so this stays outside the
    /// runtime-create/provenance machinery: the reconciler owns its lifecycle
    /// via the ontology id set.
    pub async fn materialize_ontology_project(
        &self,
        project_name: &str,
    ) -> anyhow::Result<Option<String>> {
        let brain = self.inner.clone();
        let project_name = project_name.to_string();
        tokio::task::spawn_blocking(move || {
            use spectral::graph::canonicalize::Canonicalizer;
            use spectral::graph::graph_store::Entity;

            let project = match Canonicalizer::new(brain.ontology()).resolve_one(&project_name) {
                Some(m) if m.entity_type == "project" => m,
                _ => return Ok(None),
            };
            let store = brain.store();
            if store
                .get_entity(&project.entity_id)
                .map_err(|e| anyhow::anyhow!("get_entity(project): {e}"))?
                .is_none()
            {
                let now = chrono::Utc::now();
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
            Ok(Some(hex::encode(project.entity_id.as_bytes())))
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: materialize_ontology_project: {e}"))?
    }

    /// Best-effort graph edge for a person→project association (#495 slice 3,
    /// reworked by #595): assert `works_on(person, project)` between two
    /// *resolved* graph identities. The association row (permagent.db
    /// `project_people`) is the source of truth and the graph is allowed to lag
    /// behind it (people-in-graph v1, #583) — so an endpoint whose node the
    /// graph has not materialized is a skip (`Ok(false)`), never a failure of
    /// the association itself.
    ///
    /// Identity resolution happens *before* this call: the person id comes from
    /// the `people.graph_entity_id` bridge, the project id from
    /// [`crate::project_graph::ensure_project_graph_identity`] (ontology-resolved
    /// or runtime-minted, #595). This method never creates nodes — both
    /// endpoints must already exist.
    ///
    /// Idempotent: an existing `works_on(person, project)` triple short-circuits
    /// to `Ok(false)`. Returns `Ok(true)` only when a triple was written.
    pub async fn assert_works_on_edge(
        &self,
        person_id_hex: &str,
        project_id_hex: &str,
    ) -> anyhow::Result<bool> {
        let brain = self.inner.clone();
        let person_id_hex = person_id_hex.to_string();
        let project_id_hex = project_id_hex.to_string();
        tokio::task::spawn_blocking(move || {
            use spectral::graph::graph_store::Triple;

            let person_id: spectral::core::entity_id::EntityId = person_id_hex
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid person entity id hex: {e:?}"))?;
            let project_id: spectral::core::entity_id::EntityId = project_id_hex
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid project entity id hex: {e:?}"))?;
            let store = brain.store();
            if store
                .get_entity(&person_id)
                .map_err(|e| anyhow::anyhow!("get_entity(person): {e}"))?
                .is_none()
            {
                return Ok(false); // person node lags; nothing to hang the edge on
            }
            if store
                .get_entity(&project_id)
                .map_err(|e| anyhow::anyhow!("get_entity(project): {e}"))?
                .is_none()
            {
                return Ok(false); // project node lags; identity minting failed upstream
            }

            let existing = store
                .find_triples(Some(&person_id), Some(&project_id), Some("works_on"))
                .map_err(|e| anyhow::anyhow!("find_triples: {e}"))?;
            if !existing.is_empty() {
                return Ok(false);
            }

            brain
                .ontology()
                .validate_triple("works_on", "person", "project")
                .map_err(|e| anyhow::anyhow!("validate works_on: {e}"))?;
            store
                .insert_triple(&Triple {
                    from: person_id,
                    to: project_id,
                    predicate: "works_on".to_string(),
                    confidence: 1.0,
                    source_doc_id: None,
                    source_brain_id: *brain.brain_id(),
                    asserted_at: chrono::Utc::now(),
                    visibility: spectral::Visibility::Private,
                    weight: 1.0,
                })
                .map_err(|e| anyhow::anyhow!("insert works_on triple: {e}"))?;
            Ok(true)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: assert_works_on_edge: {e}"))?
    }

    /// List all graph triples touching a person whose other endpoint is also a
    /// person. Both directions are returned so relationship direction is not
    /// lost (for example `manages`).
    pub async fn person_edges(&self, person_id_hex: &str) -> anyhow::Result<Vec<PersonGraphEdge>> {
        let brain = self.inner.clone();
        let person_id: spectral::core::entity_id::EntityId = person_id_hex
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid person entity id hex: {e:?}"))?;
        tokio::task::spawn_blocking(move || {
            let store = brain.store();
            let mut triples = store.find_triples(Some(&person_id), None, None)?;
            triples.extend(store.find_triples(None, Some(&person_id), None)?);
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for triple in triples {
                let Some(from) = store.get_entity(&triple.from)? else {
                    continue;
                };
                let Some(to) = store.get_entity(&triple.to)? else {
                    continue;
                };
                if from.entity_type != "person" || to.entity_type != "person" {
                    continue;
                }
                let row = PersonGraphEdge {
                    from_id: triple.from.to_string(),
                    to_id: triple.to.to_string(),
                    predicate: triple.predicate,
                };
                if seen.insert((
                    row.from_id.clone(),
                    row.to_id.clone(),
                    row.predicate.clone(),
                )) {
                    out.push(row);
                }
            }
            Ok::<_, anyhow::Error>(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: person_edges: {e}"))?
    }

    /// Idempotently assert a typed person→person relationship. The predicate
    /// must be declared by the active ontology and valid for person endpoints.
    pub async fn upsert_person_edge(
        &self,
        from_hex: &str,
        to_hex: &str,
        predicate: &str,
    ) -> anyhow::Result<bool> {
        let brain = self.inner.clone();
        let from: spectral::core::entity_id::EntityId = from_hex
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid source person entity id: {e:?}"))?;
        let to: spectral::core::entity_id::EntityId = to_hex
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid target person entity id: {e:?}"))?;
        let predicate = predicate.trim().to_string();
        if predicate.is_empty() || from == to {
            anyhow::bail!("relationship predicate must be non-empty and people must differ");
        }
        tokio::task::spawn_blocking(move || {
            use spectral::graph::graph_store::Triple;
            brain
                .ontology()
                .validate_triple(&predicate, "person", "person")
                .map_err(|e| anyhow::anyhow!("invalid person relationship: {e}"))?;
            let store = brain.store();
            for (id, label) in [(from, "source"), (to, "target")] {
                let entity = store
                    .get_entity(&id)?
                    .ok_or_else(|| anyhow::anyhow!("{label} person is not in the graph"))?;
                if entity.entity_type != "person" {
                    anyhow::bail!("{label} entity is not a person");
                }
            }
            if !store
                .find_triples(Some(&from), Some(&to), Some(&predicate))?
                .is_empty()
            {
                return Ok(false);
            }
            store.insert_triple(&Triple {
                from,
                to,
                predicate,
                confidence: 1.0,
                source_doc_id: None,
                source_brain_id: *brain.brain_id(),
                asserted_at: chrono::Utc::now(),
                visibility: spectral::Visibility::Private,
                weight: 1.0,
            })?;
            Ok(true)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: upsert_person_edge: {e}"))?
    }

    /// Write a typed field on a graph entity, with provenance. The
    /// manual-not-clobbered rule is enforced in Spectral's store: an
    /// `Enriched` write never overwrites a field whose stored source is
    /// `Manual`. Returns `false` when the write was suppressed by that rule,
    /// `true` when applied.
    /// Write the freeform description on a graph entity (#387 — the Brain
    /// view's entity cards read it as `note`). Idempotent in Spectral: setting
    /// the same value twice is a no-op.
    pub async fn set_entity_description(
        &self,
        entity_id: spectral::core::entity_id::EntityId,
        description: &str,
    ) -> anyhow::Result<()> {
        let brain = self.inner.clone();
        let description = description.to_string();
        tokio::task::spawn_blocking(move || brain.set_entity_description(&entity_id, &description))
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: set_entity_description: {e}"))?
            .map_err(Into::into)
    }

    /// All entities within the recall neighborhood, with their live description
    /// state — the #387-v2 entity pass's *catch-all* source (locations etc. that
    /// no table or annotation enumerates). Bounded by the same 2-hop
    /// neighborhood the Brain graph shows (Spectral has no all-entities
    /// enumeration on the pinned rev; `KuzuStore` exposes only `get_entity` /
    /// `neighborhood`). Unlike the removed `undescribed_entities`, described
    /// entities are included so the caller can run staleness checks on them.
    pub async fn neighborhood_entity_snapshots(
        &self,
        seed: &str,
        cap: usize,
    ) -> anyhow::Result<Vec<GraphEntitySnapshot>> {
        let result = self.recall(seed, spectral::Visibility::Private).await?;
        Ok(result
            .graph
            .neighborhood
            .entities
            .iter()
            .take(cap)
            .map(|e| GraphEntitySnapshot {
                id: e.id,
                entity_type: e.entity_type.clone(),
                canonical: e.canonical.clone(),
                description: e.description.clone().filter(|d| !d.is_empty()),
            })
            .collect())
    }

    /// Batch-load graph entity snapshots by bare 64-hex `EntityId` (the
    /// `people.graph_entity_id` bridge key). One blocking hop for the whole
    /// batch. `None` per slot for an unparseable hex or a node absent from the
    /// store — the #387-v2 pass must *never* describe an id the graph has not
    /// materialized, because Spectral's `set_entity_description` MERGEs and
    /// would mint a half-formed node (id + description, no type/canonical).
    pub async fn entity_snapshots_by_hex(
        &self,
        id_hexes: Vec<String>,
    ) -> anyhow::Result<Vec<Option<GraphEntitySnapshot>>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let store = brain.store();
            let mut out = Vec::with_capacity(id_hexes.len());
            for hex_id in &id_hexes {
                let parsed: Result<spectral::core::entity_id::EntityId, _> = hex_id.parse();
                let snap = match parsed {
                    Ok(id) => match store.get_entity(&id) {
                        Ok(Some(e)) => Some(GraphEntitySnapshot {
                            id: e.id,
                            entity_type: e.entity_type,
                            canonical: e.canonical,
                            description: e.description.filter(|d| !d.is_empty()),
                        }),
                        Ok(None) => None,
                        Err(e) => return Err(anyhow::anyhow!("get_entity: {e}")),
                    },
                    Err(_) => None,
                };
                out.push(snap);
            }
            Ok(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: entity_snapshots_by_hex: {e}"))?
    }

    /// Resolve free-text names (annotation terms/categories, project names) to
    /// graph entities via **exact** ontology alias matching — deliberately no
    /// fuzzy fallback: the #387-v2 pass writes *descriptions*, and a fuzzy
    /// mis-bind would put a truthful-sounding card on the wrong entity.
    /// Matching normalizes both sides with
    /// [`crate::identity::canonical::graph_canonical`] (lowercase, collapsed
    /// whitespace — the exact form Spectral hashes into `EntityId`s).
    ///
    /// Per name: [`OntologyEntityResolution::NoIdentity`] when the ontology has
    /// no such entity (a SQLite-shadow-only term — no graph card to describe),
    /// `NotInGraph` when the ontology knows it but the node was never
    /// materialized (ontology is not eager-seeded), `InGraph` with the live
    /// snapshot plus the entity's aliases otherwise.
    pub async fn resolve_ontology_entities_exact(
        &self,
        names: Vec<String>,
    ) -> anyhow::Result<Vec<OntologyEntityResolution>> {
        use crate::identity::canonical::graph_canonical;
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let ontology = brain.ontology();
            // alias (normalized) → ontology entity index; first declaration wins.
            let mut lookup: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (i, e) in ontology.entities.iter().enumerate() {
                for alias in std::iter::once(&e.canonical).chain(e.aliases.iter()) {
                    lookup.entry(graph_canonical(alias)).or_insert(i);
                }
            }
            let store = brain.store();
            let mut out = Vec::with_capacity(names.len());
            for name in &names {
                let normalized = graph_canonical(name);
                let Some(&idx) = lookup.get(&normalized) else {
                    out.push(OntologyEntityResolution::NoIdentity);
                    continue;
                };
                let entity = &ontology.entities[idx];
                let id = ontology.entity_id_for(entity);
                match store.get_entity(&id) {
                    Ok(Some(e)) => out.push(OntologyEntityResolution::InGraph {
                        snapshot: GraphEntitySnapshot {
                            id: e.id,
                            entity_type: e.entity_type,
                            canonical: e.canonical,
                            description: e.description.filter(|d| !d.is_empty()),
                        },
                        aliases: entity.aliases.clone(),
                    }),
                    Ok(None) => out.push(OntologyEntityResolution::NotInGraph),
                    Err(e) => return Err(anyhow::anyhow!("get_entity: {e}")),
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: resolve_ontology_entities_exact: {e}"))?
    }

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

    /// Atomically write a batch of manual entity fields in deterministic name
    /// order. The direct transaction is necessary because Spectral currently
    /// exposes only a single-field API; independent calls can partially commit.
    pub async fn set_manual_entity_fields(
        &self,
        entity_id: spectral::core::entity_id::EntityId,
        mut fields: Vec<(String, String)>,
    ) -> anyhow::Result<()> {
        fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
        tokio::task::spawn_blocking(move || {
            write_manual_entity_fields(&db_path, &entity_id.to_string(), fields)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: set_manual_entity_fields: {e}"))?
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

    // ── Layered-store consolidation atoms (Librarian write-side) ─────
    // The Brain surfaces recurring clusters deterministically; the Librarian
    // abstracts each into one durable, strong-model atom stored via
    // `consolidate_as` (provenance-linked, so the atom is an additive hint the
    // actor verifies against raw sources — never an authoritative replacement).

    /// Deterministic recurring-cluster candidates worth abstracting into a
    /// single higher-tier atom (co-retrieval + recognition-recurrence gated;
    /// `member_keys` always ≥ 2). `$0`, no LLM. Mirrors
    /// [`spectral::Brain::consolidation_candidates`].
    pub async fn consolidation_candidates(
        &self,
        min_co_count: u64,
        scan_limit: usize,
    ) -> anyhow::Result<Vec<spectral::graph::brain::ConsolidationCandidate>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            brain.consolidation_candidates(min_co_count, scan_limit)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: consolidation_candidates: {e}"))?
        .map_err(Into::into)
    }

    /// Store a **pre-computed** Librarian atom over `source_keys` at
    /// `target_key`: writes a higher-`tier` memory and links the sources via
    /// `consolidation_edges` (reachable through
    /// [`recall_with_provenance`](Self::recall_with_provenance)). Mirrors
    /// [`spectral::Brain::consolidate_as`].
    pub async fn consolidate_as(
        &self,
        source_keys: Vec<String>,
        target_key: String,
        tier: spectral::ingest::CompactionTier,
        content: String,
    ) -> anyhow::Result<spectral::RememberResult> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            brain.consolidate_as(&source_keys, &target_key, tier, &content)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: consolidate_as: {e}"))?
        .map_err(Into::into)
    }

    /// Deterministic `$0` extractive consolidation (longest source) — the
    /// no-LLM fallback so layered recall still exists when no strong-model
    /// provider resolves. Mirrors [`spectral::Brain::consolidate_extractive`].
    pub async fn consolidate_extractive(
        &self,
        source_keys: Vec<String>,
        target_key: String,
        tier: spectral::ingest::CompactionTier,
    ) -> anyhow::Result<spectral::RememberResult> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            brain.consolidate_extractive(&source_keys, &target_key, tier)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: consolidate_extractive: {e}"))?
        .map_err(Into::into)
    }

    /// Layered / provenance-linked recall: each hit paired with its
    /// ground-truth source memories (drill-down through `consolidation_edges`),
    /// so the actor gets the compact atom **plus** the exact raw turns it
    /// distilled and can verify a count against them. Builds a default
    /// `RecallTopKConfig` internally, matching the other recall wrappers.
    /// Mirrors [`spectral::Brain::recall_with_provenance`].
    pub async fn recall_with_provenance(
        &self,
        query: String,
        visibility: spectral::Visibility,
        max_sources: usize,
    ) -> anyhow::Result<Vec<spectral::graph::brain::LayeredHit>> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let config = spectral::RecallTopKConfig::default();
            brain.recall_with_provenance(&query, &config, visibility, max_sources)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: recall_with_provenance: {e}"))?
        .map_err(Into::into)
    }

    /// Write a memory at an explicitly chosen [`MemoryScope`] (visibility level).
    ///
    /// This is the settable entry point for Spectral's shipped visibility ladder
    /// (`Private < Team < Org < Public`). It is thin sugar over
    /// [`remember_with`](Self::remember_with): it stamps `opts.visibility` from
    /// `scope` (overriding whatever the passed `opts` carried) and forwards the
    /// rest of `opts` unchanged — so callers can still set `source`, `wing`,
    /// `confidence`, etc. alongside the level.
    ///
    /// **Default is unchanged.** Existing write paths that never call this keep
    /// their `Visibility::Private` writes. Nothing here flips a global default;
    /// it only makes a non-`Private` level *expressible* at the call site.
    ///
    /// The persisted `memories.visibility` string is what the read filter keys
    /// off: a memory written `Team` is hidden from a `Private`-clearance recall
    /// context's *floor* — see [`MemoryScope`] for the exact `allows` semantics.
    pub async fn remember_scoped(
        &self,
        key: &str,
        content: &str,
        scope: MemoryScope,
        opts: spectral::RememberOpts,
    ) -> anyhow::Result<spectral::RememberResult> {
        let opts = spectral::RememberOpts {
            visibility: scope.to_visibility(),
            ..opts
        };
        self.remember_with(key, content, opts).await
    }

    /// Hard-delete an explicit set of memory keys, aggregating each verified
    /// [`ForgetReport`](spectral::graph::brain::ForgetReport) into a
    /// [`ScopeForgetReport`]. The reusable core of the scope sweep.
    ///
    /// Each key is passed to [`forget`](Self::forget), which removes the memory
    /// across every SQLite substrate (row + FTS shadow, fingerprints,
    /// spectrogram, annotations, consolidation edges, co-retrieval pairs,
    /// retrieval-event refs, recognition sidecar) and re-probes recall +
    /// recognition to verify it is gone. A key with no memory is a no-op
    /// (`store.existed == false`), counted but not an error.
    ///
    /// The whole loop runs in a single `spawn_blocking` so N deletes cost one
    /// task dispatch rather than N.
    ///
    /// **Graph triples are not touched** — see [`ScopeForgetReport`] and the
    /// Q2 residual (`graph_triples_deleted` is always 0 at pin `fb1038db`).
    pub async fn forget_keys(&self, keys: Vec<String>) -> anyhow::Result<ScopeForgetReport> {
        let brain = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut agg = ScopeForgetReport::default();
            for key in &keys {
                let report = brain
                    .forget(key)
                    .map_err(|e| anyhow::anyhow!("forget({key}) failed: {e}"))?;
                agg.absorb(key, &report);
            }
            Ok::<_, anyhow::Error>(agg)
        })
        .await
        .map_err(|e| anyhow::anyhow!("brain task panicked: forget_keys: {e}"))?
    }

    /// Hard-delete **every memory in a wing (scope)** — the scope-based forget
    /// primitive behind the offboarding "clean divorce" claim (design doc
    /// §3.2, Step 4).
    ///
    /// This is a bounded *enumerate → forget → re-enumerate* sweep, not a single
    /// Spectral primitive: it reads the local brain `memory.db`, selects every
    /// `key` whose `wing` column equals `wing`, then hard-deletes each via
    /// [`forget_keys`](Self::forget_keys) and returns the aggregate
    /// [`ScopeForgetReport`] (the audit substrate). Any keys still present after
    /// the pass bound are returned in `residual_keys`. Memories in *other* wings —
    /// and wingless memories (`wing IS NULL`, e.g. chat turns) — are untouched.
    ///
    /// The `wing` here is the **cognitive/topical wing** carried on each memory
    /// (`RememberOpts.wing`), which is what Permagent can enumerate today. It is
    /// *not* the federation `wing_id` of the design doc's Axis A (shared-wing
    /// membership): that layer (`federation_sync::enumerate`) is not reachable
    /// from Permagent at pin `fb1038db` — the outer `spectral::Brain` exposes no
    /// `enumerate`/`share`, and the `&SqliteStore` accessor is crate-private.
    /// Binding this sweep to the federation offboarding boundary is a
    /// Spectral-gated + decision-gated follow-up (design doc Q1/Q3). See
    /// `docs/design/sovereign-offboarding-phase1-notes.md`.
    ///
    /// **Graph-triple residual:** company-derived graph facts survive this sweep
    /// (Q2). `report.graph_triples_deleted == 0` at this pin. Flagged, not
    /// hidden.
    pub async fn forget_scope(&self, wing: &str) -> anyhow::Result<ScopeForgetReport> {
        const MAX_PASSES: usize = 8;

        async fn enumerate(wing: String) -> anyhow::Result<Vec<String>> {
            tokio::task::spawn_blocking(move || {
                let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
                let conn = rusqlite::Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| anyhow::anyhow!("scope enumerate: open {}: {e}", db_path.display()))?;
                let mut stmt = conn
                    .prepare("SELECT key FROM memories WHERE wing = ?1 ORDER BY key")
                    .map_err(|e| anyhow::anyhow!("scope enumerate: prepare: {e}"))?;
                let rows = stmt
                    .query_map(rusqlite::params![wing], |r| r.get::<_, String>(0))
                    .map_err(|e| anyhow::anyhow!("scope enumerate: query: {e}"))?;
                let mut keys = Vec::new();
                for r in rows {
                    keys.push(r.map_err(|e| anyhow::anyhow!("scope enumerate: row: {e}"))?);
                }
                Ok::<_, anyhow::Error>(keys)
            })
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: forget_scope enumerate: {e}"))?
        }

        let mut report = ScopeForgetReport::default();
        for _ in 0..MAX_PASSES {
            let keys = enumerate(wing.to_string()).await?;
            if keys.is_empty() {
                break;
            }
            let pass = self.forget_keys(keys).await?;
            report.keys_swept += pass.keys_swept;
            report.existed += pass.existed;
            report.fully_forgotten += pass.fully_forgotten;
            report.forgotten_keys.extend(pass.forgotten_keys);
        }
        // Always re-query after the final deletion pass. If writers keep the
        // scope non-empty past the bound, the receipt says exactly what remains.
        report.residual_keys = enumerate(wing.to_string()).await?;
        report.wing = Some(wing.to_string());
        Ok(report)
    }
}

impl std::fmt::Debug for SafeBrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafeBrain").finish_non_exhaustive()
    }
}

// ── Associative recall (ACR) toggle ──────────────────────────────────────────
//
// ACR = local, embedding-free associative recall in Spectral's cascade: FTS
// finds seeds, then spreading activation over co-occurrence links reaches
// memories that share no words with the query. It is experimental and its
// accuracy is unvalidated, so it ships OFF by default (a pure no-op) and is
// flipped only for an A/B via the `PERMAGENT_ACR_MODE` env toggle — no recompile.
// The single resolver below is applied at EVERY cascade-construction site so all
// recall paths honor the toggle consistently.

/// Resolve the ACR spreading config from the `PERMAGENT_ACR_MODE` env toggle.
///
/// | `PERMAGENT_ACR_MODE`             | result                                        |
/// |----------------------------------|-----------------------------------------------|
/// | unset / `off` / empty / unknown  | `AssocSpreadConfig::default()` — `SpreadMode::Off` (no-op) |
/// | `precision`                      | `AssocSpreadConfig::precision()` — `Rerank`, session-safe, ~constant context |
/// | `completeness`                   | `AssocSpreadConfig::completeness()` — `Combined` |
///
/// OFF by default: with the toggle unset the cascade behaves exactly as before.
pub(crate) fn acr_spread_config() -> spectral::graph::spreading::AssocSpreadConfig {
    resolve_acr_spread(std::env::var("PERMAGENT_ACR_MODE").ok().as_deref())
}

/// The recognized shapes of `PERMAGENT_ACR_MODE`. `Unrecognized` carries the
/// offending (trimmed, lowercased) value so the resolver can name it in a warning.
#[derive(Debug, PartialEq, Eq)]
enum AcrMode {
    Off,
    Precision,
    Completeness,
    Unrecognized(String),
}

/// Classify a raw `PERMAGENT_ACR_MODE` value (trimmed, case-insensitive). Pure and
/// unit-testable without touching process-global env. Unset, empty/whitespace, and
/// `off` are [`AcrMode::Off`]; a non-empty value that matches none of the accepted
/// keywords is [`AcrMode::Unrecognized`] — a likely typo in an A/B arm.
fn classify_acr_mode(raw: Option<&str>) -> AcrMode {
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        None => AcrMode::Off,
        Some(v) => match v.as_str() {
            "" | "off" => AcrMode::Off,
            "precision" => AcrMode::Precision,
            "completeness" => AcrMode::Completeness,
            _ => AcrMode::Unrecognized(v),
        },
    }
}

/// Pure env-value → [`spectral::graph::spreading::AssocSpreadConfig`] mapping.
///
/// Split out from [`acr_spread_config`] so resolution is unit-testable without
/// mutating process-global env (which would race under parallel `cargo test`).
/// Unrecognized/empty values fall through to `Off` (fail-safe) rather than
/// panicking, so a typo in the A/B toggle can never silently alter retrieval — but
/// an unrecognized NON-empty value now emits a `warn!` naming the accepted values,
/// so a misspelled A/B arm can't silently masquerade as "ACR has no effect".
fn resolve_acr_spread(raw: Option<&str>) -> spectral::graph::spreading::AssocSpreadConfig {
    use spectral::graph::spreading::AssocSpreadConfig;
    match classify_acr_mode(raw) {
        AcrMode::Precision => AssocSpreadConfig::precision(),
        AcrMode::Completeness => AssocSpreadConfig::completeness(),
        AcrMode::Off => AssocSpreadConfig::default(),
        AcrMode::Unrecognized(value) => {
            tracing::warn!(
                target: "permagentd::brain",
                "PERMAGENT_ACR_MODE='{}' is not a recognized value — expected one of \
                 off / precision / completeness; associative recall stays OFF (no effect)",
                value
            );
            AssocSpreadConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::graph::spreading::SpreadMode;

    #[test]
    fn manual_entity_field_batch_rolls_back_on_later_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = temp.path().join("memory.db");
        let conn = rusqlite::Connection::open(&db).expect("open");
        conn.execute_batch(
            "CREATE TABLE entity_fields (
                entity_id TEXT NOT NULL, field_name TEXT NOT NULL,
                value TEXT NOT NULL, source TEXT NOT NULL, source_url TEXT,
                updated_at TEXT NOT NULL, PRIMARY KEY (entity_id, field_name)
             );
             CREATE TRIGGER reject_role BEFORE INSERT ON entity_fields
             WHEN NEW.field_name = 'role' BEGIN SELECT RAISE(FAIL, 'injected'); END;",
        )
        .expect("schema");
        drop(conn);

        let result = write_manual_entity_fields(
            &db,
            "entity",
            vec![
                ("company".into(), "Acme".into()),
                ("role".into(), "Engineer".into()),
            ],
        );
        assert!(result.is_err(), "the injected second write must fail");
        let conn = rusqlite::Connection::open(&db).expect("reopen");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entity_fields", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0, "the earlier field must roll back with the batch");
    }

    /// Verify that Clone shares the same underlying Arc (cheap clone).
    #[test]
    fn clone_shares_arc() {
        // We can't easily construct a real Brain in unit tests without a data dir,
        // but we can verify the Clone impl at the type level by confirming SafeBrain
        // is Clone. The actual sharing is guaranteed by Arc semantics.
        fn assert_clone<T: Clone>() {}
        assert_clone::<SafeBrain>();
    }

    // ── Associative recall (ACR) env-toggle resolution ───────────────────────
    // Exercises the pure resolver with string inputs (no process-env mutation,
    // so no #[serial] / no races). Covers the off-by-default no-op contract and
    // both opt-in presets.

    #[test]
    fn acr_unset_off_empty_and_garbage_resolve_to_off() {
        for raw in [
            None,
            Some("off"),
            Some(""),
            Some("   "),
            Some("nonsense"),
            Some("OFF"),
        ] {
            assert_eq!(
                resolve_acr_spread(raw).mode,
                SpreadMode::Off,
                "raw {raw:?} must resolve to Off (fail-safe)"
            );
        }
    }

    /// The warn-path classifier: an unrecognized NON-empty value (a typo'd A/B
    /// arm) is flagged as `Unrecognized` — which drives the `warn!` — while unset,
    /// empty/whitespace, explicit `off`, and the two presets are NOT flagged.
    /// Unrecognized still resolves to the Off config (fail-safe behavior kept).
    #[test]
    fn acr_classify_flags_unrecognized_but_not_off_or_presets() {
        // Flagged: likely typos, carrying the trimmed+lowercased offending value.
        assert_eq!(
            classify_acr_mode(Some("presicion")),
            AcrMode::Unrecognized("presicion".to_string())
        );
        assert_eq!(
            classify_acr_mode(Some("  GARBAGE ")),
            AcrMode::Unrecognized("garbage".to_string())
        );
        // NOT flagged: the fail-safe / valid inputs.
        assert_eq!(classify_acr_mode(None), AcrMode::Off);
        assert_eq!(classify_acr_mode(Some("")), AcrMode::Off);
        assert_eq!(classify_acr_mode(Some("   ")), AcrMode::Off);
        assert_eq!(classify_acr_mode(Some("OFF")), AcrMode::Off);
        assert_eq!(classify_acr_mode(Some("precision")), AcrMode::Precision);
        assert_eq!(
            classify_acr_mode(Some("completeness")),
            AcrMode::Completeness
        );
        // An unrecognized value still resolves to Off (behavior unchanged).
        assert_eq!(resolve_acr_spread(Some("presicion")).mode, SpreadMode::Off);
    }

    #[test]
    fn acr_precision_resolves_to_rerank_preset() {
        assert_eq!(
            resolve_acr_spread(Some("precision")).mode,
            SpreadMode::Rerank
        );
        // whitespace/case tolerant, still fail-safe
        assert_eq!(
            resolve_acr_spread(Some("  Precision ")).mode,
            SpreadMode::Rerank
        );
    }

    #[test]
    fn acr_completeness_resolves_to_combined_preset() {
        assert_eq!(
            resolve_acr_spread(Some("completeness")).mode,
            SpreadMode::Combined
        );
        assert_eq!(
            resolve_acr_spread(Some("COMPLETENESS")).mode,
            SpreadMode::Combined
        );
    }

    #[test]
    fn default_pipeline_config_carries_off_spread() {
        // The cascade config built with the toggle unset carries Off — the
        // wiring is a pure no-op by default (identical to pre-ACR behavior).
        let cfg = spectral::graph::cascade_layers::CascadePipelineConfig {
            spread: resolve_acr_spread(None),
            ..Default::default()
        };
        assert_eq!(cfg.spread.mode, SpreadMode::Off);
        // Guard against an upstream default flip silently enabling ACR under us.
        assert_eq!(
            spectral::graph::cascade_layers::CascadePipelineConfig::default()
                .spread
                .mode,
            SpreadMode::Off
        );
    }
}
