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

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use permagent::projects::{self, PERSONAL_PROJECT_ID};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    tags: Vec<String>,
    created_at: String,
    updated_at: String,
    last_opened_at: String,
}

impl From<projects::Project> for ProjectResponse {
    fn from(p: projects::Project) -> Self {
        Self {
            id: p.id, slug: p.slug, name: p.name, description: p.description,
            status: p.status, root_path: p.root_path, site_url: p.site_url,
            repo_url: p.repo_url, notes: p.notes, tags: p.tags,
            created_at: p.created_at, updated_at: p.updated_at, last_opened_at: p.last_opened_at,
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
}

#[derive(Deserialize)]
pub struct ListProjectsQuery { status: Option<String> }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTagRequest { tag: String }

#[derive(Serialize)]
pub struct DeleteResponse { deleted: bool }

#[derive(Serialize)]
pub struct TouchResponse { touched: bool }

async fn list_projects_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Json<Vec<ProjectResponse>>, StatusCode> {
    let pool = state.session_manager().pool_clone().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = projects::list_projects(&pool, query.status.as_deref()).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items.into_iter().map(ProjectResponse::from).collect()))
}

async fn get_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProjectResponse>, StatusCode> {
    let pool = state.session_manager().pool_clone().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, &id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(ProjectResponse::from(project)))
}

async fn create_project_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), (StatusCode, String)> {
    let pool = state.session_manager().pool_clone().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let input = projects::CreateProject {
        name: req.name, slug: req.slug, description: req.description,
        root_path: req.root_path, site_url: req.site_url, repo_url: req.repo_url,
        notes: req.notes, tags: req.tags,
    };
    let project = projects::create_project(&pool, input).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(ProjectResponse::from(project))))
}

async fn update_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectResponse>, (StatusCode, String)> {
    let pool = state.session_manager().pool_clone().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?.ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    if project.id == PERSONAL_PROJECT_ID && (req.slug.is_some() || req.status.is_some()) {
        return Err((StatusCode::FORBIDDEN, "Cannot change slug or status of the Personal project".to_string()));
    }
    let input = projects::UpdateProject {
        name: req.name, slug: req.slug, description: req.description, status: req.status,
        root_path: req.root_path, site_url: req.site_url, repo_url: req.repo_url, notes: req.notes,
    };
    let updated = projects::update_project(&pool, &project.id, input).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?.ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    Ok(Json(ProjectResponse::from(updated)))
}

async fn delete_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let pool = state.session_manager().pool_clone().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let project = projects::get_project_by_id_or_slug(&pool, &id).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?.ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    if project.id == PERSONAL_PROJECT_ID {
        return Err((StatusCode::FORBIDDEN, "Cannot delete the Personal project".to_string()));
    }
    let deleted = projects::delete_project(&pool, &project.id).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(DeleteResponse { deleted }))
}

async fn touch_project_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<TouchResponse>, StatusCode> {
    let pool = state.session_manager().pool_clone().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let touched = projects::touch_project(&pool, &id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if touched { Ok(Json(TouchResponse { touched })) } else { Err(StatusCode::NOT_FOUND) }
}

async fn list_tags_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let pool = state.session_manager().pool_clone().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = projects::get_project(&pool, &id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    let tags = projects::list_tags(&pool, &id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(tags))
}

async fn add_tag_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AddTagRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state.session_manager().pool_clone().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let ok = projects::add_tag(&pool, &id, &req.tag).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if ok { Ok(StatusCode::CREATED) } else { Err((StatusCode::NOT_FOUND, "Project not found".to_string())) }
}

async fn remove_tag_handler(
    State(state): State<Arc<AppState>>,
    Path((id, tag)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let pool = state.session_manager().pool_clone().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let removed = projects::remove_tag(&pool, &id, &tag).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if removed { Ok(StatusCode::OK) } else { Err(StatusCode::NOT_FOUND) }
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
        .with_state(state)
}
