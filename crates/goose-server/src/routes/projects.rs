//! Project routes for managing user projects.
//!
//! Endpoints:
//!   GET    /api/projects               — List projects (optional ?status= filter)
//!   GET    /api/projects/:id           — Get single project with tags
//!   POST   /api/projects               — Create a new project
//!   PATCH  /api/projects/:id           — Update project fields
//!   DELETE /api/projects/:id           — Hard delete (403 for Personal)
//!   POST   /api/projects/:id/touch     — Update last_opened_at
//!   GET    /api/projects/:id/tags      — List tags
//!   POST   /api/projects/:id/tags      — Add a tag
//!   DELETE /api/projects/:id/tags/:tag — Remove a tag
//!   GET    /api/projects/:id/people             — People associated with the project
//!   POST   /api/projects/:id/people             — Associate a person ({entityUuid, role?})
//!   DELETE /api/projects/:id/people/:entity_uuid — Disassociate a person
//!   GET    /api/projects/:id/memories            — Memories associated (resolved from live Brain)
//!   POST   /api/projects/:id/memories/:memory_id — Associate a Brain memory
//!   DELETE /api/projects/:id/memories/:memory_id — Disassociate a memory
//!   GET    /api/projects/:id/notes               — List a project's notes
//!   POST   /api/projects/:id/notes               — Create a note ({title?, body}), indexed into the Brain
//!   DELETE /api/projects/:id/notes/:note_id      — Delete a note
//!   POST   /api/projects/:id/index-code          — Parse the project's codebase into a Brain code map
//!   GET    /api/projects/:id/stack               — List the project's stack entries (#512)
//!   GET    /api/projects/:id/stack/detected      — Discover declared stack from connected code
//!   POST   /api/projects/:id/stack               — Add a stack entry ({serviceName, category?, identity?, notes?, dashboardUrl?})
//!   PATCH  /api/projects/:id/stack/:entry_id     — Edit a stack entry (double-Option clears for identity/dashboardUrl)
//!   DELETE /api/projects/:id/stack/:entry_id     — Remove a stack entry
//!   GET    /api/projects/:id/intel               — Cited ecosystem/competitive intelligence
//!   GET    /api/projects/:id/market              — Market series, forecasts, and their methods
//!   DELETE /api/projects/:id/intel/:item_id       — Dismiss an intelligence item
//!
//! The stack endpoints are REFERENCE-ONLY (#512): they carry the service +
//! which login identity is used, never a password/secret — no such field is
//! accepted or stored.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, patch, post},
    Json, Router,
};
use permagent::agents::platform_extensions::analyze;
use permagent::events;
use permagent::project_association::{self, ProjectPerson};
use permagent::project_documents::{self, ProjectDocument};
use permagent::project_notes::{self, ProjectNote};
use permagent::project_stack::{self, StackEntry, UpdateStackEntry};
use permagent::projects::{self, PERSONAL_PROJECT_ID};
use permagent::reader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::fs;
use tokio_util::io::ReaderStream;

/// Per-file upload ceiling for project documents (mirrors the attachments cap).
const MAX_DOCUMENT_SIZE: usize = 50 * 1024 * 1024; // 50 MB

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectResponse {
    id: String,
    slug: String,
    name: String,
    description: String,
    status: String,
    root_path: Option<String>,
    site_url: Option<String>,
    repo_url: Option<String>,
    notes: String,
    /// General project metadata bag (schema v26). Known keys: `build_command`
    /// / `build_timeout_secs` — the project-level default completion check —
    /// and `publish_sequence` — ordered post-push steps required before a
    /// change is live (#457).
    metadata_json: serde_json::Value,
    tags: Vec<String>,
    created_at: String,
    updated_at: String,
    last_opened_at: String,
}

impl From<projects::Project> for ProjectResponse {
    fn from(p: projects::Project) -> Self {
        Self {
            id: p.id,
            slug: p.slug,
            name: p.name,
            description: p.description,
            status: p.status,
            root_path: p.root_path,
            site_url: p.site_url,
            repo_url: p.repo_url,
            notes: p.notes,
            metadata_json: p.metadata_json,
            tags: p.tags,
            created_at: p.created_at,
            updated_at: p.updated_at,
            last_opened_at: p.last_opened_at,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    name: String,
    slug: Option<String>,
    description: Option<String>,
    root_path: Option<String>,
    site_url: Option<String>,
    repo_url: Option<String>,
    notes: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectRequest {
    name: Option<String>,
    slug: Option<String>,
    description: Option<String>,
    status: Option<String>,
    root_path: Option<Option<String>>,
    site_url: Option<Option<String>>,
    repo_url: Option<Option<String>>,
    notes: Option<String>,
    /// Full replacement of the project metadata bag (JSON object).
    metadata_json: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ListProjectsQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTagRequest {
    tag: String,
}

#[derive(Serialize)]
pub struct DeleteResponse {
    deleted: bool,
}

#[derive(Serialize)]
pub struct TouchResponse {
    touched: bool,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ProjectIntelItem {
    id: String,
    kind: String,
    name: String,
    note: Option<String>,
    source_url: String,
    created_at: String,
}

#[derive(Serialize)]
pub struct ProjectIntelResponse {
    competitors: Vec<ProjectIntelItem>,
    partners: Vec<ProjectIntelItem>,
    ecosystem: Vec<ProjectIntelItem>,
}

async fn list_project_intel_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectIntelResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let rows = sqlx::query_as::<_, ProjectIntelItem>(
        "SELECT id, kind, name, note, source_url, created_at
         FROM project_intel WHERE project_id = ? ORDER BY created_at DESC, name",
    )
    .bind(&project.id)
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = ProjectIntelResponse {
        competitors: Vec::new(),
        partners: Vec::new(),
        ecosystem: Vec::new(),
    };
    for row in rows {
        match row.kind.as_str() {
            "competitor" => response.competitors.push(row),
            "partner" => response.partners.push(row),
            "adjacent" => response.ecosystem.push(row),
            _ => {}
        }
    }
    Ok(Json(response))
}

/// One row of the Market card: what the series is, how much of it exists, and
/// either a forecast carrying the method that produced it, or the reason there
/// is not one. Never both, and never neither.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMarketRow {
    #[serde(flatten)]
    summary: permagent::forecaster::store::SeriesSummary,
    /// The tail of the series, for the sparkline. Capped, so a three-year daily
    /// series does not ship a thousand floats to draw sixty pixels.
    history: Vec<f64>,
    forecast: Option<permagent::forecaster::forecast::Forecast>,
    refusal: Option<permagent::forecaster::forecast::Refusal>,
    /// Direction as a sentence, derived here so the card and the brief cannot
    /// disagree about the same numbers.
    direction: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMarketResponse {
    rows: Vec<ProjectMarketRow>,
    /// True when nothing is bound at all — which is NOT the same as a flat
    /// market. The nonprofit and community projects have no real market signal,
    /// and the card says so rather than drawing a line through zero.
    no_series_bound: bool,
    generated_at: String,
}

const MARKET_SPARKLINE_POINTS: usize = 90;

/// Direction from the numbers only: no cause, no advice.
fn describe_market_direction(history: &[f64], point: &[f64]) -> Option<String> {
    let last = *history.last()?;
    let end = *point.last()?;
    if !last.is_finite() || !end.is_finite() || last.abs() < f64::EPSILON {
        return None;
    }
    let pct = (end - last) / last.abs() * 100.0;
    if pct.abs() < 2.0 {
        return Some(format!("flat over the next {}", plural_steps(point.len())));
    }
    Some(format!(
        "{} {:.0}% over the next {}",
        if pct > 0.0 { "up" } else { "down" },
        pct.abs(),
        plural_steps(point.len())
    ))
}

fn plural_steps(n: usize) -> String {
    if n == 1 {
        "step".to_string()
    } else {
        format!("{n} steps")
    }
}

/// The Market card's data. Every row carries its own verdict, so a series that
/// cannot be forecast renders its reason rather than an empty chart.
async fn project_market_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectMarketResponse>, StatusCode> {
    use permagent::forecaster::{forecast, store};

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let now = chrono::Utc::now();
    let summaries = store::summarize(&pool, Some(&project.id), now)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut rows = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let points = store::load_points(&pool, &summary.series_id)
            .await
            .unwrap_or_default();
        let history: Vec<f64> = points
            .iter()
            .rev()
            .take(MARKET_SPARKLINE_POINTS)
            .rev()
            .map(|p| p.value)
            .collect();
        let (forecast, refusal) =
            match forecast::forecast_series(&pool, &summary.series_id, None, now).await {
                Ok(f) => (Some(f), None),
                Err(r) => (None, Some(r)),
            };
        let direction = forecast
            .as_ref()
            .and_then(|f| describe_market_direction(&history, &f.point));
        rows.push(ProjectMarketRow {
            summary,
            history,
            forecast,
            refusal,
            direction,
        });
    }

    Ok(Json(ProjectMarketResponse {
        no_series_bound: rows.is_empty(),
        rows,
        generated_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    }))
}

async fn delete_project_intel_handler(
    State(state): State<Arc<AppState>>,
    Path((id, item_id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let result = sqlx::query("DELETE FROM project_intel WHERE id = ? AND project_id = ?")
        .bind(&item_id)
        .bind(&project.id)
        .execute(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_projects_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Json<Vec<ProjectResponse>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = projects::list_projects(&pool, query.status.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items.into_iter().map(ProjectResponse::from).collect()))
}

async fn get_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(ProjectResponse::from(project)))
}

async fn create_project_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let input = projects::CreateProject {
        name: req.name,
        slug: req.slug,
        description: req.description,
        root_path: req.root_path,
        site_url: req.site_url,
        repo_url: req.repo_url,
        notes: req.notes,
        tags: req.tags,
    };
    let project = projects::create_project(&pool, input)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    // #629 multi-client liveness: the `project_changed` frame is emitted by
    // `projects::create_project` — the writer `platform__project(create)` also
    // goes through — so both paths announce identically and neither doubles.
    Ok((StatusCode::CREATED, Json(ProjectResponse::from(project))))
}

async fn update_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    if project.id == PERSONAL_PROJECT_ID && (req.slug.is_some() || req.status.is_some()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot change slug or status of the Personal project".to_string(),
        ));
    }
    let input = projects::UpdateProject {
        name: req.name,
        slug: req.slug,
        description: req.description,
        status: req.status,
        root_path: req.root_path,
        site_url: req.site_url,
        repo_url: req.repo_url,
        notes: req.notes,
        metadata_json: req.metadata_json,
    };
    let updated = projects::update_project(&pool, &project.id, input)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    // #629: `projects::update_project` emits `project_changed(updated)`.
    Ok(Json(ProjectResponse::from(updated)))
}

