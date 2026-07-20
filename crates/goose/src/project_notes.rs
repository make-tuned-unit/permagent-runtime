//! Project notes — per-project freeform notes the user (or agent) writes.
//!
//! A project owns a set of notes: a title (optional) plus a body, kept in a
//! [`project_notes`] row keyed by `project_id` (FK, cascades on project delete).
//! Unlike [`crate::project_documents`] (a file on disk), a note is text the user
//! types (or dictates) directly. Its text is also indexed into the Brain by the
//! route layer — `memory_key` records the Brain key it was written under — so a
//! note is recallable and Librarian-enriched just like a dropped document.
//!
//! This module owns the DB rows plus the ONE composed write path,
//! [`create_note_indexed`]: row insert + best-effort Brain index + project
//! association. Both note writers — the HTTP route
//! (`POST /api/projects/{id}/notes`) and the `file_to_project` decision effect —
//! go through it, so the note contract (durable non-activity source
//! [`NOTE_SOURCE`], description-less write, Librarian enrichment) is enforced in
//! exactly one place.

use sqlx::{Pool, Row, Sqlite};

/// `source` tag on every memory a note writes. Deliberately NOT
/// `permagent.activity` (which pruning/consolidation reap) so notes are durable,
/// and description-less on write so the Librarian claims + enriches them exactly
/// as it does Reader-ingested documents.
pub const NOTE_SOURCE: &str = "permagent.note";

/// The Brain content for a note: title + body when a title is present, else the
/// body alone. Mirrors how the Reader stores the full text of a document.
pub fn note_memory_content(title: Option<&str>, body: &str) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("{t}\n\n{body}"),
        None => body.to_string(),
    }
}

/// Create a project note through the ONE composed path: insert the durable row,
/// best-effort index its text into the Brain (distinct durable source, Private,
/// NO description — the standing Librarian contract), and associate the
/// resulting memory with the project.
///
/// The row is the durable record; a Brain failure is logged and skipped — the
/// note still persists and is returned (`memory_key` stays `None`). Pass
/// `brain: None` when no Brain is mounted.
pub async fn create_note_indexed(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    title: Option<&str>,
    body: &str,
) -> Result<ProjectNote, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("note body is empty".to_string());
    }
    let title = title.map(str::trim).filter(|t| !t.is_empty());

    let note_id = uuid::Uuid::now_v7().to_string();
    let memory_key = format!("note:{}:{}", project_id, note_id);

    // Best-effort Brain index. On success we resolve the memory id and associate
    // it with the project; the memory_key is recorded on the row regardless (it
    // is stable/deterministic) so a later reconcile could re-associate.
    let mut stored_memory_key: Option<&str> = None;
    if let Some(brain) = brain {
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
                            crate::project_association::associate_memory(pool, project_id, &mem.id)
                                .await
                        {
                            tracing::warn!(project = %project_id, note = %note_id, error = %e, "project note brain association failed");
                        } else {
                            tracing::info!(project = %project_id, note = %note_id, memory = %mem.id, "project note indexed into brain");
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(project = %project_id, note = %note_id, key = %memory_key, "project note written but its memory was not found for association")
                    }
                    Err(e) => {
                        tracing::warn!(project = %project_id, note = %note_id, error = %e, "project note memory lookup failed")
                    }
                }
            }
            Err(e) => {
                tracing::warn!(project = %project_id, note = %note_id, error = %e, "project note brain write failed (note still saved)")
            }
        }
    }

    insert_note(pool, &note_id, project_id, title, body, stored_memory_key).await
}

/// One project note: the DB row. `memory_key` is the Brain key the note's text
/// was indexed under (`None` if the Brain write was skipped/failed — the row is
/// still the durable record of the note).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectNote {
    pub id: String,
    pub project_id: String,
    pub title: Option<String>,
    pub body: String,
    pub memory_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Insert a new project-note row. Returns the full [`ProjectNote`] (with the
/// DB-assigned timestamps) so the caller can return it directly.
pub async fn insert_note(
    pool: &Pool<Sqlite>,
    id: &str,
    project_id: &str,
    title: Option<&str>,
    body: &str,
    memory_key: Option<&str>,
) -> Result<ProjectNote, String> {
    sqlx::query(
        "INSERT INTO project_notes (id, project_id, title, body, memory_key) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(project_id)
    .bind(title)
    .bind(body)
    .bind(memory_key)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get_note(pool, project_id, id)
        .await?
        .ok_or_else(|| "note vanished immediately after insert".to_string())
}

/// List a project's notes, newest first.
pub async fn list_notes(pool: &Pool<Sqlite>, project_id: &str) -> Result<Vec<ProjectNote>, String> {
    let rows = sqlx::query(
        "SELECT id, project_id, title, body, memory_key, created_at, updated_at \
         FROM project_notes WHERE project_id = ? \
         ORDER BY created_at DESC, id ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(row_to_note).collect())
}

/// Fetch one note scoped to its project.
pub async fn get_note(
    pool: &Pool<Sqlite>,
    project_id: &str,
    note_id: &str,
) -> Result<Option<ProjectNote>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, title, body, memory_key, created_at, updated_at \
         FROM project_notes WHERE id = ? AND project_id = ?",
    )
    .bind(note_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_note))
}

