use crate::state::AppState;
use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};
use spectral::core::entity_id::EntityId;
use sqlx::Row;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct BrainSearchQuery {
    pub q: String,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// "memory", "chat", or "both" (default).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Lane R61a: the working directory of the Harness session asking. When
    /// any `scope_*` parameter is present the search is SCOPED — see
    /// [`SearchScope`]. Resolved here, against the daemon's own project
    /// registry, to the project whose `root_path` contains it.
    #[serde(default)]
    pub scope_cwd: Option<String>,
    /// The main checkout of the repo `scope_cwd` is in, when it is a linked git
    /// worktree (a worktree's own directory is under no project root).
    #[serde(default)]
    pub scope_repo_root: Option<String>,
    /// Comma-separated wing slugs of other projects the USER approved for
    /// this session. Never chosen by a model: the CLI only sends slugs its
    /// terminal approval prompt granted.
    #[serde(default)]
    pub scope_projects: Option<String>,
    /// The user approved "all memories" for this session.
    #[serde(default)]
    pub scope_all: Option<bool>,
}

/// Which memories a search may return (Lane R61a).
///
/// The user's ruling (2026-09-16): the harness only ever sees memories about
/// the project repo it is actively in, unless the user approves more at the
/// terminal. The CLI enforces who may widen; this is where the rows are
/// actually cut, so a scoped caller cannot receive an out-of-scope row even if
/// its own filtering were wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchScope {
    /// No `scope_*` parameter: the Brain view's whole-brain search, unchanged.
    Unscoped,
    /// An explicit "all memories" grant. The same rows as `Unscoped`, but the
    /// caller had to say so.
    All,
    /// Only Spectral memories whose wing is in this set. Chat transcript rows
    /// carry no project attribution and are excluded. An EMPTY set returns
    /// nothing: a directory no project claims gets no memories, never all of
    /// them (fail closed).
    Wings(std::collections::BTreeSet<String>),
}

impl SearchScope {
    /// Build the scope from the request and the wing its working directory
    /// resolved to (`None` when no project owns it or nothing was sent).
    pub fn from_query(params: &BrainSearchQuery, cwd_wing: Option<String>) -> Self {
        let scoped = params.scope_cwd.is_some()
            || params.scope_repo_root.is_some()
            || params.scope_projects.is_some()
            || params.scope_all.is_some();
        if !scoped {
            return Self::Unscoped;
        }
        if params.scope_all == Some(true) {
            return Self::All;
        }
        let mut wings: std::collections::BTreeSet<String> = cwd_wing.into_iter().collect();
        wings.extend(
            params
                .scope_projects
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|slug| !slug.is_empty())
                .map(str::to_string),
        );
        Self::Wings(wings)
    }

    /// Whether chat-transcript (FTS) rows may be returned at all.
    fn allows_chat(&self) -> bool {
        !matches!(self, Self::Wings(_))
    }

    /// Whether a memory with this wing is inside the scope. Unattributed
    /// memories (`None`) are inside only an unscoped or all-memories search.
    pub fn admits(&self, wing: Option<&str>) -> bool {
        match self {
            Self::Unscoped | Self::All => true,
            Self::Wings(wings) => wing.is_some_and(|wing| wings.contains(wing)),
        }
    }
}

/// Resolve the request's scope, reading the project registry only when a
/// working directory was sent.
async fn resolve_search_scope(state: &AppState, params: &BrainSearchQuery) -> SearchScope {
    let paths: Vec<&str> = [
        params.scope_cwd.as_deref(),
        params.scope_repo_root.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let cwd_wing = if paths.is_empty() {
        None
    } else {
        match state.session_manager().pool_clone().await {
            Ok(pool) => {
                permagent::session_wing::resolve_project_wing_for_paths(&pool, &paths).await
            }
            Err(error) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    %error,
                    "scoped brain search could not open the project registry; no project memories"
                );
                None
            }
        }
    };
    let scope = SearchScope::from_query(params, cwd_wing);
    tracing::debug!(target: "permagentd::brain", ?scope, "brain search scope");
    scope
}

