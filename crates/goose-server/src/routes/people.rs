//! People (CRM) read + manual-write routes.
//!
//! Endpoints:
//!   GET   /api/people              — list / surface people, optionally filtered
//!   PATCH /api/people/{id}/fields  — set a person's typed fields (manual edit)
//!
//! Identity comes from the typed `people` table in permagent.db; person
//! *attributes* (role/company/email/…) are read through the people↔graph bridge
//! from Spectral `entity_fields` — the graph is authoritative (Decision A, #255).
//! See [`overlay_graph_attributes`] and [`permagent::people`].
//!
//! The write path (slice 2b, #495) records fields with **`FieldSource::Manual`**
//! provenance. That's what makes the "enrichment never clobbers a manual value"
//! guarantee non-vacuous: before this, nothing ever wrote Manual, so Spectral's
//! store rule had nothing to protect. A manual write persists in the graph and a
//! later `Enriched` write for the same field is suppressed by the store.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
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

/// Body for `PATCH /api/people/{id}/fields`: a partial map of
/// `entity_fields` name → value. Only names in [`people::PERSON_FIELD_NAMES`]
/// are accepted; any other name rejects the whole request (400) so a typo can
/// never silently write an off-vocabulary field.
#[derive(Debug, Deserialize)]
pub struct SetFieldsRequest {
    pub fields: HashMap<String, String>,
}

/// Set one or more typed person fields with **manual** provenance (slice 2b).
///
/// Writes straight to the authoritative graph via `set_entity_field` /
/// `FieldSource::Manual` — never to the legacy people-table columns, which are
/// not the response source (Decision A). The response is the re-overlaid
/// [`Person`], reflecting exactly what the graph now holds, so the client sees
/// the persisted truth (and can confirm a manual value stuck).
async fn set_person_fields_handler(
    State(state): State<Arc<AppState>>,
    Path(entity_uuid): Path<String>,
    Json(req): Json<SetFieldsRequest>,
) -> Result<Json<Person>, (StatusCode, String)> {
    if req.fields.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No fields supplied".to_string()));
    }
    // Validate the whole batch before writing anything (all-or-nothing on the
    // vocabulary check; the writes themselves are per-field but pre-validated).
    for name in req.fields.keys() {
        if !people::PERSON_FIELD_NAMES.contains(&name.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown person field: {name}"),
            ));
        }
    }

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut person = people::get_by_uuid(&pool, &entity_uuid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Person not found".to_string()))?;

    let brain = state.brain.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Brain unavailable — cannot persist manual fields".to_string(),
    ))?;

    let hex = person.graph_entity_id.as_deref().ok_or((
        StatusCode::CONFLICT,
        "Person has no graph bridge yet — cannot write graph fields".to_string(),
    ))?;
    let entity_id: spectral::core::entity_id::EntityId = hex.parse().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid graph_entity_id: {e:?}"),
        )
    })?;

    for (name, value) in &req.fields {
        brain
            .set_entity_field(
                entity_id,
                name,
                value,
                spectral::ingest::FieldSource::Manual,
                None,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("set_entity_field({name}): {e}"),
                )
            })?;
    }
    tracing::info!(
        target: "permagentd::people_attrs",
        entity_uuid = %entity_uuid,
        fields = req.fields.len(),
        "Manual person-field write (FieldSource::Manual)"
    );

    // Reflect authoritative graph state back (Decision A): clear columns, overlay
    // the freshly-written `entity_fields`. The response is the graph's truth.
    overlay_graph_attributes(state.brain.as_ref(), vec![&mut person]).await;
    Ok(Json(person))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/people", get(list_people_handler))
        .route("/api/people/{id}/fields", patch(set_person_fields_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    /// Drive `PATCH /api/people/{id}/fields` against a real router. Builds the
    /// process-global AppState (→ #[serial], test_root pinned first per the #843
    /// standing rule). In the test env no brain ontology is written, so
    /// `state.brain` is `None`; these cases are chosen to resolve BEFORE the
    /// brain check (field validation, then person lookup) so they're
    /// deterministic without a brain. The manual-write persistence + provenance
    /// guarantees live in `permagent`'s `people_manual_field_edit` integration
    /// test, which owns a real Brain.
    async fn patch_fields(uuid: &str, body: serde_json::Value) -> StatusCode {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let app = routes(state);
        let req = Request::builder()
            .uri(format!("/api/people/{uuid}/fields"))
            .method("PATCH")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn rejects_off_vocabulary_field_name() {
        // Validation runs before any lookup — a bogus field never touches the DB.
        let status = patch_fields(
            "any-uuid",
            serde_json::json!({ "fields": { "ssn": "123" } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn rejects_empty_field_map() {
        let status = patch_fields("any-uuid", serde_json::json!({ "fields": {} })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn unknown_person_is_not_found() {
        // Valid field name (incl. a #495 addition), but the person doesn't exist:
        // the route is mounted and reachable, and person lookup precedes the
        // brain check, so this is a clean 404 (never a 405/404-route).
        let status = patch_fields(
            "00000000-0000-0000-0000-000000000000",
            serde_json::json!({ "fields": { "birthday": "1990-01-01" } }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
