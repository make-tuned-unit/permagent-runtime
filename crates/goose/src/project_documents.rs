//! Project documents — the per-project document hub (#471 Layer 2).
//!
//! A project owns a set of documents (decks, PDFs, briefs, contracts). Each is
//! a file on disk under `~/.permagent/project-docs/<project_id>/<id>/<filename>`
//! plus a [`project_documents`] row keyed by `project_id` (FK, cascades on
//! project delete). The row carries the stored `mime_type` so the in-app viewer
//! can dispatch a content-type renderer without sniffing the bytes.
//!
//! This mirrors the session-scoped [`crate::attachments`] pipeline (multipart →
//! disk write → streamed download) but scopes the relation to a project rather
//! than a chat session — the same storage mechanism, a different owner. Route
//! handlers in `permagent-daemon` delegate to these functions so the server
//! crate keeps no direct `sqlx` dependency.

use sqlx::{Pool, Row, Sqlite};

/// One project document: the DB row. `path` is the absolute on-disk location;
/// resolving/streaming it is the route layer's job.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectDocument {
    pub id: String,
    pub project_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub path: String,
    pub uploaded_at: String,
}

/// Insert a new project-document row. Returns `uploaded_at`.
///
/// The single writer both the multipart upload route and the inbox `project`
/// route (`save_project_document`, shared by both) go through, so this is
/// where `project_changed(project_id, "documents")` belongs (#629 one-writer
/// rule) — emitting it here, rather than in each HTTP handler, is what makes
/// EVERY caller announce, including the ones a future caller adds without
/// remembering to emit anything itself.
pub async fn insert_document(
    pool: &Pool<Sqlite>,
    id: &str,
    project_id: &str,
    filename: &str,
    mime_type: &str,
    size_bytes: i64,
    path: &str,
) -> Result<String, String> {
    sqlx::query(
        "INSERT INTO project_documents \
         (id, project_id, filename, mime_type, size_bytes, path) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(project_id)
    .bind(filename)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let uploaded_at =
        sqlx::query_scalar::<_, String>("SELECT uploaded_at FROM project_documents WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    crate::events::emit(crate::events::project_changed(project_id, "documents"));

    Ok(uploaded_at)
}

/// Count of documents attached to a project — `observe_projects`' cheap
/// per-project figure (LIST_LIMIT-bounded, so a COUNT beats fetching every
/// row).
pub async fn count_documents(pool: &Pool<Sqlite>, project_id: &str) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM project_documents WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

/// List a project's documents, newest first.
pub async fn list_documents(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<ProjectDocument>, String> {
    let rows = sqlx::query(
        "SELECT id, project_id, filename, mime_type, size_bytes, path, uploaded_at \
         FROM project_documents WHERE project_id = ? \
         ORDER BY uploaded_at DESC, filename ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(row_to_document).collect())
}

/// Fetch one document scoped to its project (used to stream / download it).
pub async fn get_document(
    pool: &Pool<Sqlite>,
    project_id: &str,
    document_id: &str,
) -> Result<Option<ProjectDocument>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, filename, mime_type, size_bytes, path, uploaded_at \
         FROM project_documents WHERE id = ? AND project_id = ?",
    )
    .bind(document_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_document))
}

/// Delete a document (project-scoped). Returns its on-disk `path` if a row
/// existed, so the caller can remove the file.
pub async fn delete_document(
    pool: &Pool<Sqlite>,
    project_id: &str,
    document_id: &str,
) -> Result<Option<String>, String> {
    let path = sqlx::query_scalar::<_, String>(
        "SELECT path FROM project_documents WHERE id = ? AND project_id = ?",
    )
    .bind(document_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if path.is_some() {
        sqlx::query("DELETE FROM project_documents WHERE id = ? AND project_id = ?")
            .bind(document_id)
            .bind(project_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        // One-writer rule (#629): the delete route used to emit this itself;
        // it now lives here so any other caller of `delete_document` announces
        // too, same reasoning as `insert_document` above.
        crate::events::emit(crate::events::project_changed(project_id, "documents"));
    }

    Ok(path)
}

fn row_to_document(r: &sqlx::sqlite::SqliteRow) -> ProjectDocument {
    ProjectDocument {
        id: r.get("id"),
        project_id: r.get("project_id"),
        filename: r.get("filename"),
        mime_type: r.get("mime_type"),
        size_bytes: r.get("size_bytes"),
        path: r.get("path"),
        uploaded_at: r.get("uploaded_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{create_project, CreateProject};
    use crate::session::spectral_schema::init_spectral_db;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // The FK cascade on project delete relies on the pragma the app sets.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn a_project(pool: &Pool<Sqlite>, name: &str) -> String {
        create_project(
            pool,
            CreateProject {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    async fn attach(pool: &Pool<Sqlite>, project: &str, id: &str, name: &str, mime: &str) {
        insert_document(
            pool,
            id,
            project,
            name,
            mime,
            123,
            &format!("/tmp/{id}/{name}"),
        )
        .await
        .unwrap();
    }

    /// Acceptance: a document attaches to a project and lists back.
    #[tokio::test]
    async fn attach_and_list_back() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;

        attach(&pool, &proj, "doc-1", "deck.pdf", "application/pdf").await;

        let docs = list_documents(&pool, &proj).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].filename, "deck.pdf");
        assert_eq!(docs[0].mime_type, "application/pdf");
        assert_eq!(docs[0].project_id, proj);
        assert!(!docs[0].uploaded_at.is_empty());
    }

    #[tokio::test]
    async fn get_and_delete_are_project_scoped() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let other = a_project(&pool, "Other").await;
        attach(&pool, &proj, "doc-1", "brief.md", "text/markdown").await;

        // Wrong project can neither read nor delete it.
        assert!(get_document(&pool, &other, "doc-1")
            .await
            .unwrap()
            .is_none());
        assert!(delete_document(&pool, &other, "doc-1")
            .await
            .unwrap()
            .is_none());

        // Correct project reads the on-disk path, then deletes once.
        let doc = get_document(&pool, &proj, "doc-1").await.unwrap().unwrap();
        assert_eq!(doc.path, "/tmp/doc-1/brief.md");
        assert_eq!(
            delete_document(&pool, &proj, "doc-1").await.unwrap(),
            Some("/tmp/doc-1/brief.md".to_string())
        );
        // Second delete is a no-op.
        assert!(delete_document(&pool, &proj, "doc-1")
            .await
            .unwrap()
            .is_none());
        assert!(list_documents(&pool, &proj).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_project_cascades_documents() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        attach(&pool, &proj, "doc-1", "a.txt", "text/plain").await;

        crate::projects::delete_project(&pool, &proj).await.unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_documents WHERE project_id = ?")
                .bind(&proj)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        let pool = test_pool().await;
        // Re-running the apply step on an initialized DB must not error.
        crate::session::spectral_schema::apply_project_documents_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v23_to_v24(&pool)
            .await
            .unwrap();
    }
}