/// Cut recall hits down to the scope, BEFORE layering, so out-of-scope rows
/// never spend the search budget either.
fn retain_in_scope(
    hits: Vec<spectral::ingest::MemoryHit>,
    scope: &SearchScope,
) -> Vec<spectral::ingest::MemoryHit> {
    hits.into_iter()
        .filter(|hit| scope.admits(hit.wing.as_deref()))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct BrainSearchResult {
    /// "memory" or "chat"
    pub source: String,
    pub id: String,
    pub preview: String,
    pub score: f64,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Progressive load: abstract | overview | full. Omitted on chat FTS hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Quiet “why this?” — lexical score + layer. Never a vector distance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Stable memory key — used to deepen Abstract/Overview to Full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BrainSearchResponse {
    pub results: Vec<BrainSearchResult>,
    pub total: usize,
    pub query: String,
    pub offset: usize,
    pub limit: usize,
    pub fts_count: usize,
    pub spectral_count: usize,
    pub dedup_count: usize,
}

/// Query chat history FTS via SessionManager.
/// FTS has no relevance scoring — all keyword matches get a constant 0.7.
async fn query_fts(
    state: &AppState,
    query: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> anyhow::Result<Vec<BrainSearchResult>> {
    let fts = state
        .session_manager()
        .search_chat_history(
            query,
            Some(100), // fetch up to 100 sessions for brain search
            since,
            until,
            None, // no session exclusion
            vec![SessionType::User, SessionType::Scheduled],
        )
        .await?;

    let mut results = Vec::new();
    for session_result in fts.results {
        for msg in session_result.messages {
            let preview = truncate_preview(&msg.content, 200);
            results.push(BrainSearchResult {
                source: "chat".to_string(),
                id: format!(
                    "{}:{}",
                    session_result.session_id,
                    msg.timestamp.timestamp()
                ),
                preview,
                score: 0.7,
                timestamp: msg.timestamp,
                session_id: Some(session_result.session_id.clone()),
                layer: None,
                why: None,
                key: None,
            });
        }
    }
    Ok(results)
}

/// Query Spectral Brain recall (semantic fingerprint similarity).
/// Uses signal_score directly (already ~0.0-1.0).
async fn query_spectral(
    state: &AppState,
    query: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    scope: &SearchScope,
) -> anyhow::Result<Vec<BrainSearchResult>> {
    if matches!(scope, SearchScope::Wings(wings) if wings.is_empty()) {
        return Ok(Vec::new());
    }
    let brain = match state.brain.as_ref() {
        Some(b) => b,
        None => return Ok(Vec::new()),
    };

    let mut recall_result = brain.recall(query, spectral::Visibility::Private).await?;
    // Spectral's wing match is a ranking boost, not a filter, so the cut is
    // applied to what recall returned. Known cost: a scoped search can come
    // back shorter than an unscoped one when other projects outrank this one.
    recall_result.memory_hits = retain_in_scope(recall_result.memory_hits, scope);
    let ts = Utc::now();
    if let Some(ref s) = since {
        if ts < *s {
            return Ok(Vec::new());
        }
    }
    if let Some(ref u) = until {
        if ts > *u {
            return Ok(Vec::new());
        }
    }

    let sources: Vec<permagent::context_layers::AssembleSource<'_>> = recall_result
        .memory_hits
        .iter()
        .map(|hit| permagent::context_layers::AssembleSource {
            key: hit.key.as_str(),
            abstract_text: hit.description.as_deref(),
            content: hit.content.as_str(),
            score: hit.signal_score,
        })
        .collect();
    let layered = permagent::context_layers::assemble(
        &sources,
        permagent::context_layers::AssembleBudget::SEARCH,
    );

    let results = layered
        .into_iter()
        .enumerate()
        .map(|(i, hit)| BrainSearchResult {
            source: "memory".to_string(),
            id: format!("spectral:{}", i),
            preview: truncate_preview(&hit.text, 200),
            score: hit.score,
            timestamp: ts,
            session_id: None,
            layer: Some(hit.layer.as_str().to_string()),
            why: Some(hit.why),
            key: Some(hit.key),
        })
        .collect();
    Ok(results)
}

/// Merge results from both backends, sort by score, deduplicate.
/// v1.0: no cross-backend dedup. FTS returns chat messages, Spectral returns
/// distilled memories — different granularities that don't naturally overlap.
fn merge_and_rank(
    fts: Vec<BrainSearchResult>,
    spectral: Vec<BrainSearchResult>,
) -> (Vec<BrainSearchResult>, usize) {
    let mut combined: Vec<BrainSearchResult> = fts.into_iter().chain(spectral).collect();
    combined.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (combined, 0)
}

fn truncate_preview(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", s.get(..end).unwrap_or(s))
    }
}

async fn brain_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BrainSearchQuery>,
) -> Result<Json<BrainSearchResponse>, crate::routes::errors::ErrorResponse> {
    let limit = params.limit.unwrap_or(50).min(200);
    let source_filter = params.source.as_deref().unwrap_or("both");
    let scope = resolve_search_scope(&state, &params).await;
    // A scoped search never returns chat transcript rows: they carry no
    // project attribution, so none of them is provably this repo's.
    let want_fts = source_filter != "memory" && scope.allows_chat();
    let want_spectral = source_filter != "chat";

    let (fts_results, spectral_results) = tokio::try_join!(
        async {
            if want_fts {
                query_fts(&state, &params.q, params.since, params.until).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if want_spectral {
                query_spectral(&state, &params.q, params.since, params.until, &scope).await
            } else {
                Ok(Vec::new())
            }
        },
    )
    .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    let fts_count = fts_results.len();
    let spectral_count = spectral_results.len();

    let (merged, dedup_count) = merge_and_rank(fts_results, spectral_results);
    let total = merged.len();

    let paginated: Vec<BrainSearchResult> =
        merged.into_iter().skip(params.offset).take(limit).collect();

    Ok(Json(BrainSearchResponse {
        results: paginated,
        total,
        query: params.q,
        offset: params.offset,
        limit,
        fts_count,
        spectral_count,
        dedup_count,
    }))
}

// ── GET /api/brain/graph — force-directed graph data for Brain View ──

#[derive(Debug, Deserialize)]
struct GraphQuery {
    /// Optional search query. When provided, recall is scoped to this term.
    /// When absent, a multi-seed strategy provides diverse coverage.
    #[serde(default)]
    q: Option<String>,
}

#[derive(Debug, Serialize)]
struct GraphSelf {
    name: String,
    id: String,
}

#[derive(Debug, Serialize)]
struct GraphEntity {
    id: String,
    #[serde(rename = "type")]
    entity_type: String,
    name: String,
    note: String,
    /// Typed fields with provenance (from Spectral's entity_fields store).
    fields: Vec<GraphEntityField>,
}

#[derive(Debug, Serialize)]
struct GraphEntityField {
    field_name: String,
    value: String,
    /// Provenance: "manual" | "enriched".
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_url: Option<String>,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct GraphMemory {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    text: String,
    description: Option<String>,
    ent: Vec<String>,
    age: f64,
    weight: f64,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<String>,
    /// Where this memory came from — the `RememberOpts.source` tag
    /// (`"chat"`, `"permagent.card"`, `"permagent.goal"`, …; see
    /// `permagent::brain_feed::FeedSource::as_str`). `None` for a memory
    /// written before the source column existed — the UI shows "source
    /// unknown" rather than guessing (R5 "a memory shows its source").
    #[serde(default)]
    source: Option<String>,
    /// When this memory was last reinforced (Spectral's Memify feedback
    /// loop — the closest durable signal to "last referenced" Spectral
    /// carries; `None` means never reinforced since creation, not unknown).
    #[serde(default)]
    last_reinforced_at: Option<String>,
}

/// An entity→entity edge (graph triple) between two emitted entities —
/// person→project / person→person connection lines (#495 slice 3).
#[derive(Debug, Serialize)]
struct GraphEdge {
    /// Source entity id, `"e:<hex>"` — matches `GraphEntity.id`.
    from: String,
    /// Target entity id, `"e:<hex>"`.
    to: String,
    predicate: String,
}

#[derive(Debug, Serialize)]
struct GraphResponse {
    #[serde(rename = "self")]
    self_node: GraphSelf,
    entities: Vec<GraphEntity>,
    edges: Vec<GraphEdge>,
    memories: Vec<GraphMemory>,
}

/// Map neighborhood triples onto the emitted entity set: keep an edge only
/// when BOTH endpoints were picked for display, dedup on (from, to, predicate).
/// Duplicate triples can legitimately exist in the store (re-asserts append);
/// display needs each connection once.
fn collect_entity_edges(
    triples: &[spectral::graph::graph_store::Triple],
    picked_ids: &std::collections::HashSet<String>,
) -> Vec<GraphEdge> {
    let mut seen = std::collections::HashSet::new();
    let mut edges = Vec::new();
    for t in triples {
        let from = format!("e:{}", hex::encode(t.from.as_bytes()));
        let to = format!("e:{}", hex::encode(t.to.as_bytes()));
        if !picked_ids.contains(&from) || !picked_ids.contains(&to) || from == to {
            continue;
        }
        if seen.insert((from.clone(), to.clone(), t.predicate.clone())) {
            edges.push(GraphEdge {
                from,
                to,
                predicate: t.predicate.clone(),
            });
        }
    }
    edges
}

/// Graph-view rows for directory people that have a graph node.
fn runtime_people_entities(
    people: &[permagent::people::Person],
) -> Vec<(String, String, String, Option<String>, EntityId)> {
    people
        .iter()
        .filter_map(|p| {
            let hex = p.graph_entity_id.as_deref()?;
            let eid = hex.parse::<EntityId>().ok()?;
            Some((
                format!("e:{}", hex::encode(eid.as_bytes())),
                "person".to_string(),
                p.display_name.clone(),
                None,
                eid,
            ))
        })
        .collect()
}

async fn brain_graph(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> Result<Json<GraphResponse>, crate::routes::errors::ErrorResponse> {
    let persona = state.persona.read().await;
    let self_node = GraphSelf {
        name: persona.first_name.clone(),
        id: "self".to_string(),
    };

    let brain = match state.brain.as_ref() {
        Some(b) => b,
        None => {
            return Ok(Json(GraphResponse {
                self_node,
                entities: Vec::new(),
                edges: Vec::new(),
                memories: Vec::new(),
            }));
        }
    };

    let now = Utc::now();
    let max_age_secs: f64 = 90.0 * 24.0 * 3600.0; // 90 days
    const MAX_MEMORIES: usize = 100;

    // --- Entities: recall's graph neighborhood, search-aware ---
    // The self-name neighborhood is the base view. When the user searches,
    // ALSO seed recall on the query and union the two neighborhoods — before
    // this, `q` re-scoped memories only and typing a person or project name
    // changed zero entities on screen (the search box lied about its reach).
    let search_seed = params.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let query_result = match search_seed {
        Some(q) => brain.recall(q, spectral::Visibility::Private).await.ok(),
        None => None,
    };
    let (entities, edges) = match brain
        .recall(&self_node.name, spectral::Visibility::Private)
        .await
    {
        Ok(mut result) => {
            if let Some(extra) = query_result {
                result
                    .graph
                    .neighborhood
                    .entities
                    .extend(extra.graph.neighborhood.entities);
                result
                    .graph
                    .neighborhood
                    .triples
                    .extend(extra.graph.neighborhood.triples);
            }
            // Dedup entities (cap 80), keeping the display data plus the raw
            // EntityId for a single batched field lookup.
            let mut seen = std::collections::HashSet::new();
            let mut picked: Vec<(String, String, String, Option<String>, EntityId)> = Vec::new();
            for ent in &result.graph.neighborhood.entities {
                let id_hex = format!("e:{}", hex::encode(ent.id.as_bytes()));
                if seen.insert(id_hex.clone()) {
                    picked.push((
                        id_hex,
                        ent.entity_type.clone(),
                        ent.canonical.clone(),
                        ent.description.clone(),
                        ent.id,
                    ));
                    if picked.len() >= 80 {
                        break;
                    }
                }
            }

            // Union in the curated ontology set (2026-07-27): the recall
            // neighborhood traverses TRIPLES only, and the Kuzu→SQLite store
            // move lost the extraction-era triples — leaving the view nearly
            // empty despite 126 restored entity nodes. The curated entities
            // are real knowledge; show them. Triple-reachable entities keep
            // priority (they come first and the cap applies to the fill).
            if picked.len() < 80 {
                if let Ok(curated) = brain.list_ontology_graph_entities().await {
                    for (id_hex_bare, entity_type, canonical, description) in curated {
                        if picked.len() >= 80 {
                            break;
                        }
                        let id_hex = format!("e:{id_hex_bare}");
                        if seen.insert(id_hex.clone()) {
                            // Re-derive the EntityId (deterministic blake3 of
                            // type+canonical — same derivation that minted it).
                            let eid =
                                spectral::core::entity_id::entity_id(&entity_type, &canonical);
                            picked.push((id_hex, entity_type, canonical, description, eid));
                        }
                    }
                }
            }

            // Union in runtime-created people (POST /api/people, the bridge,
            // the agent's own people tools). They are graph nodes but neither
            // triple-reachable from the self node nor part of the curated
            // ontology, so a Brain with real contacts still rendered 0
            // entities until some other job linked them.
            if picked.len() < 80 {
                if let Ok(pool) = state.session_manager().pool_clone().await {
                    if let Ok(people) = permagent::people::list_people(
                        &pool,
                        &permagent::people::PeopleFilter::default(),
                    )
                    .await
                    {
                        for (id_hex, entity_type, name, description, eid) in
                            runtime_people_entities(&people)
                        {
                            if picked.len() >= 80 {
                                break;
                            }
                            if seen.insert(id_hex.clone()) {
                                picked.push((id_hex, entity_type, name, description, eid));
                            }
                        }
                    }
                }
            }

            // Entity→entity edges (#495 slice 3): the neighborhood's triples
            // were previously dropped here; keep the ones connecting two
            // displayed entities.
            let picked_ids: std::collections::HashSet<String> =
                picked.iter().map(|p| p.0.clone()).collect();
            let edges = collect_entity_edges(&result.graph.neighborhood.triples, &picked_ids);

            // Batch-load typed fields once; key is the entity's bare 64-hex id.
            let mut fields_map = brain
                .entity_fields_for(picked.iter().map(|p| p.4).collect())
                .await
                .unwrap_or_default();

            let entities: Vec<GraphEntity> = picked
                .into_iter()
                .map(|(id_hex, entity_type, name, description, eid)| {
                    let fields = fields_map
                        .remove(&eid.to_string())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|f| GraphEntityField {
                            field_name: f.field_name,
                            value: f.value,
                            source: f.source.as_str().to_string(),
                            source_url: f.source_url,
                            updated_at: f.updated_at,
                        })
                        .collect();
                    GraphEntity {
                        id: id_hex,
                        entity_type,
                        name,
                        // De-hardcoded: surface the entity's real freeform
                        // description (was always "") so the modal stops
                        // rendering empty.
                        note: description.unwrap_or_default(),
                        fields,
                    }
                })
                .collect();
            (entities, edges)
        }
        Err(_) => (Vec::new(), Vec::new()),
    };

    // --- Memories ---
    let search_q = params.q.as_deref().unwrap_or("").trim().to_string();

    let mut memories = if search_q.is_empty() {
        // DEFAULT VIEW: top 100 by entity connection richness, recency tiebreak.
        // Direct SQL — shows "what does this Brain know best?"
        tokio::task::spawn_blocking(move || -> Vec<GraphMemory> {
            let conn = match crate::brain_ops::read_only_brain_conn() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let mut stmt = match conn.prepare(
                "SELECT m.id, m.key, m.content, m.description, m.signal_score, m.created_at, \
                        m.source, m.last_reinforced_at, \
                        COUNT(a.id) as entity_count \
                 FROM memories m \
                 LEFT JOIN memory_annotations a ON a.memory_id = m.id \
                 WHERE m.description IS NOT NULL \
                 GROUP BY m.id \
                 ORDER BY entity_count DESC, m.created_at DESC \
                 LIMIT ?1",
            ) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let rows = stmt
                .query_map(rusqlite::params![MAX_MEMORIES], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, f64>(4).unwrap_or(0.0),
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                })
                .ok();
            let mut result = Vec::new();
            if let Some(rows) = rows {
                for row in rows.flatten() {
                    let (
                        id,
                        key,
                        content,
                        description,
                        signal_score,
                        created_at_str,
                        source,
                        last_reinforced_at,
                    ) = row;
                    let age = created_at_str
                        .as_deref()
                        .and_then(|s| {
                            s.parse::<DateTime<Utc>>().ok().or_else(|| {
                                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                                    .ok()
                                    .map(|dt| dt.and_utc())
                            })
                        })
                        .map(|ts| {
                            ((now - ts).num_seconds().max(0) as f64 / max_age_secs).clamp(0.0, 1.0)
                        })
                        .unwrap_or(0.5);
                    let timestamp = created_at_str.unwrap_or_else(|| now.to_rfc3339());
                    result.push(GraphMemory {
                        id,
                        key,
                        text: content,
                        description,
                        ent: Vec::new(),
                        age,
                        weight: signal_score.clamp(0.0, 1.0),
                        timestamp,
                        layer: None,
                        why: None,
                        source,
                        last_reinforced_at,
                    });
                }
            }
            result
        })
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?
    } else {
        // SEARCH VIEW: semantic recall for the query neighborhood
        match brain.recall(&search_q, spectral::Visibility::Private).await {
            Ok(result) => {
                let mut memories = Vec::new();
                for hit in result.memory_hits.into_iter().take(MAX_MEMORIES) {
                    let age = hit
                        .created_at
                        .as_deref()
                        .and_then(|s| {
                            s.parse::<DateTime<Utc>>().ok().or_else(|| {
                                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                                    .ok()
                                    .map(|dt| dt.and_utc())
                            })
                        })
                        .map(|ts| {
                            ((now - ts).num_seconds().max(0) as f64 / max_age_secs).clamp(0.0, 1.0)
                        })
                        .unwrap_or(0.5);
                    let timestamp = hit.created_at.clone().unwrap_or_else(|| now.to_rfc3339());
                    memories.push(GraphMemory {
                        id: hit.id,
                        key: Some(hit.key),
                        text: hit.content,
                        description: hit.description,
                        ent: Vec::new(),
                        age,
                        weight: hit.signal_score.clamp(0.0, 1.0),
                        timestamp,
                        layer: None,
                        why: None,
                        source: hit.source,
                        last_reinforced_at: hit.last_reinforced_at,
                    });
                }
                memories
            }
            Err(_) => Vec::new(),
        }
    };

    // Populate entity links from memory_annotations table
    if !memories.is_empty() {
        let memory_ids: Vec<String> = memories.iter().map(|m| m.id.clone()).collect();
        let annotation_map = tokio::task::spawn_blocking(
            move || -> std::collections::HashMap<String, Vec<String>> {
                let conn = match crate::brain_ops::read_only_brain_conn() {
                    Ok(c) => c,
                    Err(_) => return std::collections::HashMap::new(),
                };

                let placeholders: Vec<String> =
                    (1..=memory_ids.len()).map(|i| format!("?{}", i)).collect();
                let sql = format!(
                    "SELECT memory_id, who FROM memory_annotations WHERE memory_id IN ({})",
                    placeholders.join(", ")
                );

                let mut stmt = match conn.prepare(&sql) {
                    Ok(s) => s,
                    Err(_) => return std::collections::HashMap::new(),
                };

                let params: Vec<&dyn rusqlite::types::ToSql> = memory_ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::types::ToSql)
                    .collect();

                let mut map: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();

                if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                    let memory_id: String = row.get(0)?;
                    let who_json: String = row.get(1)?;
                    Ok((memory_id, who_json))
                }) {
                    for (mem_id, who_json) in rows.flatten() {
                        if let Ok(refs) = serde_json::from_str::<Vec<serde_json::Value>>(&who_json)
                        {
                            let ent_ids: Vec<String> = refs
                                .iter()
                                .filter_map(|r| {
                                    r.get("canonical_id")
                                        .and_then(|v| v.as_str())
                                        .map(String::from)
                                })
                                .collect();
                            map.entry(mem_id).or_default().extend(ent_ids);
                        }
                    }
                }

                map
            },
        )
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

        for mem in &mut memories {
            if let Some(ents) = annotation_map.get(&mem.id) {
                mem.ent.clone_from(ents);
            }
        }
    }

    // ── Bridge: resolve annotation term:/cat: IDs to Kuzu entity IDs ──
    // Builds a lookup from entity canonical names to their graph IDs,
    // then substitutes matching annotation terms with real entity IDs
    // so the frontend can draw edges between memory and entity nodes.
    {
        let entity_map: std::collections::HashMap<String, String> = entities
            .iter()
            .map(|e| (e.name.to_lowercase(), e.id.clone()))
            .collect();

        for mem in &mut memories {
            mem.ent = mem
                .ent
                .iter()
                .map(|term_id| {
                    let name = term_id
                        .strip_prefix("term:")
                        .or_else(|| term_id.strip_prefix("cat:"))
                        .unwrap_or(term_id);
                    entity_map
                        .get(&name.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| term_id.clone())
                })
                .collect();
        }
    }

    // ── Mention-derived links (#24): the graph store's mention table is the
    // authoritative memory↔entity record (written at ingest + boot backfill).
    // Union those links into `ent` so entity nodes show their memories even
    // when no annotation names them.
    if !memories.is_empty() {
        let ids: Vec<String> = memories.iter().map(|m| m.id.clone()).collect();
        let mention_map = tokio::task::spawn_blocking(move || {
            let graph_db = permagent::config::paths::Paths::brain_dir().join("graph.sqlite");
            permagent::brain_enrichment::mention_links_for_memories(&graph_db, &ids)
        })
        .await
        .unwrap_or_default();
        for mem in &mut memories {
            if let Some(hexes) = mention_map.get(&mem.id) {
                for h in hexes {
                    let id = format!("e:{h}");
                    if !mem.ent.contains(&id) {
                        mem.ent.push(id);
                    }
                }
            }
        }
    }

    apply_assemble_layers(&mut memories);

    Ok(Json(GraphResponse {
        self_node,
        entities,
        edges,
        memories,
    }))
}