async fn delete_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    if project.id == PERSONAL_PROJECT_ID {
        return Err((
            StatusCode::FORBIDDEN,
            "Cannot delete the Personal project".to_string(),
        ));
    }
    let deleted = projects::delete_project(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    // `projects::delete_project` emits `project_changed(deleted)`.
    Ok(Json(DeleteResponse { deleted }))
}

async fn touch_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<TouchResponse>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let touched = projects::touch_project(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if touched {
        // `projects::touch_project` emits `project_changed(touched)`.
        Ok(Json(TouchResponse { touched }))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn list_tags_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = projects::get_project(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tags = projects::list_tags(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tags))
}

async fn add_tag_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AddTagRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let ok = projects::add_tag(&pool, &id, &req.tag)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if ok {
        // `projects::add_tag` emits `project_changed(tags)`.
        Ok(StatusCode::CREATED)
    } else {
        Err((StatusCode::NOT_FOUND, "Project not found".to_string()))
    }
}

async fn remove_tag_handler(
    State(state): State<Arc<AppState>>,
    Path((id, tag)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let removed = projects::remove_tag(&pool, &id, &tag)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if removed {
        // `projects::remove_tag` emits `project_changed(tags)`.
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ── Project association: people ──────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssociatePersonRequest {
    /// Opaque `people.entity_uuid` to associate.
    entity_uuid: String,
    /// Optional role within this project (distinct from the person's CRM role).
    #[serde(default)]
    role: Option<String>,
}

/// GET /api/projects/{id}/people — people associated with a project.
async fn list_project_people_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ProjectPerson>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let mut people = project_association::list_project_people(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    // Attributes come from the graph (Decision A, #255); project_role/associated_at
    // stay from the join. Overlay onto each row's inner Person.
    crate::routes::people::overlay_graph_attributes(
        state.brain.as_ref(),
        people.iter_mut().map(|pp| &mut pp.person).collect(),
    )
    .await;
    if let Ok(latest) = permagent::person_meetings::latest_starts_by_person(&pool).await {
        for pp in &mut people {
            permagent::person_meetings::merge_last_contact(
                std::slice::from_mut(&mut pp.person),
                &latest,
            );
        }
    }
    // Dag K owner-identity (2026-09-26): the owner's own record can be
    // associated with a project like any contact, and this list must render it
    // as "You" too — the People → List directory is not the only surface that
    // reads `person.is_self`.
    let owner = permagent::config::agent_identity::load_owner_identity();
    for pp in &mut people {
        permagent::people::overlay_owner_self(std::slice::from_mut(&mut pp.person), &owner);
    }
    Ok(Json(people))
}

/// GET /api/projects/{id}/meetings — 1:1s logged against this project.
async fn list_project_meetings_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<permagent::person_meetings::PersonMeetingWithName>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let rows = permagent::person_meetings::list_for_project(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows))
}

/// POST /api/projects/{id}/people — associate a person with a project.
async fn associate_person_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AssociatePersonRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    // FK violation (unknown entity_uuid) surfaces as a 400, not a 500.
    project_association::associate_person(
        &pool,
        &project.id,
        &req.entity_uuid,
        req.role.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // #495 slice 3 (+ #595): mirror the association as a works_on graph edge,
    // best-effort. The project_people row is the source of truth and the graph
    // may lag behind it (people-in-graph v1) — a skip or error here is logged,
    // never a request failure. #595 closes the non-ontology gap: the project's
    // graph identity is resolved (ontology) or minted (runtime, provenance-first)
    // before the edge is asserted, so Projects-tab projects get the edge too.
    if let Some(brain) = state.brain.as_ref() {
        match permagent::people::get_by_uuid(&pool, &req.entity_uuid).await {
            Ok(Some(person)) => {
                if let Some(graph_id) = person.graph_entity_id.as_deref() {
                    match permagent::project_graph::ensure_project_graph_identity(
                        &pool, brain, &project,
                    )
                    .await
                    {
                        Ok(Some(project_gid)) => {
                            match brain.assert_works_on_edge(graph_id, &project_gid).await {
                                Ok(true) => tracing::info!(
                                    project = %project.id, person = %req.entity_uuid,
                                    "graph works_on edge asserted"
                                ),
                                Ok(false) => tracing::debug!(
                                    project = %project.id, person = %req.entity_uuid,
                                    "graph works_on edge skipped (graph lagging)"
                                ),
                                Err(e) => tracing::warn!(
                                    project = %project.id, person = %req.entity_uuid, error = %e,
                                    "graph works_on edge assert failed"
                                ),
                            }
                        }
                        Ok(None) => tracing::debug!(
                            project = %project.id,
                            "project has no graph identity (name empty after normalization)"
                        ),
                        Err(e) => tracing::warn!(
                            project = %project.id, error = %e,
                            "project graph identity resolution failed"
                        ),
                    }
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(
                person = %req.entity_uuid, error = %e,
                "person lookup for graph edge failed"
            ),
        }
    }
    // The `person_changed(.., "associated")` frame is emitted by
    // `project_association::associate_person` itself, so the agent tool path
    // announces it too. Do not re-emit here.
    Ok(StatusCode::CREATED)
}

/// DELETE /api/projects/{id}/people/{entity_uuid} — disassociate a person.
///
/// #595: also deletes the mirrored `works_on` graph triple, best-effort — the
/// association row is the source of truth and its deletion never fails on
/// graph state. The person and project *nodes* are identity, not residue, and
/// are left in place.
async fn disassociate_person_handler(
    State(state): State<Arc<AppState>>,
    Path((id, entity_uuid)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Resolve id-or-slug like the sibling handlers (associate/list), so the
    // graph cleanup sees the project row; the association delete then uses the
    // canonical project id.
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let removed = project_association::disassociate_person(&pool, &project.id, &entity_uuid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !removed {
        return Err((StatusCode::NOT_FOUND, "Association not found".to_string()));
    }
    // `disassociate_person` emits the `person_changed(.., "disassociated")`
    // frame itself — one writer, one announcement.

    // #595: best-effort works_on residue cleanup. Read-only identity
    // resolution (never creates nodes on a delete path), then a scoped
    // triple delete against the graph store.
    if let Some(brain) = state.brain.as_ref() {
        match permagent::people::get_by_uuid(&pool, &entity_uuid).await {
            Ok(Some(person)) => {
                if let Some(person_gid) = person.graph_entity_id {
                    let candidates =
                        permagent::project_graph::project_graph_id_candidates(brain, &project)
                            .await;
                    if !candidates.is_empty() {
                        let graph_db =
                            permagent::config::paths::Paths::brain_dir().join("graph.sqlite");
                        let result = tokio::task::spawn_blocking(move || {
                            permagent::project_graph::delete_works_on_triples(
                                &graph_db,
                                &person_gid,
                                &candidates,
                            )
                        })
                        .await;
                        match result {
                            Ok(Ok(n)) if n > 0 => tracing::info!(
                                project = %project.id, person = %entity_uuid, deleted = n,
                                "graph works_on edge deleted on disassociate"
                            ),
                            Ok(Ok(_)) => tracing::debug!(
                                project = %project.id, person = %entity_uuid,
                                "no graph works_on residue to delete"
                            ),
                            Ok(Err(e)) => tracing::warn!(
                                project = %project.id, person = %entity_uuid, error = %e,
                                "graph works_on edge delete failed"
                            ),
                            Err(e) => tracing::warn!(
                                project = %project.id, person = %entity_uuid, error = %e,
                                "graph works_on edge delete task panicked"
                            ),
                        }
                    }
                }
            }
            Ok(None) => {} // person already deleted from CRM; nothing to key the cleanup on
            Err(e) => tracing::warn!(
                person = %entity_uuid, error = %e,
                "person lookup for graph cleanup failed"
            ),
        }
    }
    Ok(StatusCode::OK)
}

// ── Project association: memories ────────────────────────────────────────────

/// A project-scoped memory: live Brain content resolved from `memory.db`, plus
/// the association timestamp. `id` is the Spectral memory id.
#[derive(Serialize)]
pub struct ProjectMemory {
    id: String,
    key: String,
    content: String,
    description: Option<String>,
    signal_score: f64,
    created_at: String,
    associated_at: String,
}

/// GET /api/projects/{id}/memories — memories associated with a project.
///
/// Reads the join rows from permagent.db, then resolves each Spectral id against
/// the LIVE Brain (`read_only_brain_conn`) — never the dead permagent.db
/// `memories` table. Orphan ids (memory deleted in Spectral) are silently dropped
/// (effective INNER JOIN). Order follows association recency.
async fn list_project_memories_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ProjectMemory>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let assocs = project_association::list_project_memory_associations(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if assocs.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let resolved = tokio::task::spawn_blocking(move || -> Result<Vec<ProjectMemory>, String> {
        let conn = crate::brain_ops::read_only_brain_conn().map_err(|e| e.to_string())?;
        let ids: Vec<String> = assocs.iter().map(|a| a.memory_id.clone()).collect();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, key, content, description, signal_score, created_at \
             FROM memories WHERE id IN ({})",
            placeholders
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params = rusqlite::params_from_iter(ids.iter());
        let mut rows = stmt.query(params).map_err(|e| e.to_string())?;

        // Index resolved rows by id, then emit in association order so the
        // newest association renders first (and orphans drop out).
        let mut by_id: std::collections::HashMap<
            String,
            (String, String, Option<String>, f64, String),
        > = std::collections::HashMap::new();
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let mid: String = r.get(0).map_err(|e| e.to_string())?;
            by_id.insert(
                mid,
                (
                    r.get(1).map_err(|e| e.to_string())?,
                    r.get(2).map_err(|e| e.to_string())?,
                    r.get(3).map_err(|e| e.to_string())?,
                    r.get(4).map_err(|e| e.to_string())?,
                    r.get(5).map_err(|e| e.to_string())?,
                ),
            );
        }

        Ok(assocs
            .into_iter()
            .filter_map(|a| {
                by_id
                    .get(&a.memory_id)
                    .map(|(key, content, desc, sig, created)| ProjectMemory {
                        id: a.memory_id.clone(),
                        key: key.clone(),
                        content: content.clone(),
                        description: desc.clone(),
                        signal_score: *sig,
                        created_at: created.clone(),
                        associated_at: a.added_at.clone(),
                    })
            })
            .collect())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(resolved))
}

/// POST /api/projects/{id}/memories/{memory_id} — associate a memory.
async fn associate_memory_handler(
    State(state): State<Arc<AppState>>,
    Path((id, memory_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    project_association::associate_memory(&pool, &project.id, &memory_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    events::emit(events::project_changed(&project.id, "memories"));
    Ok(StatusCode::CREATED)
}

/// DELETE /api/projects/{id}/memories/{memory_id} — disassociate a memory.
async fn disassociate_memory_handler(
    State(state): State<Arc<AppState>>,
    Path((id, memory_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let removed = project_association::disassociate_memory(&pool, &id, &memory_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if removed {
        events::emit(events::project_changed(&id, "memories"));
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "Association not found".to_string()))
    }
}

// ── Project documents: the document hub + in-app viewer (#471 Layer 2) ───────

/// GET /api/projects/{id}/documents — documents attached to a project.
async fn list_project_documents_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ProjectDocument>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let docs = project_documents::list_documents(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(docs))
}

#[derive(Serialize)]
struct UploadDocumentsResponse {
    documents: Vec<ProjectDocument>,
}

/// Save one file as a project document: write the bytes to disk under
/// `~/.permagent/project-docs/<project_id>/<doc_id>/<doc_id>`, insert the
/// `project_documents` row, and best-effort index the extracted text into the
/// Brain scoped to the project. Factored out of the multipart upload handler so
/// the inbox routing slice (#395) shares the exact same write path — behavior
/// (logging, size cap, never-fail Brain enrichment) unchanged.
pub(crate) async fn save_project_document(
    state: &Arc<AppState>,
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project: &projects::Project,
    filename: &str,
    mime_type: &str,
    data: &[u8],
) -> Result<ProjectDocument, (StatusCode, String)> {
    if data.len() > MAX_DOCUMENT_SIZE {
        tracing::warn!(project = %project.id, %filename, size = data.len(), "project document exceeds size cap");
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("{filename} exceeds the {MAX_DOCUMENT_SIZE}-byte limit"),
        ));
    }

    let docs_base = permagent::config::paths::Paths::in_data_dir("project-docs").join(&project.id);

    let doc_id = uuid::Uuid::now_v7().to_string();
    let dir = docs_base.join(&doc_id);
    fs::create_dir_all(&dir).await.map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document dir create failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    let file_path = dir.join(&doc_id);
    let canonical_docs_base = fs::canonicalize(&docs_base).await.map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document base canonicalization failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    let canonical_parent = fs::canonicalize(file_path.parent().expect("document path has parent"))
        .await
        .map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document dir canonicalization failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    if !canonical_parent.starts_with(&canonical_docs_base) {
        tracing::error!(project = %project.id, %filename, "project document path escaped storage root");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid project document path".to_string(),
        ));
    }
    let file_path = canonical_parent.join(
        file_path
            .file_name()
            .expect("generated document path has basename"),
    );
    fs::write(&file_path, data).await.map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document write failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    let size_bytes = data.len() as i64;
    let path_str = file_path.to_string_lossy().to_string();
    let uploaded_at = project_documents::insert_document(
        pool, &doc_id, &project.id, filename, mime_type, size_bytes, &path_str,
    )
    .await
    .map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document insert failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    })?;

    tracing::info!(project = %project.id, %filename, %mime_type, size = size_bytes, "project document uploaded");

    // Best-effort Brain index: extract the file's text LOCALLY (mirroring the
    // Reader ingest route's image-vs-document decision), store the full text
    // durably in the Brain, and associate that memory with this project — so a
    // dropped document becomes recallable + Librarian-enriched, scoped to the
    // project it landed in. Every failure is logged and skipped; indexing is
    // enrichment on top of the durable file + row, and must NEVER fail the
    // upload. Guarded on the Brain being ready. Visual images (OCR finds too
    // little text) are not indexed — there is no text to recall.
    if let Some(brain) = state.brain.as_ref() {
        let ingest = if mime_type.starts_with("image/") {
            reader::ingest_image(data, filename).await
        } else {
            reader::ingest_document(data, filename, mime_type).await
        };
        match ingest {
            Ok(digest) if !digest.is_visual => {
                match brain.get_memory_by_key(&digest.memory_key).await {
                    Ok(Some(mem)) => {
                        if let Err(e) =
                            project_association::associate_memory(pool, &project.id, &mem.id).await
                        {
                            tracing::warn!(project = %project.id, %filename, error = %e, "project document brain association failed");
                        } else {
                            tracing::info!(project = %project.id, %filename, memory = %mem.id, "project document indexed into brain");
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(project = %project.id, %filename, key = %digest.memory_key, "project document ingested but its memory was not found for association")
                    }
                    Err(e) => {
                        tracing::warn!(project = %project.id, %filename, error = %e, "project document memory lookup failed")
                    }
                }
            }
            Ok(_) => {
                tracing::debug!(project = %project.id, %filename, "project document classified visual; text not indexed")
            }
            Err(e) => {
                tracing::warn!(project = %project.id, %filename, error = %e, "project document brain ingest failed (upload still succeeds)")
            }
        }
    }

    Ok(ProjectDocument {
        id: doc_id,
        project_id: project.id.clone(),
        filename: filename.to_string(),
        mime_type: mime_type.to_string(),
        size_bytes,
        path: path_str,
        uploaded_at,
    })
}

/// POST /api/projects/{id}/documents — upload one or more files (multipart).
///
/// Mirrors the session attachment pipeline: each file is written to disk under
/// `~/.permagent/project-docs/<project_id>/<doc_id>/<doc_id>`, then a
/// `project_documents` row records it. Outcomes are logged (per the #568
/// empty-body lesson — a failure must be visible, not a silent catch); errors
/// surface as a non-2xx with a message body rather than a bare status.
async fn upload_project_documents_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<UploadDocumentsResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let mut results = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let mime_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let data = field.bytes().await.map_err(|e| {
            tracing::warn!(project = %project.id, %filename, error = %e, "project document read failed");
            (StatusCode::BAD_REQUEST, format!("read failed: {e}"))
        })?;

        let doc =
            save_project_document(&state, &pool, &project, &filename, &mime_type, &data).await?;
        results.push(doc);
    }

    // #629 one-writer rule: `project_documents::insert_document` emits
    // `project_changed(id, "documents")` itself (one frame per file — mirrors
    // `cards::create_card`, which emits per card too), so every caller of
    // `save_project_document` announces, including the inbox `project` route.
    // No emit here.
    Ok(Json(UploadDocumentsResponse { documents: results }))
}

/// GET /api/projects/{id}/documents/{doc_id} — stream an inline-safe document.
///
/// Only an explicit MIME allowlist is rendered inline. Everything else is
/// forced to a generic download, and every response disables MIME sniffing.
async fn get_project_document_handler(
    State(state): State<Arc<AppState>>,
    Path((id, doc_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let doc = project_documents::get_document(&pool, &project.id, &doc_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let file = tokio::fs::File::open(&doc.path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let (content_type, disposition) = document_serving_headers(&doc.mime_type);

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, disposition.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        body,
    ))
}

fn document_serving_headers(mime_type: &str) -> (String, &'static str) {
    match mime_type {
        "application/pdf" | "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        | "text/plain" => (mime_type.to_string(), "inline"),
        _ => ("application/octet-stream".to_string(), "attachment"),
    }
}

/// DELETE /api/projects/{id}/documents/{doc_id} — remove a document + its file.
async fn delete_project_document_handler(
    State(state): State<Arc<AppState>>,
    Path((id, doc_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let path = project_documents::delete_document(&pool, &project.id, &doc_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Document not found".to_string()))?;

    let _ = fs::remove_file(&path).await;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = fs::remove_dir(parent).await;
    }
    tracing::info!(project = %project.id, doc = %doc_id, "project document deleted");
    // #629 one-writer rule: `project_documents::delete_document` emits the
    // `project_changed(id, "documents")` frame itself. No emit here.
    Ok(StatusCode::NO_CONTENT)
}

// ── Project notes: freeform notes indexed into the Brain ─────────────────────

#[derive(Deserialize)]
struct CreateNoteRequest {
    title: Option<String>,
    body: String,
    /// Optional note kind. `"meeting"` marks a meeting transcript — after the
    /// note lands, a durable Orchestrator triage pass extracts action items
    /// onto the project's kanban (see `meeting_triage`). Absent/other kinds
    /// change nothing.
    #[serde(default)]
    kind: Option<String>,
    /// Client-stable recording identity — required for `kind: "meeting"`.
    #[serde(default, rename = "recordingId")]
    recording_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateNoteResponse {
    #[serde(flatten)]
    note: ProjectNote,
    #[serde(skip_serializing_if = "Option::is_none")]
    triage: Option<permagent::meeting_triage::MeetingTriageReceipt>,
}

/// GET /api/projects/{id}/notes — notes attached to a project, newest first.
async fn list_project_notes_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ProjectNote>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let notes = project_notes::list_notes(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let notes: Vec<_> = notes
        .into_iter()
        .map(|n| project_notes::with_export_path(n, &project.slug))
        .collect();
    Ok(Json(notes))
}

/// POST /api/projects/{id}/notes — create a note (`{title?, body}`).
///
/// Delegates to [`project_notes::create_note_indexed`] — the ONE composed note
/// path (row insert + best-effort Brain index with the Reader's `RememberOpts`
/// contract: distinct durable source, Private, NO description + project
/// association), shared with the `file_to_project` decision effect. The row is
/// the durable record; the note is returned even if the Brain write fails.
async fn create_project_note_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateNoteRequest>,
) -> Result<Json<CreateNoteResponse>, (StatusCode, String)> {
    if req.body.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "note body is empty".to_string()));
    }

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let is_meeting = req.kind.as_deref() == Some("meeting");
    let (note, triage) = if is_meeting {
        let (recording_id, idempotent_recording) =
            permagent::meeting_triage::resolve_recording_id(req.recording_id.as_deref());
        let saved = permagent::meeting_triage::save_meeting_note(
            &pool,
            state.brain.as_ref(),
            &project.id,
            &recording_id,
            idempotent_recording,
            req.title.as_deref(),
            &req.body,
        )
        .await
        .map_err(|e| match e {
            permagent::meeting_triage::MeetingSaveError::EmptyBody => {
                (StatusCode::BAD_REQUEST, e.message())
            }
            permagent::meeting_triage::MeetingSaveError::BodyConflict { .. } => {
                (StatusCode::CONFLICT, e.message())
            }
            permagent::meeting_triage::MeetingSaveError::Database(msg) => {
                tracing::error!(project = %project.id, error = %msg, "meeting note save failed");
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        })?;
        (saved.note, Some(saved.triage))
    } else {
        let note = project_notes::create_note_indexed(
            &pool,
            state.brain.as_ref(),
            &project.id,
            req.title.as_deref(),
            &req.body,
        )
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, error = %e, "project note create failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;
        (note, None)
    };

    if is_meeting {
        if triage.as_ref().is_some_and(|t| t.created) {
            project_notes::write_note_export(
                &project.slug,
                &project.name,
                &note,
                req.kind.as_deref(),
                None,
            )
            .await;
        }
        permagent::meeting_triage::spawn_drain(pool.clone(), state.brain.clone());
    } else {
        project_notes::write_note_export(
            &project.slug,
            &project.name,
            &note,
            req.kind.as_deref(),
            None,
        )
        .await;
    }
    let note = project_notes::with_export_path(note, &project.slug);

    tracing::info!(
        project = %project.id,
        note = %note.id,
        meeting = is_meeting,
        "project note created"
    );
    events::emit(events::project_changed(&project.id, "notes"));

    Ok(Json(CreateNoteResponse { note, triage }))
}

async fn get_meeting_triage_handler(
    State(state): State<Arc<AppState>>,
    Path((id, recording_id)): Path<(String, String)>,
) -> Result<Json<permagent::meeting_triage::MeetingTriageReceipt>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let triage = permagent::meeting_triage::get_triage_receipt(&pool, &project.id, &recording_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Meeting triage not found".to_string(),
        ))?;
    Ok(Json(triage))
}

async fn retry_meeting_triage_handler(
    State(state): State<Arc<AppState>>,
    Path((id, recording_id)): Path<(String, String)>,
) -> Result<Json<permagent::meeting_triage::MeetingTriageReceipt>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let triage = permagent::meeting_triage::retry_triage(&pool, &project.id, &recording_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Meeting triage not found".to_string(),
        ))?;
    permagent::meeting_triage::spawn_drain(pool, state.brain.clone());
    Ok(Json(triage))
}

