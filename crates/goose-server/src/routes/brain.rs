use crate::state::AppState;
use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use permagent::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};
use spectral::core::entity_id::EntityId;
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
) -> anyhow::Result<Vec<BrainSearchResult>> {
    let brain = match state.brain.as_ref() {
        Some(b) => b,
        None => return Ok(Vec::new()),
    };

    let recall_result = brain.recall(query, spectral::Visibility::Private).await?;

    let mut results = Vec::new();
    for (i, hit) in recall_result.memory_hits.into_iter().enumerate() {
        // Spectral doesn't expose timestamps on recall hits.
        // Use Utc::now() as a placeholder; filter by score floor instead.
        let ts = Utc::now();

        // Apply date filter if provided (best-effort — Spectral lacks native date support)
        if let Some(ref s) = since {
            if ts < *s {
                continue;
            }
        }
        if let Some(ref u) = until {
            if ts > *u {
                continue;
            }
        }

        let preview = truncate_preview(&hit.content, 200);
        results.push(BrainSearchResult {
            source: "memory".to_string(),
            id: format!("spectral:{}", i),
            preview,
            score: hit.signal_score,
            timestamp: ts,
            session_id: None,
        });
    }
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

    let (fts_results, spectral_results) = match source_filter {
        "memory" => (
            Vec::new(),
            query_spectral(&state, &params.q, params.since, params.until)
                .await
                .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?,
        ),
        "chat" => (
            query_fts(&state, &params.q, params.since, params.until)
                .await
                .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?,
            Vec::new(),
        ),
        _ => {
            let (fts, spectral) = tokio::try_join!(
                query_fts(&state, &params.q, params.since, params.until),
                query_spectral(&state, &params.q, params.since, params.until),
            )
            .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
            (fts, spectral)
        }
    };

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
    triples: &[spectral::graph::kuzu_store::Triple],
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
                    ))
                })
                .ok();
            let mut result = Vec::new();
            if let Some(rows) = rows {
                for row in rows.flatten() {
                    let (id, key, content, description, signal_score, created_at_str) = row;
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
                limit,
                now,
                max_age_secs,
            )
        }
    })
    .await
    .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?
    .map_err(crate::routes::errors::ErrorResponse::internal)?;

    Ok(Json(result))
}

fn query_browse_memories(
    conn: &rusqlite::Connection,
    before: Option<&str>,
    before_id: Option<&str>,
    after: Option<&str>,
    limit: usize,
    now: DateTime<Utc>,
    max_age_secs: f64,
) -> Result<MemoriesResponse, String> {
    // Build WHERE clauses dynamically
    let mut where_clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();
    let mut param_idx = 1usize;

    // Time slider lower bound
    if let Some(a) = after {
        where_clauses.push(format!("created_at >= ?{}", param_idx));
        params_vec.push(a.to_string());
        param_idx += 1;
    }

    // Cursor upper bound
    if let (Some(b), Some(bid)) = (before, before_id) {
        where_clauses.push(format!(
            "(created_at < ?{} OR (created_at = ?{} AND id < ?{}))",
            param_idx,
            param_idx,
            param_idx + 1
        ));
        params_vec.push(b.to_string());
        params_vec.push(bid.to_string());
        param_idx += 2;
    } else if let Some(b) = before {
        where_clauses.push(format!("created_at < ?{}", param_idx));
        params_vec.push(b.to_string());
        param_idx += 1;
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    // Total count: only apply the 'after' filter (not cursor params which are for pagination)
    let total: usize = if let Some(a) = after {
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at >= ?1",
            [a],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?
    } else {
        conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(|e| e.to_string())?
    };

    let limit_param = format!("?{}", param_idx);
    params_vec.push((limit + 1).to_string());

    let sql = format!(
        "SELECT id, key, content, description, signal_score, created_at \
         FROM memories {} ORDER BY created_at DESC, id DESC LIMIT {}",
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

    // Count total matches only on first page to avoid double-query per keystroke
    let total: usize = if offset == 0 {
        conn.query_row(
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
            [&fts_query],
            |r| r.get(0),
        )
        .unwrap_or(0)
    } else {
        0 // subsequent pages don't recompute total
    };

    let sql = "SELECT m.id, m.key, m.content, m.description, m.signal_score, m.created_at \
         FROM memories_fts fts \
         JOIN memories m ON m.rowid = fts.rowid \
         WHERE memories_fts MATCH ?1 \
         ORDER BY bm25(memories_fts, 1.0, 1.0, 0.5) \
         LIMIT ?2 OFFSET ?3"
        .to_string();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![fts_query, (limit + 1) as i64, offset as i64],
            |row| row_to_graph_memory(row, now, max_age_secs),
        )
        .map_err(|e| e.to_string())?;

    let mut memories: Vec<GraphMemory> = rows.filter_map(|r| r.ok()).collect();
    let has_more = memories.len() > limit;
    memories.truncate(limit);

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
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/brain/search", get(brain_search))
        .route("/api/brain/graph", get(brain_graph))
        .route("/api/brain/memories", get(brain_memories))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::core::entity_id::entity_id;
    use spectral::graph::kuzu_store::Triple;

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
}