// ── GET /api/brain/memories — paginated browse + FTS search ──────────

#[derive(Debug, Deserialize)]
struct MemoriesQuery {
    /// Full-text search query. Absent = browse mode.
    #[serde(default)]
    q: Option<String>,
    /// Cursor for browse mode: only return memories with created_at < this value.
    #[serde(default)]
    before: Option<String>,
    /// Cursor tiebreaker: memory id of the last row in the previous page.
    #[serde(default)]
    before_id: Option<String>,
    /// Offset for search mode pagination (ignored in browse mode).
    #[serde(default)]
    offset: Option<usize>,
    /// Page size. Default 50, max 200.
    #[serde(default)]
    limit: Option<usize>,
    /// Time slider lower bound: only return memories with created_at >= this value (browse mode).
    #[serde(default)]
    after: Option<String>,
    /// Filter to one `RememberOpts.source` tag (List's "sources" filter row,
    /// R5). The sentinel `"unknown"` selects memories with no source recorded
    /// (written before the column existed) — never a literal `FeedSource` tag.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Serialize)]
struct MemoriesResponse {
    memories: Vec<GraphMemory>,
    total: usize,
    has_more: bool,
}

async fn brain_memories(
    Query(params): Query<MemoriesQuery>,
) -> Result<Json<MemoriesResponse>, crate::routes::errors::ErrorResponse> {
    let limit = params.limit.unwrap_or(50).min(200);
    let now = Utc::now();
    let max_age_secs: f64 = 90.0 * 24.0 * 3600.0;

    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::brain_ops::read_only_brain_conn()
            .map_err(|e| format!("Failed to open brain DB: {}", e))?;

        let q = params.q.as_deref().unwrap_or("").trim().to_string();
        let is_search = !q.is_empty();

        if is_search {
            query_fts_memories(
                &conn,
                &q,
                params.offset.unwrap_or(0),
                params.source.as_deref(),
                limit,
                now,
                max_age_secs,
            )
        } else {
            query_browse_memories(
                &conn,
                params.before.as_deref(),
                params.before_id.as_deref(),
                params.after.as_deref(),
                params.source.as_deref(),
                limit,
                (now, max_age_secs),
            )
        }
    })
    .await
    .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?
    .map_err(crate::routes::errors::ErrorResponse::internal)?;

    Ok(Json(result))
}

