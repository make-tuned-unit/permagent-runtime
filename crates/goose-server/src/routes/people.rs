//! People (CRM) read + manual-write routes.
//!
//! Endpoints:
//!   GET   /api/people              — list / surface people, optionally filtered
//!   PATCH /api/people/{id}/fields  — set a person's typed fields (manual edit)
//!   GET/POST/DELETE /api/people/{id}/relationships — typed Brain graph edges
//!   GET   /api/people/{id}/activity — memories/notes/cards referencing a person
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
    routing::{delete, get, patch},
    Json, Router,
};
use permagent::brain_handle::SafeBrain;
use permagent::people::{self, PeopleFilter, Person};
use serde::{Deserialize, Serialize};
use sqlx::Row;
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

const MAX_PERSON_FIELD_VALUE_LEN: usize = 2_000;

fn validate_person_field(name: &str, value: &str) -> Result<(), String> {
    if !people::PERSON_FIELD_NAMES.contains(&name) {
        return Err(format!("Unknown person field: {name}"));
    }
    if value.chars().count() > MAX_PERSON_FIELD_VALUE_LEN {
        return Err(format!(
            "Person field {name} exceeds {MAX_PERSON_FIELD_VALUE_LEN} characters"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("Person field {name} contains control characters"));
    }
    if name == "email" && !value.is_empty() {
        let (local, domain) = value
            .split_once('@')
            .ok_or_else(|| "Invalid email address".to_string())?;
        if local.is_empty()
            || domain.starts_with('.')
            || domain.ends_with('.')
            || !domain.contains('.')
            || value.chars().any(char::is_whitespace)
        {
            return Err("Invalid email address".to_string());
        }
    }
    if name == "birthday" && !value.is_empty() {
        let valid = value.len() == 10
            && value.as_bytes()[4] == b'-'
            && value.as_bytes()[7] == b'-'
            && value
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
        if !valid {
            return Err("Invalid birthday; expected YYYY-MM-DD".to_string());
        }
        let month: u8 = value.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0);
        let day: u8 = value.get(8..10).and_then(|s| s.parse().ok()).unwrap_or(0);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err("Invalid birthday; expected YYYY-MM-DD".to_string());
        }
    }
    Ok(())
}

/// Set one or more typed person fields with **manual** provenance (slice 2b).
///
/// Writes to the authoritative graph in one transaction with manual provenance
/// — never to the legacy people-table columns, which are not the response
/// source (Decision A). The response is the re-overlaid
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
    // Validate the whole batch before opening the all-or-nothing write.
    for (name, value) in &req.fields {
        validate_person_field(name, value).map_err(|message| (StatusCode::BAD_REQUEST, message))?;
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

    brain
        .set_manual_entity_fields(entity_id, req.fields.clone().into_iter().collect())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
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

#[derive(Debug, Serialize)]
pub struct PersonRelationship {
    from_entity_uuid: String,
    to_entity_uuid: String,
    predicate: String,
    other_person: Person,
}

#[derive(Debug, Deserialize)]
pub struct AddRelationshipRequest {
    target_entity_uuid: String,
    predicate: String,
}

async fn person_and_brain(
    state: &AppState,
    uuid: &str,
) -> Result<(Person, SafeBrain, String), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let person = people::get_by_uuid(&pool, uuid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Person not found".into()))?;
    let graph_id = person
        .graph_entity_id
        .clone()
        .ok_or((StatusCode::CONFLICT, "Person has no graph identity".into()))?;
    let brain = state
        .brain
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Brain unavailable".into()))?;
    Ok((person, brain, graph_id))
}

async fn list_relationships_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Result<Json<Vec<PersonRelationship>>, (StatusCode, String)> {
    let (_, brain, graph_id) = person_and_brain(&state, &uuid).await?;
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let people_rows = people::list_people(&pool, &PeopleFilter::default())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let by_graph: HashMap<String, Person> = people_rows
        .into_iter()
        .filter_map(|p| p.graph_entity_id.clone().map(|id| (id, p)))
        .collect();
    let rows = brain
        .person_edges(&graph_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        rows.into_iter()
            .filter_map(|edge| {
                let other_id = if edge.from_id == graph_id {
                    &edge.to_id
                } else {
                    &edge.from_id
                };
                let other = by_graph.get(other_id)?.clone();
                let from_uuid = if edge.from_id == graph_id {
                    uuid.clone()
                } else {
                    other.entity_uuid.clone()
                };
                let to_uuid = if edge.to_id == graph_id {
                    uuid.clone()
                } else {
                    other.entity_uuid.clone()
                };
                Some(PersonRelationship {
                    from_entity_uuid: from_uuid,
                    to_entity_uuid: to_uuid,
                    predicate: edge.predicate,
                    other_person: other,
                })
            })
            .collect(),
    ))
}

