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
use permagent::project_association::{self, ProjectPerson};
use permagent::project_documents::{self, ProjectDocument};
use permagent::project_notes::{self, ProjectNote};
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
    /// / `build_timeout_secs` — the project-level default completion check.
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
    Ok(Json(people))
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
/// `~/.permagent/project-docs/<project_id>/<doc_id>/<filename>`, insert the
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

    let docs_base = dirs::home_dir()
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "no home directory".to_string(),
        ))?
        .join(".permagent")
        .join("project-docs")
        .join(&project.id);

    let doc_id = uuid::Uuid::now_v7().to_string();
    let dir = docs_base.join(&doc_id);
    fs::create_dir_all(&dir).await.map_err(|e| {
        tracing::error!(project = %project.id, %filename, error = %e, "project document dir create failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    let file_path = dir.join(filename);
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
/// `~/.permagent/project-docs/<project_id>/<doc_id>/<filename>`, then a
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

    Ok(Json(UploadDocumentsResponse { documents: results }))
}

/// GET /api/projects/{id}/documents/{doc_id} — stream a document inline.
///
/// The stored `mime_type` and `inline` disposition let the browser (and the
/// in-app viewer's object URL) render PDFs/images natively.
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
    let disposition = format!("inline; filename=\"{}\"", doc.filename);

    Ok((
        [
            (header::CONTENT_TYPE, doc.mime_type),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    ))
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
    Ok(StatusCode::NO_CONTENT)
}

// ── Project notes: freeform notes indexed into the Brain ─────────────────────

/// `source` tag on every memory a note writes. Deliberately NOT
/// `permagent.activity` (which pruning/consolidation reap) so notes are durable,
/// and description-less on write so the Librarian claims + enriches them exactly
/// as it does Reader-ingested documents.
const NOTE_SOURCE: &str = "permagent.note";

#[derive(Deserialize)]
struct CreateNoteRequest {
    title: Option<String>,
    body: String,
}

/// The Brain content for a note: title + body when a title is present, else the
/// body alone. Mirrors how the Reader stores the full text of a document.
fn note_memory_content(title: Option<&str>, body: &str) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("{t}\n\n{body}"),
        None => body.to_string(),
    }
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
    Ok(Json(notes))
}

/// POST /api/projects/{id}/notes — create a note (`{title?, body}`).
///
/// The row is the durable record; indexing the note's text into the Brain (so it
/// is recallable + Librarian-enriched, scoped to the project) is best-effort —
/// the note is returned even if the Brain write fails. The Brain write mirrors
/// the Reader's `RememberOpts` exactly (a distinct source, Private, and NO
/// description) so the Librarian claims and enriches notes the same way it does
/// ingested documents.
async fn create_project_note_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateNoteRequest>,
) -> Result<Json<ProjectNote>, (StatusCode, String)> {
    let body = req.body.trim();
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "note body is empty".to_string()));
    }
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;

    let note_id = uuid::Uuid::now_v7().to_string();
    let memory_key = format!("note:{}:{}", project.id, note_id);

    // Best-effort Brain index. On success we resolve the memory id and associate
    // it with the project; the memory_key is recorded on the row regardless (it
    // is stable/deterministic) so a later reconcile could re-associate. A Brain
    // failure is logged and skipped — the note still persists and returns.
    let mut stored_memory_key: Option<&str> = None;
    if let Some(brain) = state.brain.as_ref() {
        let content = note_memory_content(title, body);
        let opts = spectral::RememberOpts {
            source: Some(NOTE_SOURCE.to_string()),
            visibility: spectral::Visibility::Private,
            ..Default::default()
        };
        match brain.remember_with(&memory_key, &content, opts).await {
            Ok(_) => {
                stored_memory_key = Some(&memory_key);
                match brain.get_memory_by_key(&memory_key).await {
                    Ok(Some(mem)) => {
                        if let Err(e) =
                            project_association::associate_memory(&pool, &project.id, &mem.id).await
                        {
                            tracing::warn!(project = %project.id, note = %note_id, error = %e, "project note brain association failed");
                        } else {
                            tracing::info!(project = %project.id, note = %note_id, memory = %mem.id, "project note indexed into brain");
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(project = %project.id, note = %note_id, key = %memory_key, "project note written but its memory was not found for association")
                    }
                    Err(e) => {
                        tracing::warn!(project = %project.id, note = %note_id, error = %e, "project note memory lookup failed")
                    }
                }
            }
            Err(e) => {
                tracing::warn!(project = %project.id, note = %note_id, error = %e, "project note brain write failed (note still saved)")
            }
        }
    }

    let note = project_notes::insert_note(&pool, &note_id, &project.id, title, body, stored_memory_key)
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, note = %note_id, error = %e, "project note insert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

    tracing::info!(project = %project.id, note = %note_id, "project note created");
    Ok(Json(note))
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

    let root_path = project
        .root_path
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Project has no root_path to index".to_string(),
        ))?;
    let root = std::path::Path::new(&root_path).to_path_buf();
    if !root.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("root_path is not a readable directory: {root_path}"),
        ));
    }

    // The whole point of this route is durable code in the Brain — unlike the
    // document/note handlers there is no other record, so an absent Brain is a
    // 503, not a silent skip.
    let brain = state.brain.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Brain is not available".to_string(),
    ))?;

    // Tree-sitter parsing is CPU-bound (rayon) — never run it on the async
    // executor. Build the map off-thread. `max_depth = 0` = the whole tree
    // (WalkBuilder still skips .gitignore'd artifacts like node_modules/target).
    let map = tokio::task::spawn_blocking(move || analyze::build_code_map(&root, 0))
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, error = %e, "code index parse task panicked");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("code parse task panicked: {e}"),
            )
        })?;

    if map.files == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "No source files could be parsed under root_path".to_string(),
        ));
    }

    let memory_key = format!("code:{}:map", project.id);
    let opts = spectral::RememberOpts {
        source: Some(CODE_MAP_SOURCE.to_string()),
        visibility: spectral::Visibility::Private,
        ..Default::default()
    };
    brain
        .remember_with(&memory_key, &map.text, opts)
        .await
        .map_err(|e| {
            tracing::error!(project = %project.id, error = %e, "code map brain write failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("brain write failed: {e}"),
            )
        })?;

    // Resolve the just-written memory's id and scope it to the project.
    // Best-effort (logged, not fatal): the map is already durable in the Brain.
    match brain.get_memory_by_key(&memory_key).await {
        Ok(Some(mem)) => {
            if let Err(e) = project_association::associate_memory(&pool, &project.id, &mem.id).await
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

    Ok(Json(IndexCodeResponse {
        indexed: true,
        files: map.files,
        memory_key,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects_handler))
        .route("/api/projects", post(create_project_handler))
        .route("/api/projects/{id}", get(get_project_handler))
        .route("/api/projects/{id}", patch(update_project_handler))
        .route("/api/projects/{id}", delete(delete_project_handler))
        .route("/api/projects/{id}/touch", post(touch_project_handler))
        .route("/api/projects/{id}/tags", get(list_tags_handler))
        .route("/api/projects/{id}/tags", post(add_tag_handler))
        .route("/api/projects/{id}/tags/{tag}", delete(remove_tag_handler))
        .route(
            "/api/projects/{id}/people",
            get(list_project_people_handler),
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
            "/api/projects/{id}/index-code",
            post(index_project_code_handler),
        )
        .with_state(state)
}