/// Whether `table` has `column`. Used to guard the optional `event_at` column
/// (#92) so the timeline query stays correct on a Brain DB that predates the
/// boot-time backfill (returns `false`, we fall back to `created_at`).
fn table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    while let Ok(Some(row)) = rows.next() {
        if matches!(row.get::<_, String>(1), Ok(name) if name == column) {
            return true;
        }
    }
    false
}

fn query_browse_memories(
    conn: &rusqlite::Connection,
    before: Option<&str>,
    before_id: Option<&str>,
    after: Option<&str>,
    source: Option<&str>,
    limit: usize,
    age_context: (DateTime<Utc>, f64),
) -> Result<MemoriesResponse, String> {
    let (now, max_age_secs) = age_context;
    // #92 — order the timeline by original event time when we have it, so
    // imported memories (Slack/browser history) sit at their event date rather
    // than the batch-import date. `event_at` is backfilled at boot; on an older
    // DB without the column we transparently fall back to `created_at`.
    let time_col = if table_has_column(conn, "memories", "event_at") {
        "COALESCE(event_at, created_at)"
    } else {
        "created_at"
    };

    // Build WHERE clauses dynamically
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();
    let mut param_idx = 1usize;

    // Time slider lower bound
    if let Some(a) = after {
        where_clauses.push(format!("{time_col} >= ?{param_idx}"));
        params_vec.push(a.to_string());
        param_idx += 1;
    }

    // Source filter (List's "sources" chip row, R5). "unknown" is a sentinel
    // for "no source recorded" (pre-dates the column), never a real tag.
    if let Some(s) = source {
        if s == "unknown" {
            where_clauses.push("source IS NULL".to_string());
        } else {
            where_clauses.push(format!("source = ?{param_idx}"));
            params_vec.push(s.to_string());
            param_idx += 1;
        }
    }

    // Cursor upper bound
    if let (Some(b), Some(bid)) = (before, before_id) {
        where_clauses.push(format!(
            "({time_col} < ?{} OR ({time_col} = ?{} AND id < ?{}))",
            param_idx,
            param_idx,
            param_idx + 1
        ));
        params_vec.push(b.to_string());
        params_vec.push(bid.to_string());
        param_idx += 2;
    } else if let Some(b) = before {
        where_clauses.push(format!("{time_col} < ?{param_idx}"));
        params_vec.push(b.to_string());
        param_idx += 1;
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    // Total count: applies the 'after' and 'source' filters (not cursor params,
    // which are for pagination only) — built independently of `param_idx`
    // above so the cursor's placeholder numbering above is unaffected.
    let mut count_where: Vec<String> = Vec::new();
    let mut count_params: Vec<String> = Vec::new();
    if let Some(a) = after {
        count_where.push(format!("{time_col} >= ?{}", count_params.len() + 1));
        count_params.push(a.to_string());
    }
    if let Some(s) = source {
        if s == "unknown" {
            count_where.push("source IS NULL".to_string());
        } else {
            count_where.push(format!("source = ?{}", count_params.len() + 1));
            count_params.push(s.to_string());
        }
    }
    let count_where_sql = if count_where.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", count_where.join(" AND "))
    };
    let total: usize = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM memories {count_where_sql}"),
            rusqlite::params_from_iter(count_params.iter()),
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let limit_param = format!("?{}", param_idx);
    params_vec.push((limit + 1).to_string());

    // Alias the effective time into the `created_at` slot so `row_to_graph_memory`
    // reads it unchanged and the cursor the frontend echoes back (before/before_id)
    // stays consistent with the sort key.
    let sql = format!(
        "SELECT id, key, content, description, signal_score, {time_col} AS created_at, \
                source, last_reinforced_at \
         FROM memories {} ORDER BY {time_col} DESC, id DESC LIMIT {}",
        where_sql, limit_param
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
            row_to_graph_memory(row, now, max_age_secs)
        })
        .map_err(|e| e.to_string())?;

    let mut memories: Vec<GraphMemory> = rows.filter_map(|r| r.ok()).collect();
    let has_more = memories.len() > limit;
    memories.truncate(limit);
    apply_assemble_layers(&mut memories);

    Ok(MemoriesResponse {
        memories,
        total,
        has_more,
    })
}

