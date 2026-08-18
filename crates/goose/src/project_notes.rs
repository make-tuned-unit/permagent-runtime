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
use std::path::{Path, PathBuf};

use crate::config::paths::Paths;
use crate::meeting_writeup::WriteupProvenance;

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
    create_note_indexed_with_id(pool, brain, &note_id, project_id, title, body).await
}

/// Create a note with a caller-supplied stable id.
///
/// Durable decision effects use a decision-derived id so replay cannot create
/// a duplicate note.
pub async fn create_note_indexed_with_id(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    note_id: &str,
    project_id: &str,
    title: Option<&str>,
    body: &str,
) -> Result<ProjectNote, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("note body is empty".to_string());
    }
    let title = title.map(str::trim).filter(|t| !t.is_empty());

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

    insert_note(pool, note_id, project_id, title, body, stored_memory_key).await
}

/// Attach the permagent-owned markdown export path for API responses.
pub fn with_export_path(mut note: ProjectNote, project_slug: &str) -> ProjectNote {
    note.export_path = Some(
        note_export_path(project_slug, &note)
            .to_string_lossy()
            .into_owned(),
    );
    note
}

/// Slug for the markdown filename — title when present, else a short id prefix.
pub fn note_filename_slug(note: &ProjectNote) -> String {
    note.title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(crate::projects::slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| note.id.chars().take(8).collect())
}

/// Permagent-owned export path: `notes/<project-slug>/<YYYY-MM-DD>-<note-slug>.md`.
pub fn note_export_path(project_slug: &str, note: &ProjectNote) -> PathBuf {
    note_export_path_in(&Paths::in_state_dir("notes"), project_slug, note)
}

pub(crate) fn note_export_path_in(
    notes_root: &Path,
    project_slug: &str,
    note: &ProjectNote,
) -> PathBuf {
    let date = note.created_at.get(..10).unwrap_or("unknown-date");
    let slug = note_filename_slug(note);
    notes_root
        .join(project_slug)
        .join(format!("{date}-{slug}.md"))
}

fn yaml_escape(value: &str) -> String {
    if value.contains([':', '#', '\n', '"', '\'']) {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Render a note as markdown with YAML frontmatter (export-only; DB row is canonical).
pub fn render_note_markdown(
    project_name: &str,
    note: &ProjectNote,
    kind: Option<&str>,
    provenance: Option<&WriteupProvenance>,
) -> String {
    let title = note
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Untitled note");
    let date = note.created_at.get(..10).unwrap_or("unknown-date");
    let kind = kind.unwrap_or("note");
    let mut frontmatter = format!(
        "---\ntitle: {}\nproject: {}\ndate: {}\nkind: {}\n",
        yaml_escape(title),
        yaml_escape(project_name),
        yaml_escape(date),
        yaml_escape(kind),
    );
    if let Some(p) = provenance {
        frontmatter.push_str(&format!(
            "writeup_provider: {}\nwriteup_model: {}\nwriteup_privacy: {}\n",
            yaml_escape(&p.provider),
            yaml_escape(&p.model),
            yaml_escape(match p.privacy {
                crate::meeting_writeup::WriteupPrivacy::LocalOnly => "local_only",
                crate::meeting_writeup::WriteupPrivacy::TrustedPool => "trusted_pool",
                crate::meeting_writeup::WriteupPrivacy::CloudSession => "cloud_session",
            }),
        ));
    }
    frontmatter.push_str("---\n\n");
    format!("{frontmatter}{}", note.body)
}

async fn write_note_export_inner(
    notes_root: &Path,
    project_slug: &str,
    project_name: &str,
    note: &ProjectNote,
    kind: Option<&str>,
    provenance: Option<&WriteupProvenance>,
) -> Result<PathBuf, String> {
    let path = note_export_path_in(notes_root, project_slug, note);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create export dir: {e}"))?;
    }
    let markdown = render_note_markdown(project_name, note, kind, provenance);
    tokio::fs::write(&path, markdown)
        .await
        .map_err(|e| format!("write export file: {e}"))?;
    Ok(path)
}

/// Best-effort markdown export on save / rewrite. Failures are logged only.
pub async fn write_note_export(
    project_slug: &str,
    project_name: &str,
    note: &ProjectNote,
    kind: Option<&str>,
    provenance: Option<&WriteupProvenance>,
) {
    if let Err(error) = write_note_export_inner(
        &Paths::in_state_dir("notes"),
        project_slug,
        project_name,
        note,
        kind,
        provenance,
    )
    .await
    {
        tracing::warn!(
            project = %note.project_id,
            note = %note.id,
            %error,
            "project note markdown export failed (note still saved)"
        );
    }
}

#[cfg(test)]
pub(crate) async fn write_note_export_to_root(
    notes_root: &Path,
    project_slug: &str,
    project_name: &str,
    note: &ProjectNote,
    kind: Option<&str>,
    provenance: Option<&WriteupProvenance>,
) -> Result<PathBuf, String> {
    write_note_export_inner(
        notes_root,
        project_slug,
        project_name,
        note,
        kind,
        provenance,
    )
    .await
}

/// Replace a note's body in place and re-index it into the Brain under the
/// SAME memory key, so the enriched text supersedes the original rather than
/// accumulating a second memory for the same note.
///
/// Exists for the meeting-note enhancement pass: the raw transcript is saved
/// immediately (words are never dropped, even if the model call fails), then a
/// background pass rewrites the body into structured notes. Best-effort on the
/// Brain half, like the create path — the row is the durable record.
pub async fn update_note_body(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    note_id: &str,
    project_id: &str,
    body: &str,
) -> Result<(), String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("note body is empty".to_string());
    }
    let existing = get_note(pool, project_id, note_id).await?;
    let title = existing.as_ref().and_then(|n| n.title.clone());

    sqlx::query(
        "UPDATE project_notes SET body = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id = ? AND project_id = ?",
    )
    .bind(body)
    .bind(note_id)
    .bind(project_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(brain) = brain {
        let memory_key = format!("note:{}:{}", project_id, note_id);
        let content = note_memory_content(title.as_deref(), body);
        let opts = spectral::RememberOpts {
            source: Some(NOTE_SOURCE.to_string()),
            visibility: spectral::Visibility::Private,
            ..Default::default()
        };
        if let Err(e) = brain.remember_with(&memory_key, &content, opts).await {
            tracing::warn!(
                project = %project_id, note = %note_id, error = %e,
                "note body updated but its brain re-index failed"
            );
        }
    }
    Ok(())
}