/// Delete a note (project-scoped). Returns the note's `memory_key` (if any) when
/// a row existed, so the caller can best-effort disassociate its Brain memory.
/// The outer `Option` distinguishes "no such note" (`None`) from "deleted, but
/// it had no memory_key" (`Some(None)`).
pub async fn delete_note(
    pool: &Pool<Sqlite>,
    project_id: &str,
    note_id: &str,
) -> Result<Option<Option<String>>, String> {
    let existing = get_note(pool, project_id, note_id).await?;
    let Some(note) = existing else {
        return Ok(None);
    };

    sqlx::query("DELETE FROM project_notes WHERE id = ? AND project_id = ?")
        .bind(note_id)
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(Some(note.memory_key))
}

fn row_to_note(r: &sqlx::sqlite::SqliteRow) -> ProjectNote {
    ProjectNote {
        id: r.get("id"),
        project_id: r.get("project_id"),
        title: r.get("title"),
        body: r.get("body"),
        memory_key: r.get("memory_key"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
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

    /// Acceptance: a note attaches to a project and lists back.
    #[tokio::test]
    async fn insert_and_list_back() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;

        let note = insert_note(
            &pool,
            "note-1",
            &proj,
            Some("Kickoff"),
            "Ship the beta by Friday.",
            Some("note:acme:note-1"),
        )
        .await
        .unwrap();
        assert_eq!(note.title.as_deref(), Some("Kickoff"));
        assert_eq!(note.body, "Ship the beta by Friday.");
        assert_eq!(note.memory_key.as_deref(), Some("note:acme:note-1"));
        assert!(!note.created_at.is_empty());

        let notes = list_notes(&pool, &proj).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "note-1");
        assert_eq!(notes[0].project_id, proj);
    }

    /// A note can be titleless (body-only) and still round-trips.
    #[tokio::test]
    async fn titleless_note_roundtrips() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let note = insert_note(&pool, "note-1", &proj, None, "just a body", None)
            .await
            .unwrap();
        assert!(note.title.is_none());
        assert!(note.memory_key.is_none());
        assert_eq!(list_notes(&pool, &proj).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_and_delete_are_project_scoped() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let other = a_project(&pool, "Other").await;
        insert_note(&pool, "note-1", &proj, None, "hi", Some("note:acme:note-1"))
            .await
            .unwrap();

        // Wrong project can neither read nor delete it.
        assert!(get_note(&pool, &other, "note-1").await.unwrap().is_none());
        assert!(delete_note(&pool, &other, "note-1")
            .await
            .unwrap()
            .is_none());

        // Correct project reads it, then deletes once (returning its memory_key).
        assert!(get_note(&pool, &proj, "note-1").await.unwrap().is_some());
        assert_eq!(
            delete_note(&pool, &proj, "note-1").await.unwrap(),
            Some(Some("note:acme:note-1".to_string()))
        );
        // Second delete is a no-op.
        assert!(delete_note(&pool, &proj, "note-1").await.unwrap().is_none());
        assert!(list_notes(&pool, &proj).await.unwrap().is_empty());
    }

    /// The composed path without a Brain: the row persists (memory_key None),
    /// trimming applies, and an empty body is refused — the same contract the
    /// HTTP route enforces, now shared with the file_to_project effect.
    #[tokio::test]
    async fn create_note_indexed_brainless_persists_row() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;

        let note = create_note_indexed(&pool, None, &proj, Some("  Kickoff  "), "  body text  ")
            .await
            .unwrap();
        assert_eq!(note.title.as_deref(), Some("Kickoff"));
        assert_eq!(note.body, "body text");
        assert!(note.memory_key.is_none(), "no Brain -> no memory_key");
        assert_eq!(list_notes(&pool, &proj).await.unwrap().len(), 1);

        // Empty (whitespace-only) body is refused, and a blank title is None.
        assert!(create_note_indexed(&pool, None, &proj, None, "   ")
            .await
            .is_err());
        let untitled = create_note_indexed(&pool, None, &proj, Some("   "), "more")
            .await
            .unwrap();
        assert!(untitled.title.is_none());
    }

    /// note_memory_content mirrors the Reader: title + body, or body alone.
    #[test]
    fn note_memory_content_shapes() {
        assert_eq!(note_memory_content(Some("T"), "b"), "T\n\nb");
        assert_eq!(note_memory_content(None, "b"), "b");
        assert_eq!(note_memory_content(Some("  "), "b"), "b");
    }

    #[tokio::test]
    async fn deleting_project_cascades_notes() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        insert_note(&pool, "note-1", &proj, None, "hi", None)
            .await
            .unwrap();

        crate::projects::delete_project(&pool, &proj).await.unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_notes WHERE project_id = ?")
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
        crate::session::spectral_schema::apply_project_notes_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v24_to_v25(&pool)
            .await
            .unwrap();
    }
}