fn query_fts_memories(
    conn: &rusqlite::Connection,
    query: &str,
    offset: usize,
    source: Option<&str>,
    limit: usize,
    now: DateTime<Utc>,
    max_age_secs: f64,
) -> Result<MemoriesResponse, String> {
    // Sanitize query for FTS5: wrap each word in quotes, join with OR
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .map(|w| {
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            format!("\"{}\"", clean)
        })
        .filter(|w| w.len() > 2)
        .collect();

    if words.is_empty() {
        return Ok(MemoriesResponse {
            memories: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    let fts_query = words.join(" OR ");

    // Count total matches only on first page to avoid double-query per keystroke.
    // The no-filter path stays a single-table scan (no join) — unchanged from
    // before the source filter existed.
    let total: usize = if offset == 0 {
        let base = "SELECT COUNT(*) FROM memories_fts fts JOIN memories m ON m.rowid = fts.rowid \
             WHERE memories_fts MATCH ?1";
        match source {
            Some("unknown") => conn
                .query_row(&format!("{base} AND m.source IS NULL"), [&fts_query], |r| {
                    r.get(0)
                })
                .unwrap_or(0),
            Some(s) => conn
                .query_row(
                    &format!("{base} AND m.source = ?2"),
                    rusqlite::params![fts_query, s],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            None => conn
                .query_row(
                    "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
                    [&fts_query],
                    |r| r.get(0),
                )
                .unwrap_or(0),
        }
    } else {
        0 // subsequent pages don't recompute total
    };

    // "unknown" filters IS NULL inline (no bindable param); a real tag binds
    // as ?4, after the three positional params every query already sends.
    let source_clause = match source {
        Some("unknown") => " AND m.source IS NULL",
        Some(_) => " AND m.source = ?4",
        None => "",
    };
    let sql = format!(
        "SELECT m.id, m.key, m.content, m.description, m.signal_score, m.created_at, \
                m.source, m.last_reinforced_at \
         FROM memories_fts fts \
         JOIN memories m ON m.rowid = fts.rowid \
         WHERE memories_fts MATCH ?1{source_clause} \
         ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) \
         LIMIT ?2 OFFSET ?3"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let real_source = source.filter(|s| *s != "unknown");
    let rows = if let Some(s) = real_source {
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(fts_query.clone()),
            Box::new((limit + 1) as i64),
            Box::new(offset as i64),
            Box::new(s.to_string()),
        ];
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        stmt.query_map(param_refs.as_slice(), |row| {
            row_to_graph_memory(row, now, max_age_secs)
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>()
    } else {
        stmt.query_map(
            rusqlite::params![fts_query, (limit + 1) as i64, offset as i64],
            |row| row_to_graph_memory(row, now, max_age_secs),
        )
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>()
    };

    let mut memories: Vec<GraphMemory> = rows;
    let has_more = memories.len() > limit;
    memories.truncate(limit);
    apply_assemble_layers(&mut memories);

    Ok(MemoriesResponse {
        memories,
        total,
        has_more,
    })
}

fn row_to_graph_memory(
    row: &rusqlite::Row,
    now: DateTime<Utc>,
    max_age_secs: f64,
) -> rusqlite::Result<GraphMemory> {
    let id: String = row.get(0)?;
    let key: String = row.get(1)?;
    let content: String = row.get(2)?;
    let description: Option<String> = row.get(3)?;
    let signal_score: f64 = row.get(4)?;
    let created_at: Option<String> = row.get(5)?;
    let source: Option<String> = row.get(6)?;
    let last_reinforced_at: Option<String> = row.get(7)?;

    let age = created_at
        .as_deref()
        .and_then(|s| {
            s.parse::<DateTime<Utc>>().ok().or_else(|| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .map(|dt| dt.and_utc())
            })
        })
        .map(|ts| ((now - ts).num_seconds().max(0) as f64 / max_age_secs).clamp(0.0, 1.0))
        .unwrap_or(0.5);

    let timestamp = created_at.unwrap_or_else(|| now.to_rfc3339());

    Ok(GraphMemory {
        id,
        key: Some(key),
        text: content,
        description,
        ent: Vec::new(),
        age,
        weight: signal_score.clamp(0.0, 1.0),
        timestamp,
        layer: None,
        why: None,
        source,
        last_reinforced_at,
    })
}

/// Stamp Abstract/Overview/Full on graph/list rows so the side panel can deepen.
fn apply_assemble_layers(memories: &mut [GraphMemory]) {
    if memories.is_empty() {
        return;
    }
    let keys: Vec<String> = memories
        .iter()
        .map(|m| m.key.clone().unwrap_or_else(|| m.id.clone()))
        .collect();
    let abstracts: Vec<Option<String>> = memories.iter().map(|m| m.description.clone()).collect();
    let contents: Vec<String> = memories.iter().map(|m| m.text.clone()).collect();
    let scores: Vec<f64> = memories.iter().map(|m| m.weight).collect();
    let sources: Vec<permagent::context_layers::AssembleSource<'_>> = keys
        .iter()
        .zip(abstracts.iter())
        .zip(contents.iter())
        .zip(scores.iter())
        .map(
            |(((k, a), c), s)| permagent::context_layers::AssembleSource {
                key: k,
                abstract_text: a.as_deref(),
                content: c,
                score: *s,
            },
        )
        .collect();
    let layered = permagent::context_layers::assemble(
        &sources,
        permagent::context_layers::AssembleBudget::SEARCH,
    );
    let by_key: std::collections::HashMap<String, permagent::context_layers::LayeredHit> =
        layered.into_iter().map(|h| (h.key.clone(), h)).collect();
    for (mem, key) in memories.iter_mut().zip(keys.iter()) {
        if let Some(hit) = by_key.get(key) {
            mem.text = hit.text.clone();
            mem.layer = Some(hit.layer.as_str().to_string());
            mem.why = Some(hit.why.clone());
        }
    }
}

#[derive(Debug, Deserialize)]
struct MemoryByKeyQuery {
    key: String,
}

/// GET /api/brain/memory?key= — Full text for a side-panel deepen.
async fn brain_memory_by_key(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryByKeyQuery>,
) -> Result<Json<GraphMemory>, crate::routes::errors::ErrorResponse> {
    let key = params.key.trim();
    if key.is_empty() {
        return Err(crate::routes::errors::ErrorResponse::bad_request(
            "key is required",
        ));
    }
    let brain = state.brain.as_ref().ok_or_else(|| {
        crate::routes::errors::ErrorResponse::service_unavailable("Brain is not ready")
    })?;
    let mem = brain
        .get_memory_by_key(key)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?
        .ok_or_else(|| crate::routes::errors::ErrorResponse::not_found("memory not found"))?;
    let now = Utc::now();
    let max_age_secs: f64 = 90.0 * 24.0 * 3600.0;
    let age = mem
        .created_at
        .as_deref()
        .and_then(|s| {
            s.parse::<DateTime<Utc>>().ok().or_else(|| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .map(|dt| dt.and_utc())
            })
        })
        .map(|ts| ((now - ts).num_seconds().max(0) as f64 / max_age_secs).clamp(0.0, 1.0))
        .unwrap_or(0.5);
    Ok(Json(GraphMemory {
        id: mem.id,
        key: Some(mem.key),
        text: mem.content,
        description: mem.description,
        ent: Vec::new(),
        age,
        weight: mem.signal_score.clamp(0.0, 1.0),
        timestamp: mem.created_at.unwrap_or_else(|| now.to_rfc3339()),
        layer: Some("full".to_string()),
        why: Some("requested full text".to_string()),
        source: mem.source,
        last_reinforced_at: mem.last_reinforced_at,
    }))
}

/// Response for `DELETE /api/brain/memory?key=`.
#[derive(Debug, Serialize)]
struct ForgetMemoryResponse {
    /// `false` means the key named no memory — a no-op, not an error (the row
    /// was already gone, e.g. a stale UI or a double-click).
    existed: bool,
    /// Both post-delete probes (`recall`, `recognize`) came back clear. `false`
    /// with `existed: true` means the hard-delete ran but residue was found —
    /// surfaced so a caller doesn't take a silent partial forget as done.
    verified: bool,
}

/// DELETE /api/brain/memory?key= — hard-delete a memory across every Spectral
/// substrate (R5 "explicit forget with undo"; the undo window itself is
/// client-side — see BrainList.tsx's Forget action, which delays this request
/// until an 8s toast expires unactioned). Reuses `SafeBrain::forget`, the same
/// verified-forgetting primitive the Reader uses to retire a stale document.
///
/// Auth: this route sits behind the same bearer-token layer as every other
/// `/api/brain/*` route (see `routes::mod::routes`) — no extra check here.
async fn forget_memory(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MemoryByKeyQuery>,
) -> Result<Json<ForgetMemoryResponse>, crate::routes::errors::ErrorResponse> {
    let key = params.key.trim();
    if key.is_empty() {
        return Err(crate::routes::errors::ErrorResponse::bad_request(
            "key is required",
        ));
    }
    let brain = state.brain.as_ref().ok_or_else(|| {
        crate::routes::errors::ErrorResponse::service_unavailable("Brain is not ready")
    })?;
    let report = brain
        .forget(key)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    // Audit trail (R5 "what did the agent — or I — do"): record the forget in
    // the durable activity journal even though the memory itself is now gone.
    // Best-effort — a journal write failure must never undo the forget or fail
    // the request, since the memory is already gone from Spectral by this
    // point. Not added to `activity_journal::KNOWN_KINDS` (that list also
    // drives the frontend's kind-filter chips); the row is still durable and
    // shows on the unfiltered activity timeline and via direct query.
    if report.store.existed {
        if let Ok(pool) = state.session_manager().pool_clone().await {
            let entry = permagent::activity_journal::NewEntry {
                id: uuid::Uuid::now_v7().to_string(),
                ts: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                kind: "memory_forgotten".to_string(),
                actor: permagent::activity_journal::Actor::User,
                title: "Forgot a memory".to_string(),
                detail: None,
                ref_kind: Some("memory".to_string()),
                ref_id: Some(key.to_string()),
            };
            let _ = permagent::activity_journal::insert_entry(&pool, &entry).await;
        }
    }

    Ok(Json(ForgetMemoryResponse {
        existed: report.store.existed,
        verified: report.recall_clear && report.recognize_clear,
    }))
}

/// One-shot repair: move approval records misfiled into the `event` hall into
/// `fact`.
///
/// See [`permagent::hall_backfill`] for what this does and why. Snapshots all
/// three brain databases before it writes anything, and is safe to re-run — a
/// second call selects zero rows.
///
/// The response carries the before/after `COUNT(*)` verbatim. A non-zero
/// `after` means the repair did not reach every row; the ids are in `missed`
/// and `errors`. Do not read `repaired` as success on its own.
#[utoipa::path(post, path = "/api/brain/backfill-approval-halls",
    responses(
        (status = 200, description = "Approval records moved from the event hall to fact; before/after counts and per-row failures"),
        (status = 503, description = "Brain is not ready"),
    )
)]
async fn backfill_approval_halls(
    State(state): State<Arc<AppState>>,
) -> Result<Json<permagent::hall_backfill::HallBackfillReport>, crate::routes::errors::ErrorResponse>
{
    let brain = state.brain.as_ref().ok_or_else(|| {
        crate::routes::errors::ErrorResponse::service_unavailable("Brain is not ready")
    })?;
    let report = permagent::hall_backfill::run_on_default_paths(brain)
        .await
        .map_err(crate::routes::errors::ErrorResponse::internal)?;
    Ok(Json(report))
}

/// Survey chat memories in the catch-all wing, and (when Spectral supports the
/// write) assign a wing where two independent signals agree.
///
/// See [`permagent::wing_backfill`] for the rule and the measurement behind it.
/// **Dry run by default**: `apply` is `false` unless explicitly passed, and a
/// dry run is the shape to run first. The response reports all four buckets —
/// `corroborated`, `conflicting`, `unverifiable`, `noHint` — plus corroborated
/// counts by signal and by wing, and up to 25 verbatim examples.
///
/// A `applyBlocked` string in the response means a write was requested and
/// refused, with the reason. Do not read a zero `applied` as "nothing matched";
/// read `corroborated`.
#[utoipa::path(post, path = "/api/brain/backfill-chat-wings",
    params(("apply" = Option<bool>, Query, description = "Write the corroborated bucket. Default false (dry run).")),
    responses(
        (status = 200, description = "Bucket counts, per-signal yield, and sample rows"),
        (status = 503, description = "Session store is not ready"),
    )
)]
async fn backfill_chat_wings(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BackfillChatWingsQuery>,
) -> Result<Json<permagent::wing_backfill::WingBackfillReport>, crate::routes::errors::ErrorResponse>
{
    let pool =
        state.session_manager().pool_clone().await.map_err(|e| {
            crate::routes::errors::ErrorResponse::service_unavailable(e.to_string())
        })?;
    let report = permagent::wing_backfill::run_on_default_paths(&pool, params.apply)
        .await
        .map_err(crate::routes::errors::ErrorResponse::internal)?;
    Ok(Json(report))
}