async fn get_meeting_triage_session_handler(
    State(state): State<Arc<AppState>>,
    Path((id, recording_id)): Path<(String, String)>,
) -> Result<Json<permagent::meeting_triage::MeetingTriageSessionView>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let view =
        permagent::meeting_triage::get_triage_session_view(&pool, &project.id, &recording_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or((
                StatusCode::NOT_FOUND,
                "Meeting triage not found".to_string(),
            ))?;
    Ok(Json(view))
}

/// DELETE /api/projects/{id}/notes/{note_id} — delete a note (+ best-effort
/// disassociate its Brain memory). 200 on delete, 404 if no such note.
async fn delete_project_note_handler(
    State(state): State<Arc<AppState>>,
    Path((id, note_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let deleted = project_notes::delete_note(&pool, &project.id, &note_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Note not found".to_string()))?;

    // Best-effort: drop the project↔memory association for the note's memory.
    // The memory itself stays in the Brain (recall is content-addressed and the
    // Librarian may have enriched it); we only remove the project scoping.
    if let Some(key) = deleted {
        if let Some(brain) = state.brain.as_ref() {
            if let Ok(Some(mem)) = brain.get_memory_by_key(&key).await {
                if let Err(e) =
                    project_association::disassociate_memory(&pool, &project.id, &mem.id).await
                {
                    tracing::warn!(project = %project.id, note = %note_id, error = %e, "project note disassociate failed");
                }
            }
        }
    }

    tracing::info!(project = %project.id, note = %note_id, "project note deleted");
    events::emit(events::project_changed(&project.id, "notes"));
    Ok(StatusCode::OK)
}

// ── Project code index: parse a project's codebase into the Brain (#471) ─────

/// `source` tag on the code-map memory. Like project notes, deliberately NOT
/// `permagent.activity` (which pruning/consolidation reap) so the map is
/// durable, and the memory is written description-less so the Librarian claims +
/// enriches it exactly as it does ingested documents and notes.
const CODE_MAP_SOURCE: &str = "permagent.code";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexCodeResponse {
    indexed: bool,
    files: usize,
    memory_key: String,
}

/// Deterministic Brain key for a project's code map. One key per project —
/// re-indexing overwrites it (idempotent), and its EXISTENCE is the signal
/// "someone asked for this project to be indexed" that the post-landing
/// refresh keys off.
pub fn code_map_memory_key(project_id: &str) -> String {
    format!("code:{project_id}:map")
}

/// What a successful (re)index pass produced.
pub struct CodeIndexOutcome {
    pub files: usize,
    pub memory_key: String,
}

/// Why a (re)index pass failed. Carries the exact human-readable messages the
/// route has always returned, split by cause so the HTTP handler can keep its
/// status mapping while the post-landing hook only needs one warn line.
#[derive(Debug)]
pub enum CodeIndexError {
    /// `root_path` missing, blank, or not a readable directory (route: 400).
    BadRoot(String),
    /// The spawn_blocking parse task panicked (route: 500).
    ParsePanic(String),
    /// Nothing parseable under `root_path` (route: 400).
    NoSourceFiles,
    /// The Brain rejected the write (route: 500).
    BrainWrite(String),
}

impl std::fmt::Display for CodeIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRoot(msg) => write!(f, "{msg}"),
            Self::ParsePanic(e) => write!(f, "code parse task panicked: {e}"),
            Self::NoSourceFiles => {
                write!(f, "No source files could be parsed under root_path")
            }
            Self::BrainWrite(e) => write!(f, "brain write failed: {e}"),
        }
    }
}

