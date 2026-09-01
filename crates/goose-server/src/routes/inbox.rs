//! File-intake inbox routes.
//!
//! Endpoints:
//!   GET  /api/inbox              — list inbox files (metadata rows), newest first
//!   POST /api/inbox              — record a file that landed in the Permagent inbox
//!   POST /api/inbox/{id}/route   — send an inbox file to a destination surface
//!
//! The in-app Browser webview (desktop process) redirects a download onto disk
//! under `~/.permagent/inbox/` and then POSTs the metadata here so it persists in
//! permagent.db and is listable (#393). Routing (#395) is explicit and
//! user-controlled — the user picks the destination from the inbox panel;
//! Permagent never guesses where a file goes. See [`permagent::inbox`].
//!
//! # Destinations (#395 routing model)
//!
//! * `brain`     — the Reader reads the file (local OCR / text extraction) and
//!   stores the full text in the Brain (`permagent.reader` source, so the
//!   Librarian claims + enriches it). Row status → `ingested`.
//! * `project`   — the file becomes a project document via the exact write path
//!   `POST /api/projects/{id}/documents` uses (disk copy + row + best-effort
//!   Brain index). Row status → `routed`, `project_id` stamped.
//! * `scheduler` — a `social_post` card is created on the chosen project's
//!   board (the scheduler surface is card-based; there is no standalone
//!   scheduler yet — verified in scoping). The card's `metadata_json` carries
//!   an `attachment` block referencing the inbox file. Row status → `routed`,
//!   `project_id` stamped.
//!
//! Routing does not move or delete the bytes in `~/.permagent/inbox/` — the
//! row's status records the lifecycle, and re-routing the same file to another
//! destination is deliberately allowed (send a flyer to the Brain AND a
//! project); each route is an explicit user action.
//!
//! # Error shape
//!
//! Every handler here returns [`ErrorResponse`] on failure — a JSON
//! `{ "message": "…" }` body, not axum's bare `(StatusCode, String)`
//! `text/plain` response. `apiFetch` on the client only ever reads JSON, so a
//! `text/plain` error used to surface as the generic "Unknown error" no matter
//! what the route actually said (the filename, the reason) — see
//! `crate::routes::errors`.

use crate::routes::errors::ErrorResponse;
use crate::routes::projects::save_project_document;
use crate::state::AppState;
use axum::extract::Path;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use permagent::config::paths::Paths;
use permagent::events;
use permagent::inbox::{self, InboxFile, NewInboxFile};
use permagent::{cards, grow_media, projects, reader};
use std::sync::Arc;

/// Same cap as the Reader ingest route and the project documents path.
const MAX_ROUTE_FILE_SIZE: usize = 50 * 1024 * 1024; // 50 MB

async fn list_inbox_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<InboxFile>>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;

    let files = inbox::list_inbox_files(&pool)
        .await
        .map_err(ErrorResponse::internal)?;

    Ok(Json(files))
}

/// Best-effort hostname from a download's origin URL, for the toast body and
/// the `inbox_file_received` event payload. `None` on a missing/unparseable
/// URL — never a guess.
fn source_host(original_url: Option<&str>) -> Option<String> {
    original_url
        .and_then(|u| url::Url::parse(u).ok())?
        .host_str()
        .map(str::to_string)
}

async fn create_inbox_handler(
    State(state): State<Arc<AppState>>,
    Json(new): Json<NewInboxFile>,
) -> Result<(StatusCode, Json<InboxFile>), ErrorResponse> {
    if new.filename.trim().is_empty() || new.disk_path.trim().is_empty() {
        return Err(ErrorResponse::bad_request(
            "filename and disk_path are required",
        ));
    }

    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;

    let saved = inbox::insert_inbox_file(&pool, &new)
        .await
        .map_err(ErrorResponse::internal)?;

    // #395 download feedback: announce arrival on the bus so the Downloads
    // pane can toast even before anyone has chosen where the file goes — this
    // fires on landing, distinct from `project_changed(id, "documents")`,
    // which only fires once a file is *routed* to a project.
    events::emit(events::inbox_file_received(
        &saved.filename,
        saved.size_bytes,
        source_host(saved.original_url.as_deref()).as_deref(),
        &saved.status,
    ));

    Ok((StatusCode::CREATED, Json(saved)))
}

// ── Routing (#395) ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct RouteRequest {
    /// `brain` | `project` | `scheduler`.
    destination: String,
    /// Required for `project` and `scheduler` (both are project-scoped);
    /// ignored for `brain`. Accepts a project id or slug.
    project_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct RouteResponse {
    /// The inbox row after routing (status/project_id updated).
    file: InboxFile,
    destination: String,
    /// Reader digest summary — set for `brain` (empty when the Reader
    /// classified the file as visual and stored nothing).
    summary: Option<String>,
    /// Project document id — set for `project`.
    document_id: Option<String>,
    /// social_post card id — set for `scheduler`.
    card_id: Option<String>,
    /// Brain memory key — set for `brain` when the Reader stored text.
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_key: Option<String>,
}