/// `apply` defaults to false: the safe shape has to be the one you get by not
/// thinking about it.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackfillChatWingsQuery {
    #[serde(default)]
    apply: bool,
}

/// Chat persistence's own outbox depth (`chat_memory_outbox`) — the memory-feed
/// panel's "Conversations" row needs this alongside [`permagent::brain_feed`]'s
/// per-source counts, since chat turns are enqueued through the older,
/// dedicated outbox (`crates/goose-server/src/brain_ops.rs:559-719`), not
/// `brain_feed`'s.
#[derive(Debug, Clone, Copy, Default, Serialize, utoipa::ToSchema)]
pub struct ChatOutboxStatus {
    /// Queued (`pending` or `running`), not yet delivered.
    pub pending: i64,
    /// Gave up after the outbox's own attempt cap.
    pub dead: i64,
}

async fn chat_outbox_status(pool: &sqlx::Pool<sqlx::Sqlite>) -> ChatOutboxStatus {
    let row = sqlx::query(
        "SELECT
            SUM(CASE WHEN status IN ('pending','running') THEN 1 ELSE 0 END) AS pending,
            SUM(CASE WHEN status = 'dead' THEN 1 ELSE 0 END) AS dead
         FROM chat_memory_outbox",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some(r) => ChatOutboxStatus {
            pending: r
                .try_get::<Option<i64>, _>("pending")
                .ok()
                .flatten()
                .unwrap_or(0),
            dead: r
                .try_get::<Option<i64>, _>("dead")
                .ok()
                .flatten()
                .unwrap_or(0),
        },
        None => ChatOutboxStatus::default(),
    }
}