async fn add_relationship_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Json(req): Json<AddRelationshipRequest>,
) -> Result<(StatusCode, Json<PersonRelationship>), (StatusCode, String)> {
    let (source, brain, source_id) = person_and_brain(&state, &uuid).await?;
    let (target, _, target_id) = person_and_brain(&state, &req.target_entity_uuid).await?;
    if source.entity_uuid == target.entity_uuid {
        return Err((
            StatusCode::BAD_REQUEST,
            "A person cannot relate to themselves".into(),
        ));
    }
    let inserted = brain
        .upsert_person_edge(&source_id, &target_id, &req.predicate)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let status = if inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(PersonRelationship {
            from_entity_uuid: source.entity_uuid,
            to_entity_uuid: target.entity_uuid.clone(),
            predicate: req.predicate.trim().to_string(),
            other_person: target,
        }),
    ))
}

async fn delete_relationship_handler(
    State(state): State<Arc<AppState>>,
    Path((uuid, target_uuid, predicate)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let (_, _, from_id) = person_and_brain(&state, &uuid).await?;
    let (_, _, to_id) = person_and_brain(&state, &target_uuid).await?;
    let db = permagent::config::paths::Paths::brain_dir().join("graph.sqlite");
    let deleted = tokio::task::spawn_blocking(move || {
        permagent::project_graph::delete_graph_triple(&db, &from_id, &to_id, &predicate)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if deleted == 0 {
        Ok(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

#[derive(Debug, Serialize)]
pub struct PersonActivity {
    id: String,
    kind: String,
    title: String,
    detail: String,
    timestamp: String,
}

/// Escape SQL LIKE wildcards so a literal string matches literally.
/// Pair with `ESCAPE '\'` in the query.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// True if `needle` occurs in `haystack` as a whole word (case-insensitive),
/// where word chars are alphanumeric; punctuation/space/start/end are boundaries.
fn contains_whole_word_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }

    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    haystack.match_indices(&needle).any(|(start, matched)| {
        let end = start + matched.len();
        let before_is_boundary = haystack
            .get(..start)
            .and_then(|prefix| prefix.chars().next_back())
            .is_none_or(|c| !c.is_alphanumeric());
        let after_is_boundary = haystack
            .get(end..)
            .and_then(|suffix| suffix.chars().next())
            .is_none_or(|c| !c.is_alphanumeric());
        before_is_boundary && after_is_boundary
    })
}

async fn person_activity_handler(
    State(state): State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Result<Json<Vec<PersonActivity>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let person = people::get_by_uuid(&pool, &uuid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Person not found".into()))?;
    let (note_rows, card_rows) = if person.display_name.trim().is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let needle = format!("%{}%", escape_like(&person.display_name));
        let note_rows = sqlx::query(
            "SELECT id, title, body, created_at FROM project_notes WHERE title LIKE ? ESCAPE '\\' OR body LIKE ? ESCAPE '\\'",
        )
        .bind(&needle)
        .bind(&needle)
        .fetch_all(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let card_rows = sqlx::query("SELECT id, title, description, updated_at FROM cards WHERE title LIKE ? ESCAPE '\\' OR description LIKE ? ESCAPE '\\'")
            .bind(&needle).bind(&needle).fetch_all(&pool).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        (note_rows, card_rows)
    };
    let mut items: Vec<PersonActivity> = note_rows
        .into_iter()
        .filter(|r| {
            r.get::<Option<String>, _>("title")
                .is_some_and(|title| contains_whole_word_ci(&title, &person.display_name))
                || contains_whole_word_ci(&r.get::<String, _>("body"), &person.display_name)
        })
        .map(|r| PersonActivity {
            id: format!("note-{}", r.get::<String, _>("id")),
            kind: "note".into(),
            title: r
                .get::<Option<String>, _>("title")
                .unwrap_or_else(|| "Note added".into()),
            detail: r.get("body"),
            timestamp: r.get("created_at"),
        })
        .chain(
            card_rows
                .into_iter()
                .filter(|r| {
                    contains_whole_word_ci(&r.get::<String, _>("title"), &person.display_name)
                        || contains_whole_word_ci(
                            &r.get::<String, _>("description"),
                            &person.display_name,
                        )
                })
                .map(|r| PersonActivity {
                    id: format!("card-{}", r.get::<String, _>("id")),
                    kind: "task".into(),
                    title: r.get("title"),
                    detail: r.get("description"),
                    timestamp: r.get("updated_at"),
                }),
        )
        .collect();

    let display = person.display_name;
    let memories = tokio::task::spawn_blocking(move || -> Result<Vec<PersonActivity>, String> {
        let conn = crate::brain_ops::read_only_brain_conn().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT m.id, m.content, m.description, m.created_at, a.who FROM memories m JOIN memory_annotations a ON a.memory_id=m.id WHERE a.who IS NOT NULL").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,Option<String>>(2)?, r.get::<_,Option<String>>(3)?, r.get::<_,String>(4)?))).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for row in rows.flatten() {
            let refs: Vec<serde_json::Value> = serde_json::from_str(&row.4).unwrap_or_default();
            let mentioned = refs.iter().any(|v| v.get("display_name").and_then(|v| v.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(&display)));
            if mentioned && seen.insert(row.0.clone()) { out.push(PersonActivity { id: format!("memory-{}", row.0), kind: "memory".into(), title: row.2.unwrap_or_else(|| "Memory linked".into()), detail: row.1, timestamp: row.3.unwrap_or_default() }); }
        }
        Ok(out)
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
      .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    items.extend(memories);
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    items.truncate(20);
    Ok(Json(items))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/people", get(list_people_handler))
        .route("/api/people/{id}/fields", patch(set_person_fields_handler))
        .route(
            "/api/people/{id}/relationships",
            get(list_relationships_handler).post(add_relationship_handler),
        )
        .route(
            "/api/people/{id}/relationships/{target_id}/{predicate}",
            delete(delete_relationship_handler),
        )
        .route("/api/people/{id}/activity", get(person_activity_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serial_test::serial;
    use tower::ServiceExt;

    #[test]
    fn escapes_like_metacharacters() {
        assert_eq!(escape_like(r"100%_safe\name"), r"100\%\_safe\\name");
        assert_eq!(escape_like("ordinary chars"), "ordinary chars");
    }

    #[test]
    fn matches_names_as_case_insensitive_whole_words() {
        assert!(contains_whole_word_ci("Meeting with Jon today", "jon"));
        assert!(!contains_whole_word_ci("Call Jonathan re: budget", "Jon"));
        assert!(!contains_whole_word_ci("SAMSUNG order", "Sam"));
        assert!(contains_whole_word_ci("Note about Jo_", "jo_"));
        assert!(!contains_whole_word_ci("Note about Jo_x", "Jo_"));
        assert!(contains_whole_word_ci("Jon starts the note", "jon"));
        assert!(contains_whole_word_ci("The note ends with JON", "jon"));
        assert!(!contains_whole_word_ci("Anything", ""));
    }

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

    #[test]
    fn rejects_invalid_person_field_values() {
        assert!(validate_person_field("email", "not-an-email").is_err());
        assert!(validate_person_field("birthday", "January sometime").is_err());
        assert!(validate_person_field("birthday", "1990-13-01").is_err());
        assert!(
            validate_person_field("notes", &"x".repeat(MAX_PERSON_FIELD_VALUE_LEN + 1)).is_err()
        );
        assert!(validate_person_field("company", "Acme\nInjected").is_err());
        assert!(validate_person_field("email", "jane@example.com").is_ok());
        assert!(validate_person_field("birthday", "1990-01-31").is_ok());
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

    async fn get_status(uri: &str) -> StatusCode {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        routes(state)
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn relationship_and_activity_routes_are_mounted() {
        let missing = "00000000-0000-0000-0000-000000000000";
        assert_eq!(
            get_status(&format!("/api/people/{missing}/relationships")).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_status(&format!("/api/people/{missing}/activity")).await,
            StatusCode::NOT_FOUND
        );
    }
}