/// Load the routed file's bytes from the inbox directory. `disk_path` is
/// written by our own download bridge, but reject traversal anyway — a DB row
/// must never be able to read outside `~/.permagent/inbox/`.
async fn read_inbox_bytes(file: &InboxFile) -> Result<Vec<u8>, ErrorResponse> {
    let rel = std::path::Path::new(&file.disk_path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ErrorResponse::bad_request("invalid inbox disk_path"));
    }

    let abs = Paths::inbox_dir().join(rel);
    let data = tokio::fs::read(&abs).await.map_err(|e| {
        tracing::warn!(file = %file.id, path = %abs.display(), error = %e, "inbox route: file missing on disk");
        ErrorResponse::conflict(format!(
            "{} is no longer on disk in the inbox",
            file.filename
        ))
    })?;
    if data.len() > MAX_ROUTE_FILE_SIZE {
        return Err(ErrorResponse::bad_request(format!(
            "{} exceeds the {MAX_ROUTE_FILE_SIZE}-byte limit",
            file.filename
        )));
    }
    Ok(data)
}

/// Resolve the project a project-scoped destination targets, or 400/404.
async fn resolve_target_project(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    destination: &str,
    project_id: Option<&str>,
) -> Result<projects::Project, ErrorResponse> {
    let pid = project_id
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            ErrorResponse::bad_request(format!("destination '{destination}' requires a project_id"))
        })?;
    projects::get_project_by_id_or_slug(pool, pid)
        .await
        .map_err(ErrorResponse::internal)?
        .ok_or_else(|| ErrorResponse::not_found("Project not found"))
}

/// POST /api/inbox/{id}/route — explicitly send an inbox file to one
/// destination surface. The user picks; Permagent never guesses (#395).
async fn route_inbox_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RouteRequest>,
) -> Result<Json<RouteResponse>, ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;

    let file = inbox::get_inbox_file(&pool, &id)
        .await
        .map_err(ErrorResponse::internal)?
        .ok_or_else(|| ErrorResponse::not_found("Inbox file not found"))?;
    if file.status == "deleted" {
        return Err(ErrorResponse::conflict("This inbox file was deleted"));
    }

    let destination = req.destination.trim().to_lowercase();
    let content_type = file.content_type.clone().unwrap_or_default();

    let (new_status, project_stamp, summary, document_id, card_id, memory_key) = match destination
        .as_str()
    {
        "brain" => {
            let data = read_inbox_bytes(&file).await?;
            // Mirror the Reader ingest route's image-vs-document decision.
            let result = if content_type.starts_with("image/") {
                reader::ingest_image(&data, &file.filename).await
            } else {
                reader::ingest_document(&data, &file.filename, &content_type).await
            };
            let digest = result.map_err(|e| {
                tracing::warn!(file = %file.id, error = %e, "inbox route: reader ingest failed");
                ErrorResponse::unprocessable(format!(
                    "The Reader could not read {}: {e}",
                    file.filename
                ))
            })?;
            let key = if digest.is_visual {
                None
            } else {
                Some(digest.memory_key)
            };
            ("ingested", None, Some(digest.summary), None, None, key)
        }
        "project" => {
            let project =
                resolve_target_project(&pool, &destination, req.project_id.as_deref()).await?;
            let data = read_inbox_bytes(&file).await?;
            let mime = file
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let doc = save_project_document(&state, &pool, &project, &file.filename, &mime, &data)
                .await?;
            ("routed", Some(project.id), None, Some(doc.id), None, None)
        }
        "scheduler" => {
            let project =
                resolve_target_project(&pool, &destination, req.project_id.as_deref()).await?;
            // Prove the bytes still exist before creating a card that points at
            // them (same guard the other destinations get by reading the file).
            let _ = read_inbox_bytes(&file).await?;

            let description = format!(
                "Attachment from the Downloads inbox{}",
                file.original_url
                    .as_deref()
                    .map(|u| format!(" (downloaded from {u})"))
                    .unwrap_or_default()
            );
            let metadata_json = grow_media::enrich_new_social_post(
                &pool,
                &project,
                &file.filename,
                Some(&description),
                serde_json::json!({
                    "attachment": {
                        "inbox_file_id": file.id,
                        "filename": file.filename,
                        "disk_path": file.disk_path,
                        "content_type": file.content_type,
                        "size_bytes": file.size_bytes,
                    }
                }),
                Some("compose"),
                None,
                None,
            )
            .await
            .map_err(ErrorResponse::internal)?;

            let card = cards::create_card(
                &pool,
                cards::CreateCard {
                    project_id: project.id.clone(),
                    title: file.filename.clone(),
                    description: Some(description),
                    card_type: Some("social_post".to_string()),
                    column_id: None,
                    created_by: Some("user".to_string()),
                    metadata_json: Some(metadata_json),
                },
            )
            .await
            .map_err(ErrorResponse::internal)?;
            grow_media::enqueue_after_create(pool.clone(), project.id.clone(), card.id.clone());
            ("routed", Some(project.id), None, None, Some(card.id), None)
        }
        other => {
            return Err(ErrorResponse::bad_request(format!(
                "Unknown destination '{other}'. Must be brain, project, or scheduler"
            )));
        }
    };

    let updated = inbox::mark_inbox_file(&pool, &file.id, new_status, project_stamp.as_deref())
        .await
        .map_err(ErrorResponse::internal)?
        .ok_or_else(|| ErrorResponse::internal("Inbox row vanished while routing"))?;

    tracing::info!(
        file = %updated.id,
        filename = %updated.filename,
        %destination,
        status = %updated.status,
        "inbox file routed"
    );

    Ok(Json(RouteResponse {
        file: updated,
        destination,
        summary,
        document_id,
        card_id,
        memory_key,
    }))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/inbox",
            get(list_inbox_handler).post(create_inbox_handler),
        )
        .route("/api/inbox/{id}/route", post(route_inbox_handler))
        .with_state(state)
}