impl CodeIndexError {
    /// The status mapping the route has always used — byte-identical messages.
    fn http(self) -> (StatusCode, String) {
        let status = match &self {
            Self::BadRoot(_) | Self::NoSourceFiles => StatusCode::BAD_REQUEST,
            Self::ParsePanic(_) | Self::BrainWrite(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string())
    }
}

/// The indexing core shared by POST /api/projects/{id}/index-code and the
/// post-landing refresh: validate `root_path`, run the tree-sitter pass
/// off-thread (CPU-bound — never on the async executor), write the rendered
/// map to the Brain under [`code_map_memory_key`], and best-effort associate
/// the memory with the project (logged, not fatal — the map is already durable
/// in the Brain once written).
pub async fn reindex_project_code(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    brain: &permagent::brain_handle::SafeBrain,
    project: &projects::Project,
) -> Result<CodeIndexOutcome, CodeIndexError> {
    let root_path = project
        .root_path
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| CodeIndexError::BadRoot("Project has no root_path to index".to_string()))?;
    let root = std::path::Path::new(&root_path).to_path_buf();
    if !root.is_dir() {
        return Err(CodeIndexError::BadRoot(format!(
            "root_path is not a readable directory: {root_path}"
        )));
    }

    // Tree-sitter parsing is CPU-bound (rayon) — never run it on the async
    // executor. Build the map off-thread. `max_depth = 0` = the whole tree
    // (WalkBuilder still skips .gitignore'd artifacts like node_modules/target).
    let notes_root = root.clone();
    let map = tokio::task::spawn_blocking(move || analyze::build_code_map(&root, 0))
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, error = %e, "code index parse task panicked");
            CodeIndexError::ParsePanic(e.to_string())
        })?;

    // Markdown/text project knowledge is useful even when tree-sitter finds no
    // source language. Run this before the NoSourceFiles decision so a notes-only
    // project still gets durable, project-scoped Brain memories.
    //
    // Unlike a chat turn (where the UI's open project is only a hypothesis
    // about what the conversation is about — see `permagent::session_wing`),
    // this ingest IS a property of a known project: `project` is the row
    // itself, so the slug needs no lookup. Skip the implicit "personal" project
    // the same way
    // `crate::wing_rules::project_wing_rules` and `project_notes::resolve_note_wing`
    // do: its wing would swallow anything mentioning the word "personal", so an
    // unscoped project should keep falling through to Spectral's classifier
    // instead of a literal "personal" wing.
    let wing =
        (project.slug != permagent::session_wing::PERSONAL_SLUG).then_some(project.slug.as_str());
    let text_digests = match reader::ingest_tree_for_project(&project.id, &notes_root, wing).await {
        Ok(digests) => digests,
        Err(e) => {
            tracing::warn!(project = %project.id, error = %e, "tree ingest of project notes failed");
            Vec::new()
        }
    };

    for digest in &text_digests {
        if digest.is_visual {
            continue;
        }
        if let Ok(Some(mem)) = brain.get_memory_by_key(&digest.memory_key).await {
            if let Err(e) = project_association::associate_memory(pool, &project.id, &mem.id).await
            {
                tracing::warn!(
                    project = %project.id,
                    key = %digest.memory_key,
                    error = %e,
                    "tree ingest association failed"
                );
            }
        }
    }

    if map.files == 0 {
        let Some(first) = text_digests.first() else {
            return Err(CodeIndexError::NoSourceFiles);
        };
        tracing::info!(
            project = %project.id,
            files = text_digests.len(),
            "project text indexed into brain (no parseable source files)"
        );
        return Ok(CodeIndexOutcome {
            files: text_digests.len(),
            memory_key: first.memory_key.clone(),
        });
    }

    let memory_key = code_map_memory_key(&project.id);
    let opts = spectral::RememberOpts {
        source: Some(CODE_MAP_SOURCE.to_string()),
        visibility: spectral::Visibility::Private,
        // A project's code map is its own episode (R45): the key is stable per
        // project and every re-index upserts the same memory, so successive
        // indexes of one project stay one episode instead of being filed apart
        // by however long passed between them.
        episode_id: Some(memory_key.clone()),
        wing: wing.map(str::to_string),
        ..Default::default()
    };
    brain
        .remember_with(&memory_key, &map.text, opts)
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, error = %e, "code map brain write failed");
            CodeIndexError::BrainWrite(e.to_string())
        })?;

    // Resolve the just-written memory's id and scope it to the project.
    // Best-effort (logged, not fatal): the map is already durable in the Brain.
    match brain.get_memory_by_key(&memory_key).await {
        Ok(Some(mem)) => {
            if let Err(e) = project_association::associate_memory(pool, &project.id, &mem.id).await
            {
                tracing::warn!(project = %project.id, error = %e, "code map project association failed");
            } else {
                tracing::info!(project = %project.id, memory = %mem.id, files = map.files, "project code indexed into brain");
            }
        }
        Ok(None) => {
            tracing::warn!(project = %project.id, key = %memory_key, "code map written but its memory was not found for association")
        }
        Err(e) => {
            tracing::warn!(project = %project.id, error = %e, "code map memory lookup failed")
        }
    }

    Ok(CodeIndexOutcome {
        files: map.files,
        memory_key,
    })
}