/// Response for `GET /api/brain/feed-status` — Settings → Memory's "What Henry
/// learns from" panel (memory-feed lane, 2026-09-25).
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FeedStatusResponse {
    pub sources: Vec<permagent::brain_feed::FeedSourceStatus>,
    pub chat_outbox: ChatOutboxStatus,
}

/// GET /api/brain/feed-status — per-source counts, last-fed time and errors
/// for every [`permagent::brain_feed::FeedSource`], plus the chat outbox's own
/// depth. Never 404s: every source appears even with zero facts fed, so the
/// panel can render "not yet" honestly instead of omitting the row.
async fn brain_feed_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FeedStatusResponse>, crate::routes::errors::ErrorResponse> {
    let pool =
        state.session_manager().pool_clone().await.map_err(|e| {
            crate::routes::errors::ErrorResponse::service_unavailable(e.to_string())
        })?;
    let disabled = permagent::brain_feed::disabled_sources();
    let sources = permagent::brain_feed::feed_status(&pool, &disabled)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
    let chat_outbox = chat_outbox_status(&pool).await;
    Ok(Json(FeedStatusResponse {
        sources,
        chat_outbox,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/brain/search", get(brain_search))
        .route("/api/brain/graph", get(brain_graph))
        .route("/api/brain/memories", get(brain_memories))
        .route(
            "/api/brain/memory",
            get(brain_memory_by_key).delete(forget_memory),
        )
        .route("/api/brain/feed-status", get(brain_feed_status))
        .route(
            "/api/brain/backfill-approval-halls",
            post(backfill_approval_halls),
        )
        .route("/api/brain/backfill-chat-wings", post(backfill_chat_wings))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::core::entity_id::entity_id;
    use spectral::graph::graph_store::Triple;

    /// A directory person with a graph node must reach the graph view even
    /// though no triple or ontology entry mentions them (film staging: three
    /// contacts, 0 entities).
    #[tokio::test]
    async fn runtime_people_become_graph_entities() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        let hex = permagent::identity::canonical::graph_entity_id_hex("person", "Maya Chen");
        let mut maya = permagent::people::upsert_person(
            &pool,
            "person:maya-chen",
            "Maya Chen",
            &permagent::people::PersonAttrs::default(),
        )
        .await
        .unwrap();
        maya.graph_entity_id = Some(hex.clone());
        let mut orphan = maya.clone();
        orphan.graph_entity_id = None;
        let rows = runtime_people_entities(&[maya, orphan]);
        assert_eq!(rows.len(), 1, "a person without a graph node is skipped");
        assert_eq!(rows[0].0, format!("e:{hex}"));
        assert_eq!(rows[0].1, "person");
        assert_eq!(rows[0].2, "Maya Chen");
    }

    /// GET /api/brain/feed-status's two ingredients: every `brain_feed`
    /// source (even unfed ones, count 0) plus the chat outbox's own pending/
    /// dead depth read straight from `chat_memory_outbox` — a table
    /// `brain_feed` does not own, so the panel's "Conversations" row would be
    /// permanently wrong without this second query.
    #[tokio::test]
    async fn feed_status_response_covers_every_source_and_the_chat_outbox() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::brain_feed::apply_schema(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE chat_memory_outbox (
                id INTEGER PRIMARY KEY,
                status TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO chat_memory_outbox (status) VALUES ('pending'), ('running'), ('dead'), ('applied')")
            .execute(&pool)
            .await
            .unwrap();

        let outbox = chat_outbox_status(&pool).await;
        assert_eq!(outbox.pending, 2, "pending + running");
        assert_eq!(outbox.dead, 1);

        let sources = permagent::brain_feed::feed_status(&pool, &[])
            .await
            .unwrap();
        assert_eq!(
            sources.len(),
            permagent::brain_feed::FeedSource::ALL.len(),
            "every source appears even with zero facts fed"
        );
        let chat_row = sources.iter().find(|s| s.source == "chat").unwrap();
        assert_eq!(
            chat_row.count, 0,
            "chat is fed via its own outbox, not brain_feed's"
        );
    }

    fn triple(from_canonical: &str, to_canonical: &str, predicate: &str) -> Triple {
        Triple {
            from: entity_id("person", from_canonical),
            to: entity_id("project", to_canonical),
            predicate: predicate.to_string(),
            confidence: 1.0,
            source_doc_id: None,
            source_brain_id: spectral::core::identity::BrainId::from_bytes([0u8; 32]),
            asserted_at: Utc::now(),
            visibility: spectral::Visibility::Private,
            weight: 1.0,
        }
    }

    fn graph_id(entity_type: &str, canonical: &str) -> String {
        format!(
            "e:{}",
            hex::encode(entity_id(entity_type, canonical).as_bytes())
        )
    }

    #[test]
    fn edges_kept_only_between_picked_entities_and_deduped() {
        let triples = vec![
            triple("jesse sharratt", "permagent", "works_on"),
            // Duplicate assertion — the store appends, display must not.
            triple("jesse sharratt", "permagent", "works_on"),
            // Endpoint outside the picked set — dropped.
            triple("jesse sharratt", "unpicked project", "works_on"),
        ];
        let picked: std::collections::HashSet<String> = [
            graph_id("person", "jesse sharratt"),
            graph_id("project", "permagent"),
        ]
        .into_iter()
        .collect();

        let edges = collect_entity_edges(&triples, &picked);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, graph_id("person", "jesse sharratt"));
        assert_eq!(edges[0].to, graph_id("project", "permagent"));
        assert_eq!(edges[0].predicate, "works_on");
    }

    #[test]
    fn self_loops_and_foreign_edges_are_dropped() {
        let mut loop_triple = triple("jesse sharratt", "x", "works_on");
        loop_triple.to = loop_triple.from; // degenerate self-loop
        let triples = vec![loop_triple, triple("someone else", "elsewhere", "works_on")];
        let picked: std::collections::HashSet<String> =
            [graph_id("person", "jesse sharratt")].into_iter().collect();

        assert!(collect_entity_edges(&triples, &picked).is_empty());
    }

    // ── #92: timeline orders by COALESCE(event_at, created_at) ────────────

    fn seed_browse_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                 id TEXT PRIMARY KEY,
                 key TEXT,
                 content TEXT,
                 description TEXT,
                 signal_score REAL,
                 created_at TEXT,
                 event_at TEXT,
                 source TEXT,
                 last_reinforced_at TEXT
             );",
        )
        .unwrap();
        // Imported in July but the event was in March — event_at wins.
        conn.execute(
            "INSERT INTO memories (id, key, content, description, signal_score, created_at, event_at, source, last_reinforced_at) \
             VALUES ('march','import-core','march slack',NULL,0.0,'2026-07-01T00:00:00+00:00','2026-03-14T09:30:00+00:00','permagent.card',NULL)",
            [],
        ).unwrap();
        // A native June memory with no event_at — falls back to created_at.
        // No source (pre-dates the column) — exercises the "unknown" filter.
        conn.execute(
            "INSERT INTO memories (id, key, content, description, signal_score, created_at, event_at, source, last_reinforced_at) \
             VALUES ('june','session:chat','june chat',NULL,0.0,'2026-06-01T00:00:00+00:00',NULL,NULL,'2026-06-02T00:00:00+00:00')",
            [],
        ).unwrap();
        conn
    }

    #[test]
    fn browse_orders_by_event_at_then_created_at() {
        let conn = seed_browse_db();
        let now = Utc::now();
        let res = query_browse_memories(
            &conn,
            None,
            None,
            None,
            None,
            50,
            (now, 90.0 * 24.0 * 3600.0),
        )
        .unwrap();
        let ids: Vec<&str> = res.memories.iter().map(|m| m.id.as_str()).collect();
        // June (event 2026-06) sorts above March (event 2026-03), even though
        // March was *imported* (created_at) more recently in July.
        assert_eq!(ids, vec!["june", "march"]);
        // The returned timestamp reflects the effective (event) time, so the
        // cursor the frontend echoes back stays consistent with the sort key.
        let march = res.memories.iter().find(|m| m.id == "march").unwrap();
        assert_eq!(march.timestamp, "2026-03-14T09:30:00+00:00");
        // Source and last-reinforced pass through untouched (R5: "a memory
        // shows its source").
        assert_eq!(march.source.as_deref(), Some("permagent.card"));
        let june = res.memories.iter().find(|m| m.id == "june").unwrap();
        assert_eq!(june.source, None);
        assert_eq!(
            june.last_reinforced_at.as_deref(),
            Some("2026-06-02T00:00:00+00:00")
        );
    }

    #[test]
    fn browse_after_bound_uses_effective_time() {
        let conn = seed_browse_db();
        let now = Utc::now();
        // Slider lower bound at 2026-05: March's *import* date (July) would pass a
        // created_at filter, but its effective event time (March) must exclude it.
        let res = query_browse_memories(
            &conn,
            None,
            None,
            Some("2026-05-01T00:00:00+00:00"),
            None,
            50,
            (now, 90.0 * 24.0 * 3600.0),
        )
        .unwrap();
        let ids: Vec<&str> = res.memories.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["june"]);
        assert_eq!(res.total, 1);
    }

    #[test]
    fn browse_falls_back_when_event_at_column_absent() {
        // A pre-backfill DB with no event_at column must still work (created_at).
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                 id TEXT PRIMARY KEY, key TEXT, content TEXT, description TEXT,
                 signal_score REAL, created_at TEXT, source TEXT, last_reinforced_at TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, key, content, description, signal_score, created_at) \
             VALUES ('a','k','c',NULL,0.0,'2026-06-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        assert!(!table_has_column(&conn, "memories", "event_at"));
        let res = query_browse_memories(
            &conn,
            None,
            None,
            None,
            None,
            50,
            (Utc::now(), 90.0 * 24.0 * 3600.0),
        )
        .unwrap();
        assert_eq!(res.memories.len(), 1);
        assert_eq!(res.memories[0].id, "a");
    }

    #[test]
    fn browse_filters_by_source() {
        let conn = seed_browse_db();
        let now = Utc::now();
        let res = query_browse_memories(
            &conn,
            None,
            None,
            None,
            Some("permagent.card"),
            50,
            (now, 90.0 * 24.0 * 3600.0),
        )
        .unwrap();
        let ids: Vec<&str> = res.memories.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["march"]);
        assert_eq!(res.total, 1);
    }

    #[test]
    fn browse_filters_by_unknown_source_sentinel() {
        let conn = seed_browse_db();
        let now = Utc::now();
        // "unknown" selects rows with no source recorded (NULL) — "june" here —
        // never a literal FeedSource tag string.
        let res = query_browse_memories(
            &conn,
            None,
            None,
            None,
            Some("unknown"),
            50,
            (now, 90.0 * 24.0 * 3600.0),
        )
        .unwrap();
        let ids: Vec<&str> = res.memories.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["june"]);
        assert_eq!(res.total, 1);
    }

    // ── Lane R61a: repo-scoped search ──

    fn scoped_query(
        cwd: Option<&str>,
        projects: Option<&str>,
        all: Option<bool>,
    ) -> BrainSearchQuery {
        BrainSearchQuery {
            q: "retry policy".to_string(),
            since: None,
            until: None,
            source: None,
            offset: 0,
            limit: None,
            scope_cwd: cwd.map(str::to_string),
            scope_repo_root: None,
            scope_projects: projects.map(str::to_string),
            scope_all: all,
        }
    }

    fn hit_in(key: &str, wing: Option<&str>) -> spectral::ingest::MemoryHit {
        spectral::ingest::MemoryHit {
            id: format!("id-{key}"),
            key: key.to_string(),
            content: format!("memory {key}"),
            wing: wing.map(str::to_string),
            hall: None,
            signal_score: 0.9,
            visibility: "private".to_string(),
            hits: 1,
            source: Some("chat".to_string()),
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    /// Rows from two projects plus general chat memory. A search scoped to the
    /// harness's repo returns only that repo's rows.
    fn two_project_fixture() -> Vec<spectral::ingest::MemoryHit> {
        vec![
            hit_in("acme-1", Some("acme-api")),
            hit_in("plekk-1", Some("plekk")),
            hit_in("acme-2", Some("acme-api")),
            hit_in("general-1", None),
            hit_in("plekk-2", Some("plekk")),
        ]
    }

    fn keys(hits: &[spectral::ingest::MemoryHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.key.as_str()).collect()
    }

    #[test]
    fn a_scoped_search_returns_only_the_current_projects_memories() {
        let scope = SearchScope::from_query(
            &scoped_query(Some("/work/acme-api"), None, None),
            Some("acme-api".into()),
        );
        let kept = retain_in_scope(two_project_fixture(), &scope);
        assert_eq!(keys(&kept), ["acme-1", "acme-2"]);
        assert!(
            !scope.allows_chat(),
            "chat transcript rows carry no project attribution and are never in a repo scope"
        );
    }

    #[test]
    fn a_directory_no_project_claims_gets_no_memories_not_all_of_them() {
        let scope = SearchScope::from_query(&scoped_query(Some("/tmp/scratch"), None, None), None);
        assert_eq!(scope, SearchScope::Wings(Default::default()));
        assert!(retain_in_scope(two_project_fixture(), &scope).is_empty());
        let explicit_false = SearchScope::from_query(&scoped_query(None, None, Some(false)), None);
        assert!(
            retain_in_scope(two_project_fixture(), &explicit_false).is_empty(),
            "`scope_all=false` alone is still a scoped search with nothing in it"
        );
    }

    #[test]
    fn a_granted_project_widens_the_scope_by_exactly_that_project() {
        let scope = SearchScope::from_query(
            &scoped_query(Some("/work/acme-api"), Some(" plekk , "), None),
            Some("acme-api".into()),
        );
        let kept = retain_in_scope(two_project_fixture(), &scope);
        assert_eq!(keys(&kept), ["acme-1", "plekk-1", "acme-2", "plekk-2"]);
        assert!(!scope.admits(None), "unattributed memories stay out");
    }

    #[test]
    fn only_an_all_memories_grant_or_no_scope_at_all_admits_unattributed_rows() {
        let all = SearchScope::from_query(
            &scoped_query(Some("/work/acme-api"), None, Some(true)),
            None,
        );
        assert_eq!(all, SearchScope::All);
        assert_eq!(retain_in_scope(two_project_fixture(), &all).len(), 5);
        let unscoped = SearchScope::from_query(&scoped_query(None, None, None), None);
        assert_eq!(
            unscoped,
            SearchScope::Unscoped,
            "the Brain view's own search sends no scope and is unchanged"
        );
        assert!(unscoped.allows_chat());
    }
}
