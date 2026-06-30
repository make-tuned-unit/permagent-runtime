//! People (CRM) read routes.
//!
//! Endpoints:
//!   GET /api/people        — list / surface people, optionally filtered by attribute
//!
//! Identity comes from the typed `people` table in permagent.db; person
//! *attributes* (role/company/email/…) are read through the people↔graph bridge
//! from Spectral `entity_fields` — the graph is authoritative (Decision A, #255).
//! See [`overlay_graph_attributes`] and [`permagent::people`].

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use permagent::brain_handle::SafeBrain;
use permagent::people::{self, PeopleFilter, Person};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Overlay person attributes from the graph (`entity_fields`) onto identity rows,
/// **replacing** any people-table column values — the graph is the single source
/// of truth for attributes (Decision A, #255). Read-through in one batched
/// `entity_fields_for` hop, keyed by each row's immutable `graph_entity_id`.
///
/// Attributes are blank where the graph holds nothing (unenriched until slice
/// 2b/4 write them). The columns remain in the DB as a safety net until the
/// Step-3 drop, but are never the response source. Logs the read-through latency
/// for the Decision E (measure-before-cache) ruling.
pub(crate) async fn overlay_graph_attributes(brain: Option<&SafeBrain>, people: Vec<&mut Person>) {
    // Decision A: the response reflects the graph only — clear column-sourced
    // values first so a stale/empty column can never leak through.
    let mut people = people;
    for p in people.iter_mut() {
        p.clear_attributes();
    }

    let Some(brain) = brain else {
        return;
    };

    // Batch the graph read: collect valid EntityIds, remembering which rows map
    // to each bare-hex id.
    let mut ids = Vec::new();
    let mut by_hex: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in people.iter().enumerate() {
        if let Some(hex) = p.graph_entity_id.as_deref() {
            if let Ok(eid) = hex.parse::<spectral::core::entity_id::EntityId>() {
                by_hex.entry(hex.to_string()).or_default().push(i);
                ids.push(eid);
            }
        }
    }
    if ids.is_empty() {
        return;
    }

    let n = ids.len();
    let started = Instant::now();
    let fields_map = match brain.entity_fields_for(ids).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                target: "permagentd::people_attrs",
                error = %e,
                "Graph attribute read failed — attributes blank this request"
            );
            return;
        }
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(
        target: "permagentd::people_attrs",
        people = n,
        with_fields = fields_map.len(),
        elapsed_ms,
        "Graph attribute read-through (Decision E latency)"
    );

    for (hex, fields) in fields_map {
        if let Some(idxs) = by_hex.get(&hex) {
            for &i in idxs {
                for f in &fields {
                    people[i].set_attribute(&f.field_name, f.value.clone());
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PeopleQuery {
    /// Exact company match.
    #[serde(default)]
    pub company: Option<String>,
    /// Exact role match.
    #[serde(default)]
    pub role: Option<String>,
    /// Free-text substring over display_name / email / company.
    #[serde(default)]
    pub q: Option<String>,
}

async fn list_people_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PeopleQuery>,
) -> Result<Json<Vec<Person>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let filter = PeopleFilter {
        company: params.company,
        role: params.role,
        query: params.q,
    };

    let mut people = people::list_people(&pool, &filter)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    overlay_graph_attributes(state.brain.as_ref(), people.iter_mut().collect()).await;

    Ok(Json(people))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/people", get(list_people_handler))
        .with_state(state)
}