/// POST /api/projects/{id}/index-code — parse the project's codebase into a
/// durable, project-scoped **code map** memory in the Brain.
///
/// The keystone of the "code understanding" thread (#471). The `analyze`
/// extension's tree-sitter structure pass was 100% ephemeral — a text blob
/// streamed to a transcript, persisting nothing; code was the one artifact class
/// that never landed durably. This runs that same pass over the project's
/// `root_path` and stores the rendered map in the Brain (Private, a distinct
/// `permagent.code` source, and — the load-bearing rule — NO description) under
/// a deterministic key, then associates it with the project. Re-indexing
/// overwrites the same key (idempotent). Because the map is written
/// description-less, the Librarian claims and enriches it just as it does
/// documents and notes — so a codebase becomes recallable + described like every
/// other artifact class, not parsed once and forgotten.
///
/// Observable, not silent: a missing/unreadable `root_path` is a 400, an absent
/// Brain a 503, and a Brain-write failure a 500 with a message — never a bare
/// status. The project↔memory association is best-effort (logged, not fatal):
/// the map is already durable in the Brain once written; association only scopes
/// it to the project.
async fn index_project_code_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<IndexCodeResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    // The whole point of this route is durable code in the Brain — unlike the
    // document/note handlers there is no other record, so an absent Brain is a
    // 503, not a silent skip.
    let brain = state.brain.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Brain is not available".to_string(),
    ))?;

    let outcome = reindex_project_code(&pool, brain, &project)
        .await
        .map_err(CodeIndexError::http)?;

    Ok(Json(IndexCodeResponse {
        indexed: true,
        files: outcome.files,
        memory_key: outcome.memory_key,
    }))
}

// ── Post-landing code-map refresh: keep the map honest when the trunk moves ──

/// Whether landing approved goal work should refresh the project's code map.
/// Pure — decidable from two facts, so it is testable without a Brain or a
/// filesystem. The load-bearing rule: landing REFRESHES an index someone asked
/// for; it never CREATES one nobody did (a never-indexed project skips, and
/// skips silently — this runs detached behind every approval).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostLandingReindex {
    /// The project has an existing `code:{id}:map` memory and a root to parse.
    Refresh,
    /// No existing code-map memory: the project was never indexed. Silent.
    SkipNeverIndexed,
    /// No usable `root_path` — nothing to parse even if a stale map exists.
    SkipNoRoot,
}

pub fn post_landing_reindex_decision(
    root_path: Option<&str>,
    previously_indexed: bool,
) -> PostLandingReindex {
    match root_path {
        None => PostLandingReindex::SkipNoRoot,
        Some(p) if p.trim().is_empty() => PostLandingReindex::SkipNoRoot,
        Some(_) if !previously_indexed => PostLandingReindex::SkipNeverIndexed,
        Some(_) => PostLandingReindex::Refresh,
    }
}

/// Body of the detached best-effort task decisions.rs spawns after a
/// `LandOutcome::FastForwarded`: the trunk just moved, so an existing code map
/// now describes the pre-landing tree. Refresh it.
///
/// Never returns an error — approval already succeeded and must not be blocked
/// or failed by a tree-sitter pass. Every skip/failure path here is a log line
/// at most: info on a successful refresh, warn on a failed one, and SILENCE
/// when the project was never indexed (see [`post_landing_reindex_decision`]).
/// Brain access goes through the process-wide handle rather than `AppState`
/// because the effect layer only carries a pool; an absent Brain also means no
/// existing map, so skipping is the correct behavior, not a degraded one.
pub async fn refresh_code_map_after_landing(pool: &sqlx::Pool<sqlx::Sqlite>, project_id: &str) {
    let Some(brain) = permagent::agents::platform_extensions::get_global_brain() else {
        return;
    };
    let project = match projects::get_project_by_id_or_slug(pool, project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            tracing::warn!(project = %project_id, "post-landing code reindex: project not found");
            return;
        }
        Err(e) => {
            tracing::warn!(project = %project_id, error = %e, "post-landing code reindex: project lookup failed");
            return;
        }
    };
    let previously_indexed = match brain
        .get_memory_by_key(&code_map_memory_key(&project.id))
        .await
    {
        Ok(existing) => existing.is_some(),
        Err(e) => {
            tracing::warn!(project = %project.id, error = %e, "post-landing code reindex: code-map lookup failed");
            return;
        }
    };
    match post_landing_reindex_decision(project.root_path.as_deref(), previously_indexed) {
        PostLandingReindex::SkipNeverIndexed | PostLandingReindex::SkipNoRoot => {}
        PostLandingReindex::Refresh => match reindex_project_code(pool, &brain, &project).await {
            Ok(outcome) => {
                tracing::info!(project = %project.id, files = outcome.files, key = %outcome.memory_key, "code map refreshed after goal landing");
            }
            Err(e) => {
                tracing::warn!(project = %project.id, error = %e, "post-landing code map refresh failed");
            }
        },
    }
}

// ── Project stack organizer (#512): services + login identity, reference-only ─