/// One project note: the DB row. `memory_key` is the Brain key the note's text
/// was indexed under (`None` if the Brain write was skipped/failed — the row is
/// still the durable record of the note). `export_path` is the permagent-owned
/// markdown export path (computed, not stored in SQLite).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectNote {
    pub id: String,
    pub project_id: String,
    pub title: Option<String>,
    pub body: String,
    pub memory_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_path: Option<String>,
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
        export_path: None,
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

    /// Guard 5: markdown export exists after save and is rewritten after write-up.
    #[tokio::test]
    async fn guard_markdown_export_written_on_save_and_rewrite() {
        let pool = test_pool().await;
        let proj_id = a_project(&pool, "Acme").await;
        let slug = "acme";
        let root = tempfile::tempdir().unwrap();
        let note = insert_note(
            &pool,
            "note-meeting",
            &proj_id,
            Some("Standup"),
            "raw transcript only",
            None,
        )
        .await
        .unwrap();
        let raw_path =
            write_note_export_to_root(root.path(), slug, "Acme", &note, Some("meeting"), None)
                .await
                .unwrap();
        let raw_bytes = std::fs::read(&raw_path).unwrap();
        assert!(!raw_bytes.is_empty(), "export must not be zero bytes");
        assert!(String::from_utf8_lossy(&raw_bytes).contains("raw transcript only"));

        let provenance = WriteupProvenance {
            provider: "ollama".to_string(),
            model: "qwen3".to_string(),
            privacy: crate::meeting_writeup::WriteupPrivacy::LocalOnly,
        };
        let structured = "## Summary\n\n- shipped\n\n## Transcript\n\nraw transcript only";
        update_note_body(&pool, None, &note.id, &proj_id, structured)
            .await
            .unwrap();
        let updated = get_note(&pool, &proj_id, &note.id).await.unwrap().unwrap();
        let structured_path = write_note_export_to_root(
            root.path(),
            slug,
            "Acme",
            &updated,
            Some("meeting"),
            Some(&provenance),
        )
        .await
        .unwrap();
        assert_eq!(raw_path, structured_path);
        let structured_bytes = std::fs::read(&structured_path).unwrap();
        let structured_text = String::from_utf8_lossy(&structured_bytes);
        assert!(structured_bytes.len() > raw_bytes.len());
        assert!(structured_text.contains("writeup_provider: ollama"));
        assert!(structured_text.contains("## Summary"));
        assert!(structured_text.contains("raw transcript only"));
    }

    /// Guard 6: a failed file write leaves the note saved (export errors do not propagate).
    #[tokio::test]
    async fn guard_export_failure_does_not_block_note_save() {
        let pool = test_pool().await;
        let proj_id = a_project(&pool, "Acme").await;
        let note = insert_note(&pool, "note-1", &proj_id, None, "still here", None)
            .await
            .unwrap();
        let blocking_file = tempfile::NamedTempFile::new().unwrap();
        let err =
            write_note_export_to_root(blocking_file.path(), "acme", "Acme", &note, None, None)
                .await
                .unwrap_err();
        assert!(err.contains("export"), "expected export error, got: {err}");
        let fetched = get_note(&pool, &proj_id, "note-1").await.unwrap().unwrap();
        assert_eq!(fetched.body, "still here");
    }

    /// Guard 7: export file is non-empty and contains the transcript text.
    #[tokio::test]
    async fn guard_export_file_nonempty_and_contains_transcript() {
        let pool = test_pool().await;
        let proj_id = a_project(&pool, "Acme").await;
        let transcript = "Alice: we ship Friday\nBob: agreed";
        let note = insert_note(&pool, "note-2", &proj_id, Some("Sync"), transcript, None)
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let path =
            write_note_export_to_root(root.path(), "acme", "Acme", &note, Some("meeting"), None)
                .await
                .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(transcript));
    }
}