/// Create body for a stack entry. REFERENCE-ONLY: there is deliberately no
/// password/secret/token field here and none may be added — `identity` is the
/// account label (email/handle) used to log in, nothing more. Unknown JSON
/// fields are rejected outright (`deny_unknown_fields`) so a client attempting
/// to send `password: …` gets a 422, not silent acceptance.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateStackEntryRequest {
    service_name: String,
    /// One of `project_stack::VALID_CATEGORIES`; defaults to "other".
    category: Option<String>,
    identity: Option<String>,
    notes: Option<String>,
    dashboard_url: Option<String>,
}

/// Deserialize an `Option<Option<T>>` field so a MISSING key (outer `None`,
/// "leave unchanged") is distinguished from an explicit JSON `null`
/// (`Some(None)`, "clear to NULL"). Paired with `#[serde(default)]` for the
/// missing case. serde's stock `Option<Option<T>>` collapses both a missing
/// key and an explicit `null` to `None`; this restores the difference the
/// PATCH clear semantics depend on.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

/// Patch body for a stack entry. Single-Option = leave unchanged; the nullable
/// fields (`identity`, `dashboardUrl`) use double-Option via [`double_option`]
/// so an explicit JSON `null` clears them (vs. an omitted key = unchanged).
/// `deny_unknown_fields` for the same no-secrets reason as create.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateStackEntryRequest {
    service_name: Option<String>,
    category: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    identity: Option<Option<String>>,
    notes: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    dashboard_url: Option<Option<String>>,
}

/// Map a `project_stack` CRUD error to a status: validation messages become
/// 400s, anything else is a 500.
fn stack_error_status(e: String) -> (StatusCode, String) {
    if e.contains("Invalid category") || e.contains("service_name is empty") {
        (StatusCode::BAD_REQUEST, e)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}

/// GET /api/projects/{id}/stack — the project's stack entries, grouped by
/// category then service name.
async fn list_stack_entries_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<StackEntry>>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let entries = project_stack::list_entries(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(entries))
}

/// GET /api/projects/{id}/stack/detected — infer declarations from local code.
/// Read known manifests in the connected folder without running project code.
async fn detect_stack_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<super::project_stack_detection::Detection>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let root = project.root_path.filter(|p| !p.trim().is_empty()).ok_or((
        StatusCode::BAD_REQUEST,
        "Connect a code folder to discover the stack.".to_string(),
    ))?;
    tokio::task::spawn_blocking(move || {
        super::project_stack_detection::detect(std::path::Path::new(&root))
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Stack discovery could not finish.".to_string(),
        )
    })?
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// POST /api/projects/{id}/stack — add a stack entry.
async fn create_stack_entry_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateStackEntryRequest>,
) -> Result<(StatusCode, Json<StackEntry>), (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let entry_id = uuid::Uuid::now_v7().to_string();
    let entry = project_stack::insert_entry(
        &pool,
        &entry_id,
        &project.id,
        &req.service_name,
        req.category.as_deref().unwrap_or("other"),
        req.identity
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        req.notes.as_deref().unwrap_or(""),
        req.dashboard_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )
    .await
    .map_err(stack_error_status)?;

    tracing::info!(project = %project.id, entry = %entry.id, service = %entry.service_name, "stack entry created");
    Ok((StatusCode::CREATED, Json(entry)))
}

/// PATCH /api/projects/{id}/stack/{entry_id} — edit a stack entry.
async fn update_stack_entry_handler(
    State(state): State<Arc<AppState>>,
    Path((id, entry_id)): Path<(String, String)>,
    Json(req): Json<UpdateStackEntryRequest>,
) -> Result<Json<StackEntry>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let entry = project_stack::update_entry(
        &pool,
        &project.id,
        &entry_id,
        UpdateStackEntry {
            service_name: req.service_name,
            category: req.category,
            // Explicit null clears; a set value is trimmed, and trimming to
            // empty also clears (an all-whitespace identity is no identity).
            identity: req
                .identity
                .map(|v| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
            notes: req.notes,
            dashboard_url: req
                .dashboard_url
                .map(|v| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
        },
    )
    .await
    .map_err(stack_error_status)?
    .ok_or((StatusCode::NOT_FOUND, "Stack entry not found".to_string()))?;

    tracing::info!(project = %project.id, entry = %entry.id, "stack entry updated");
    Ok(Json(entry))
}

/// DELETE /api/projects/{id}/stack/{entry_id} — remove a stack entry. 200 on
/// delete, 404 if no such entry.
async fn delete_stack_entry_handler(
    State(state): State<Arc<AppState>>,
    Path((id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let deleted = project_stack::delete_entry(&pool, &project.id, &entry_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Stack entry not found".to_string()));
    }
    tracing::info!(project = %project.id, entry = %entry_id, "stack entry deleted");
    Ok(StatusCode::OK)
}

#[derive(Debug, serde::Deserialize)]
struct SetStrategyBody {
    content: String,
    #[serde(default)]
    points: Option<serde_json::Value>,
    #[serde(default)]
    metrics: Option<serde_json::Value>,
}

/// PUT /api/projects/{id}/strategy/{pillar} — save one GTM strategy pillar
/// (`metadata_json.strategy.<pillar>`), the UI-edit counterpart of the
/// `set_project_strategy` agent tool. Merge-writes the metadata bag.
async fn set_project_strategy_handler(
    State(state): State<Arc<AppState>>,
    Path((id, pillar)): Path<(String, String)>,
    Json(body): Json<SetStrategyBody>,
) -> Result<Json<ProjectResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let content = body.content.trim();
    if content.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "content must not be empty".to_string(),
        ));
    }
    let updated = projects::set_project_strategy(
        &pool,
        &project.id,
        pillar.trim(),
        content,
        projects::StrategyExtras {
            points: body.points,
            metrics: body.metrics,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    // `projects::update_project` emits `project_changed(updated)`.
    Ok(Json(ProjectResponse::from(updated)))
}

#[derive(Debug, serde::Deserialize)]
struct SetBrandBody {
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    bg: Option<String>,
    #[serde(default)]
    fg: Option<String>,
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    typeface: Option<String>,
    #[serde(default)]
    donts: Option<Vec<String>>,
}

/// Body for `PUT /api/projects/{id}/verification-approval`. Every field is
/// optional and only the present ones are applied, so two settings panels
/// cannot overwrite each other's field.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetVerificationApprovalBody {
    /// Replaces the project's allowlist wholesale (de-duplicated and sorted).
    #[serde(default)]
    allowlist: Option<Vec<String>>,
    #[serde(default)]
    read_only_threshold: Option<u32>,
    #[serde(default)]
    full_threshold: Option<u32>,
    /// Set the earned-privilege count directly. Lets a person hand back trust
    /// they took away, and take back trust they gave.
    #[serde(default)]
    clean_runs: Option<u32>,
    /// Restore defaults: no allowlist, no privilege, no standing grants. The
    /// audit history is kept.
    #[serde(default)]
    reset: Option<bool>,
}

/// PUT /api/projects/{id}/brand — merge-write this project's Grow brand kit.
async fn set_project_brand_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetBrandBody>,
) -> Result<Json<ProjectResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    let updated = projects::set_project_brand(
        &pool,
        &project.id,
        projects::ProjectBrand {
            voice: body.voice.unwrap_or_default(),
            origin: body.origin.unwrap_or_default(),
            bg: body.bg.unwrap_or_default(),
            fg: body.fg.unwrap_or_default(),
            accent: body.accent.unwrap_or_default(),
            typeface: body.typeface.unwrap_or_default(),
            donts: body.donts.unwrap_or_default(),
            updated_at: None,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    // `projects::update_project` emits `project_changed(updated)`.
    Ok(Json(ProjectResponse::from(updated)))
}

/// PUT /api/projects/{id}/verification-approval — merge-write this project's
/// command-approval ladder.
///
/// A merge, not a replacement: the caller sends only what it is changing, and
/// the audit history and any standing approve-once grants survive untouched.
/// This is the ONLY write path for the allowlist and the privilege count, so
/// that "editable in the project's Settings" means one place and not several.
///
/// Note what is not here: there is no way to *grant* privilege by API beyond
/// setting the count, and the agent has no credentials to call it. Widening
/// happens when a person answers a Decision Inbox card, or here.
async fn set_verification_approval_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetVerificationApprovalBody>,
) -> Result<Json<ProjectResponse>, (StatusCode, String)> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    permagent::verification_approval::update(&pool, &project.id, move |s| {
        if body.reset.unwrap_or(false) {
            // A reset returns the ladder to a project that has earned nothing.
            // The audit survives: it is the record of how we got here, and
            // erasing it would make the reset itself invisible.
            let audit = std::mem::take(&mut s.audit);
            *s = permagent::verification_approval::ApprovalSettings {
                audit,
                ..Default::default()
            };
            return;
        }
        if let Some(list) = body.allowlist {
            s.allowlist = Vec::new();
            for tok in list {
                s.allowlist_token(&tok);
            }
        }
        if let Some(n) = body.read_only_threshold {
            s.read_only_threshold = n;
        }
        if let Some(n) = body.full_threshold {
            s.full_threshold = n;
        }
        if let Some(n) = body.clean_runs {
            s.clean_runs = n;
        }
    })
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let updated = projects::get_project(&pool, &project.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    // `projects::update_project` emits `project_changed(updated)`.
    Ok(Json(ProjectResponse::from(updated)))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects_handler))
        .route("/api/projects", post(create_project_handler))
        .route("/api/projects/{id}", get(get_project_handler))
        .route("/api/projects/{id}", patch(update_project_handler))
        .route("/api/projects/{id}", delete(delete_project_handler))
        .route("/api/projects/{id}/touch", post(touch_project_handler))
        .route(
            "/api/projects/{id}/strategy/{pillar}",
            axum::routing::put(set_project_strategy_handler),
        )
        .route(
            "/api/projects/{id}/brand",
            axum::routing::put(set_project_brand_handler),
        )
        .route(
            "/api/projects/{id}/verification-approval",
            axum::routing::put(set_verification_approval_handler),
        )
        .route("/api/projects/{id}/intel", get(list_project_intel_handler))
        .route("/api/projects/{id}/market", get(project_market_handler))
        .route(
            "/api/projects/{id}/intel/{item_id}",
            delete(delete_project_intel_handler),
        )
        .route("/api/projects/{id}/tags", get(list_tags_handler))
        .route("/api/projects/{id}/tags", post(add_tag_handler))
        .route("/api/projects/{id}/tags/{tag}", delete(remove_tag_handler))
        .route(
            "/api/projects/{id}/people",
            get(list_project_people_handler),
        )
        .route(
            "/api/projects/{id}/meetings",
            get(list_project_meetings_handler),
        )
        .route("/api/projects/{id}/people", post(associate_person_handler))
        .route(
            "/api/projects/{id}/people/{entity_uuid}",
            delete(disassociate_person_handler),
        )
        .route(
            "/api/projects/{id}/memories",
            get(list_project_memories_handler),
        )
        .route(
            "/api/projects/{id}/memories/{memory_id}",
            post(associate_memory_handler).delete(disassociate_memory_handler),
        )
        .route(
            "/api/projects/{id}/documents",
            get(list_project_documents_handler),
        )
        .route(
            "/api/projects/{id}/documents",
            post(upload_project_documents_handler)
                .layer(DefaultBodyLimit::max(MAX_DOCUMENT_SIZE * 10)),
        )
        .route(
            "/api/projects/{id}/documents/{doc_id}",
            get(get_project_document_handler).delete(delete_project_document_handler),
        )
        .route(
            "/api/projects/{id}/notes",
            get(list_project_notes_handler).post(create_project_note_handler),
        )
        .route(
            "/api/projects/{id}/notes/{note_id}",
            delete(delete_project_note_handler),
        )
        .route(
            "/api/projects/{id}/recordings/{recording_id}/triage",
            get(get_meeting_triage_handler),
        )
        .route(
            "/api/projects/{id}/recordings/{recording_id}/triage/retry",
            post(retry_meeting_triage_handler),
        )
        .route(
            "/api/projects/{id}/recordings/{recording_id}/triage/session",
            get(get_meeting_triage_session_handler),
        )
        .route(
            "/api/projects/{id}/index-code",
            post(index_project_code_handler),
        )
        .route(
            "/api/projects/{id}/stack",
            get(list_stack_entries_handler).post(create_stack_entry_handler),
        )
        .route(
            "/api/projects/{id}/stack/detected",
            get(detect_stack_handler),
        )
        .route(
            "/api/projects/{id}/stack/{entry_id}",
            patch(update_stack_entry_handler).delete(delete_stack_entry_handler),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use serial_test::serial;
    use std::path::Path as StdPath;
    use tower::ServiceExt;

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn detected_stack_route_resolves_id_and_slug_without_writing_manual_services() {
        let _root = crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        let code = tempfile::tempdir().unwrap();
        std::fs::write(
            code.path().join("package.json"),
            r#"{"dependencies":{"react":"19","next":"15"},"private_key":"fixture-must-not-leak"}"#,
        )
        .unwrap();
        let project = projects::create_project(
            &pool,
            projects::CreateProject {
                name: format!("Stack route {}", uuid::Uuid::new_v4()),
                root_path: Some(code.path().to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let manual = permagent::project_stack::insert_entry(
            &pool,
            &uuid::Uuid::new_v4().to_string(),
            &project.id,
            "User-corrected service",
            "other",
            Some("fixture@example.test"),
            "Keep my correction",
            None,
        )
        .await
        .unwrap();
        let authorization = format!(
            "Bearer {}",
            state.daemon_token.as_ref().expect("isolated test token")
        );
        // Use the complete production router, including its auth middleware.
        let app = crate::routes::configure(state);
        for header in [None, Some("Bearer invalid-test-token")] {
            let mut request =
                Request::builder().uri(format!("/api/projects/{}/stack/detected", project.id));
            if let Some(value) = header {
                request = request.header("authorization", value);
            }
            assert_eq!(
                app.clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        for id in [&project.id, &project.slug] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/projects/{id}/stack/detected"))
                        .header("authorization", &authorization)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(json["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["service_name"] == "React"));
            assert!(!String::from_utf8_lossy(&bytes).contains("fixture-must-not-leak"));
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM project_stack_entries WHERE project_id=?1")
                .bind(&project.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            count, 1,
            "discovery must never create manual account records"
        );
        assert_eq!(
            permagent::project_stack::get_entry(&pool, &project.id, &manual.id)
                .await
                .unwrap(),
            Some(manual)
        );
        projects::update_project(
            &pool,
            &project.id,
            projects::UpdateProject {
                root_path: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let missing_root = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/projects/{}/stack/detected", project.id))
                    .header("authorization", &authorization)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_root.status(), StatusCode::BAD_REQUEST);
        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/api/projects/missing-stack-fixture/stack/detected")
                    .header("authorization", &authorization)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    /// Dedicated staging gate. Never falls back to the live daemon or a random
    /// port. Run explicitly with --ignored; a busy :3002 is a failed preflight.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    #[ignore = "requires exclusive isolated staging port 3002"]
    async fn ux_stack_discovery_http_staging() {
        let root = crate::test_support::test_root();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:3002")
            .await
            .expect("staging :3002 must be free; do not stop an unrelated daemon");
        // The serving application bootstraps this ontology before AppState;
        // ordinary lib tests deliberately omit it. This isolated acceptance
        // test needs the real graph to verify person-field persistence.
        let ontology = permagent::config::paths::Paths::brain_ontology();
        assert!(
            ontology.starts_with(&root),
            "ontology must be inside test_root"
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ontology)
            .unwrap();
        std::io::Write::write_all(
            &mut file,
            crate::brain_bootstrap::BASELINE_ONTOLOGY.as_bytes(),
        )
        .unwrap();
        let state = AppState::new(true).await.unwrap();
        assert!(
            state.brain.is_some(),
            "isolated Brain must mount for People acceptance"
        );
        let token = state.daemon_token.clone().expect("isolated staging token");
        let pool = state.session_manager().pool_clone().await.unwrap();
        let code = tempfile::tempdir().unwrap();
        std::fs::write(
            code.path().join("package.json"),
            r#"{"dependencies":{"react":"19"}}"#,
        )
        .unwrap();
        let project = projects::create_project(
            &pool,
            projects::CreateProject {
                name: format!("UX HTTP fixture {}", uuid::Uuid::new_v4()),
                root_path: Some(code.path().to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        struct ServerGuard(tokio::task::JoinHandle<()>);
        impl Drop for ServerGuard {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let server = ServerGuard(tokio::spawn(async move {
            axum::serve(listener, crate::routes::configure(state))
                .await
                .unwrap();
        }));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let url = format!(
            "http://127.0.0.1:3002/api/projects/{}/stack/detected",
            project.id
        );
        assert_eq!(
            client.get(&url).send().await.unwrap().status().as_u16(),
            401
        );
        let response = client.get(&url).bearer_auth(&token).send().await.unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let body: serde_json::Value = response.json().await.unwrap();
        assert!(body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["service_name"] == "React"));
        assert!(permagent::project_stack::list_entries(&pool, &project.id)
            .await
            .unwrap()
            .is_empty());

        // Exercise the same authenticated routes used by the selected Project,
        // People, Automate and History views. All records belong to test_root;
        // no schedules are started and no communication is sent.
        let project_url = format!("http://127.0.0.1:3002/api/projects/{}", project.id);
        let shelved = client
            .patch(&project_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({"status": "paused"}))
            .send()
            .await
            .unwrap();
        assert_eq!(shelved.status().as_u16(), 200);
        let reread: serde_json::Value = client
            .get(&project_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reread["status"], "paused");

        let cards_url = format!("{project_url}/cards");
        let created = client
            .post(&cards_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({"title": "Inspect synthetic planting guide"}))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status().as_u16(), 201);
        let card: serde_json::Value = created.json().await.unwrap();
        let card_id = card["id"].as_str().expect("created task id");
        let listed: serde_json::Value = client
            .get(&cards_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"].as_str() == Some(card_id)));

        let created = client
            .post("http://127.0.0.1:3002/api/people")
            .bearer_auth(&token)
            .json(&serde_json::json!({"display_name": format!("UX fixture {}", uuid::Uuid::new_v4())}))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status().as_u16(), 201);
        let person: serde_json::Value = created.json().await.unwrap();
        let person_id = person["person"]["entity_uuid"]
            .as_str()
            .expect("created person id");
        let directory: serde_json::Value = client
            .get("http://127.0.0.1:3002/api/people/directory")
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(directory
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["entity_uuid"].as_str() == Some(person_id)));
        for (path, field) in [
            ("/schedule/list", "jobs"),
            ("/api/sessions?types=user,scheduled,sub_agent", "sessions"),
        ] {
            let data: serde_json::Value = client
                .get(format!("http://127.0.0.1:3002{path}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            assert!(
                data[field].is_array(),
                "{path} must preserve the UI list contract"
            );
        }
        // Optional interactive acceptance window. Only the explicit ignored
        // staging test can enter it, and the outer build guard still applies.
        if let Ok(directory) = std::env::var("PERMAGENT_UX_BROWSER_DIR") {
            use std::io::Write;
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let directory = std::path::PathBuf::from(directory);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(directory.join("state.json")).unwrap();
            file.write_all(
                serde_json::json!({"token": token, "projectId": project.id, "personId": person_id})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
            println!("Isolated browser acceptance ready on :3002");
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
            while !directory.join("finish").is_file() {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "browser acceptance timed out"
                );
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            assert_eq!(
                projects::get_project_by_id_or_slug(&pool, &project.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "active",
                "browser status control must persist its change"
            );
            let directory: serde_json::Value = client
                .get("http://127.0.0.1:3002/api/people/directory")
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let edited = directory
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["entity_uuid"].as_str() == Some(person_id))
                .unwrap();
            assert_eq!(
                edited["notes"], "UX browser wiring verified",
                "browser person editor must persist its change"
            );
            assert!(
                permagent::project_stack::list_entries(&pool, &project.id)
                    .await
                    .unwrap()
                    .is_empty(),
                "browser discovery must not create manual service records"
            );
        }
        drop(server);
    }

    // ── Post-landing reindex decision (pure) ────────────────────────────────
    //
    // The invariant under test: landing approved goal work REFRESHES a code
    // map someone asked for, and never CREATES one nobody did. The decision is
    // a pure function of (root_path, previously_indexed) so these run without
    // a Brain, a filesystem, or a tokio runtime.

    #[test]
    fn landing_skips_when_project_was_never_indexed() {
        // Perfectly indexable project — but nobody ever asked for an index.
        assert_eq!(
            post_landing_reindex_decision(Some("/some/real/project"), false),
            PostLandingReindex::SkipNeverIndexed
        );
    }

    #[test]
    fn landing_refreshes_an_existing_index() {
        assert_eq!(
            post_landing_reindex_decision(Some("/some/real/project"), true),
            PostLandingReindex::Refresh
        );
    }

    #[test]
    fn landing_skips_without_a_root_path_even_if_previously_indexed() {
        // A stale map with no root to re-parse: nothing can refresh it.
        assert_eq!(
            post_landing_reindex_decision(None, true),
            PostLandingReindex::SkipNoRoot
        );
        assert_eq!(
            post_landing_reindex_decision(None, false),
            PostLandingReindex::SkipNoRoot
        );
    }

    #[test]
    fn landing_treats_blank_root_path_as_absent() {
        // Mirrors the route's `.filter(|p| !p.trim().is_empty())` — an empty
        // or whitespace root_path column must not read as "has a root".
        assert_eq!(
            post_landing_reindex_decision(Some(""), true),
            PostLandingReindex::SkipNoRoot
        );
        assert_eq!(
            post_landing_reindex_decision(Some("   "), true),
            PostLandingReindex::SkipNoRoot
        );
    }

    #[test]
    fn code_map_key_is_deterministic_and_matches_the_route_format() {
        // The route has always written `code:{project_id}:map`; the landing
        // hook's existence probe must look up the SAME key or the skip logic
        // silently never refreshes anything.
        assert_eq!(code_map_memory_key("abc-123"), "code:abc-123:map");
        assert_eq!(
            code_map_memory_key("abc-123"),
            code_map_memory_key("abc-123")
        );
    }

    // `project_reindex_scopes_same_named_files_and_replaces_edits` and
    // `note_only_project_reindex_succeeds_before_no_source_files` do NOT live
    // here. Both need a real Brain, and `state.brain` is always `None` in this
    // lib-test binary: `test_support::test_root` creates brain_dir but never
    // writes `ontology.toml`, and `AppState::new` mounts a Brain only when both
    // exist. That is deliberate — hundreds of tests in this binary are written
    // against a brainless AppState — so the two reindex tests live in their own
    // integration binaries, which own their process-wide PERMAGENT_PATH_ROOT
    // and write the ontology:
    //   tests/project_reindex_scoping.rs
    //   tests/project_reindex_notes_only.rs

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn project_documents_use_generated_paths_and_safe_serving_headers() {
        let test_root = crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        let project = projects::create_project(
            &pool,
            projects::CreateProject {
                name: format!("Safe documents {}", uuid::Uuid::new_v4()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let traversal_name = format!("../../evil-{}", uuid::Uuid::new_v4());
        let traversal_target = test_root
            .join("project-docs")
            .join(traversal_name.trim_start_matches("../../"));
        let traversal_doc = save_project_document(
            &state,
            &pool,
            &project,
            &traversal_name,
            "text/html",
            b"<script>alert(1)</script>",
        )
        .await
        .unwrap();
        assert_eq!(traversal_doc.filename, traversal_name);
        assert_eq!(
            StdPath::new(&traversal_doc.path).file_name().unwrap(),
            traversal_doc.id.as_str()
        );
        assert!(StdPath::new(&traversal_doc.path).exists());
        assert!(!traversal_target.exists());

        let absolute_target = test_root.join(format!("absolute-evil-{}", uuid::Uuid::new_v4()));
        let absolute_name = absolute_target.to_string_lossy().to_string();
        let pdf_doc = save_project_document(
            &state,
            &pool,
            &project,
            &absolute_name,
            "application/pdf",
            b"%PDF-1.7\n",
        )
        .await
        .unwrap();
        assert_eq!(pdf_doc.filename, absolute_name);
        assert_eq!(
            StdPath::new(&pdf_doc.path).file_name().unwrap(),
            pdf_doc.id.as_str()
        );
        assert!(StdPath::new(&pdf_doc.path).exists());
        assert!(!absolute_target.exists());

        let stored = project_documents::list_documents(&pool, &project.id)
            .await
            .unwrap();
        assert!(stored.iter().any(|doc| doc.filename == traversal_name));
        assert!(stored.iter().any(|doc| doc.filename == absolute_name));

        let app = routes(state);
        let html_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{}/documents/{}",
                        project.id, traversal_doc.id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            html_response.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        assert_eq!(
            html_response.headers()[header::CONTENT_DISPOSITION],
            "attachment"
        );
        assert_eq!(
            html_response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );

        let pdf_response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{}/documents/{}",
                        project.id, pdf_doc.id
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            pdf_response.headers()[header::CONTENT_TYPE],
            "application/pdf"
        );
        assert_eq!(
            pdf_response.headers()[header::CONTENT_DISPOSITION],
            "inline"
        );
        assert_eq!(
            pdf_response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn delete_project_intel_is_scoped_and_returns_not_found() {
        crate::test_support::test_root();
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        let first = projects::create_project(
            &pool,
            projects::CreateProject {
                name: format!("Intel delete first {}", uuid::Uuid::new_v4()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let second = projects::create_project(
            &pool,
            projects::CreateProject {
                name: format!("Intel delete second {}", uuid::Uuid::new_v4()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let item_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO project_intel
             (id, project_id, kind, name, source_url, created_at)
             VALUES (?, ?, 'competitor', 'Rival', 'https://rival.example',
                     strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        )
        .bind(&item_id)
        .bind(&first.id)
        .execute(&pool)
        .await
        .unwrap();
        let app = routes(state);

        let mismatched = Request::builder()
            .method("DELETE")
            .uri(format!("/api/projects/{}/intel/{}", second.id, item_id))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(mismatched).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let still_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_intel WHERE id = ?)")
                .bind(&item_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(still_exists);

        let unknown = Request::builder()
            .method("DELETE")
            .uri(format!(
                "/api/projects/{}/intel/{}",
                first.id,
                uuid::Uuid::new_v4()
            ))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(unknown).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );

        let delete_item = Request::builder()
            .method("DELETE")
            .uri(format!("/api/projects/{}/intel/{}", first.id, item_id))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(delete_item).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_intel WHERE id = ?)")
                .bind(&item_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!exists);
    }

    /// The daemon's split must invert the UI's compose exactly. These two
    /// literals are a cross-language contract (composeMeetingBody in
    /// useMeetingDictation.ts); drift silently orphans the user's notes from
    /// the summary that is supposed to be steered by them.
    #[test]
    fn split_meeting_body_inverts_the_ui_compose() {
        let body = "## Your notes\n\npricing objections\n\n## Transcript\n\nthey said 2000";
        let (notes, transcript) = permagent::meeting_triage::split_meeting_body(body);
        assert_eq!(notes.as_deref(), Some("pricing objections"));
        assert_eq!(transcript, "they said 2000");
    }

    /// A transcript-only body (the user typed nothing) is all transcript —
    /// never mistaken for notes.
    #[test]
    fn split_meeting_body_without_notes_is_all_transcript() {
        let (notes, transcript) =
            permagent::meeting_triage::split_meeting_body("just the words that were said");
        assert!(notes.is_none());
        assert_eq!(transcript, "just the words that were said");
    }

    /// Multi-line shorthand survives intact — the model is told to read it
    /// generously, so losing lines here would silently drop the user's intent.
    #[test]
    fn split_meeting_body_keeps_multiline_notes() {
        let body = "## Your notes\n\n- a\n- b??\n- c\n\n## Transcript\n\nx";
        let (notes, _) = permagent::meeting_triage::split_meeting_body(body);
        assert_eq!(notes.as_deref(), Some("- a\n- b??\n- c"));
    }

    #[test]
    fn item_key_ignores_case_and_whitespace_but_not_words() {
        assert_eq!(
            permagent::meeting_triage::item_key(0, "  Ship   the Thing "),
            permagent::meeting_triage::item_key(0, "ship the thing")
        );
        assert_ne!(
            permagent::meeting_triage::item_key(0, "ship the thing"),
            permagent::meeting_triage::item_key(0, "ship the other thing")
        );
    }
}
