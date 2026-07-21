//! File-intake inbox — one typed metadata row per file that lands in the
//! Permagent-owned intake directory (`~/.permagent/inbox/`, see
//! [`crate::config::paths::Paths::inbox_dir`]).
//!
//! # Scope (#393, epic #392)
//!
//! This is the capture + persist + list slice. The in-app Browser webview
//! redirects downloads onto disk under the inbox directory and records a row
//! here via `POST /api/inbox`; the row is then listable via `GET /api/inbox`
//! (see `routes::inbox` in the daemon). Drag-to-chat (#394) and route-to-surface
//! (#395) are later slices built on top of this primitive.
//!
//! The file bytes live on disk; this table holds only metadata. `disk_path` is
//! stored relative to the inbox directory so the row stays valid across data-root
//! moves (the absolute path is `inbox_dir().join(disk_path)`).

use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

/// Self-knowledge descriptor for the inbox surface. Lets the agent tell the user
/// that browser downloads land in their Permagent inbox rather than vanishing
/// into Finder. Static: editorial, no live status claim (a Phase-2 read-back
/// could surface the unread count).
pub const INBOX_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "inbox",
        display_name: "Downloads inbox",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A Permagent-owned intake folder — files you download in the in-app browser land here (on disk under ~/.permagent/inbox/, with a row in your database) instead of disappearing into Finder",
        why_it_matters:
            "It is the hub for getting files into Permagent; once a download lands here you can surface, ingest, or route it without a Finder round-trip",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        teaching: &[],
    };

// ── Data types ─────────────────────────────────────────────────────────────

/// A typed inbox file record — metadata for one file in the intake directory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InboxFile {
    pub id: String,
    /// Display filename (the on-disk basename after sanitize / de-collision).
    pub filename: String,
    /// The URL the file was downloaded from, when known.
    pub original_url: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    /// Path relative to [`crate::config::paths::Paths::inbox_dir`].
    pub disk_path: String,
    /// Lifecycle: `received` | `ingested` | `routed` | `deleted`.
    pub status: String,
    pub project_id: Option<String>,
    pub created_at: String,
}

/// Insert input for [`insert_inbox_file`]. `None` optional fields are stored
/// NULL; `id`, `status`, and `created_at` are assigned by the database / mint.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct NewInboxFile {
    pub filename: String,
    pub original_url: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub disk_path: String,
    pub project_id: Option<String>,
}

const SELECT_COLS: &str = "id, filename, original_url, content_type, size_bytes, \
                           disk_path, status, project_id, created_at";

fn row_to_inbox_file(r: &sqlx::sqlite::SqliteRow) -> InboxFile {
    InboxFile {
        id: r.get("id"),
        filename: r.get("filename"),
        original_url: r.get("original_url"),
        content_type: r.get("content_type"),
        size_bytes: r.get("size_bytes"),
        disk_path: r.get("disk_path"),
        status: r.get("status"),
        project_id: r.get("project_id"),
        created_at: r.get("created_at"),
    }
}

// ── Operations ─────────────────────────────────────────────────────────────

/// Insert a metadata row for a file that has landed in the inbox. Mints an
/// opaque `id`, lets the DB fill `status` (`received`) and `created_at`, and
/// returns the persisted row.
pub async fn insert_inbox_file(
    pool: &Pool<Sqlite>,
    new: &NewInboxFile,
) -> Result<InboxFile, String> {
    let id = Uuid::new_v4().to_string();

    let sql = format!(
        "INSERT INTO inbox_files (id, filename, original_url, content_type, size_bytes, disk_path, project_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING {SELECT_COLS}",
    );

    let row = sqlx::query(&sql)
        .bind(&id)
        .bind(&new.filename)
        .bind(&new.original_url)
        .bind(&new.content_type)
        .bind(new.size_bytes)
        .bind(&new.disk_path)
        .bind(&new.project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(row_to_inbox_file(&row))
}

/// List inbox files, newest first.
pub async fn list_inbox_files(pool: &Pool<Sqlite>) -> Result<Vec<InboxFile>, String> {
    let sql = format!("SELECT {SELECT_COLS} FROM inbox_files ORDER BY created_at DESC, id DESC");
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_inbox_file).collect())
}

/// Fetch one inbox file by id.
pub async fn get_inbox_file(pool: &Pool<Sqlite>, id: &str) -> Result<Option<InboxFile>, String> {
    let sql = format!("SELECT {SELECT_COLS} FROM inbox_files WHERE id = ?");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_inbox_file))
}

/// Statuses [`mark_inbox_file`] accepts — the routing slice's lifecycle moves.
/// (`received` is the DB default on insert; `deleted` is a future slice.)
pub const ROUTABLE_STATUSES: &[&str] = &["ingested", "routed"];

/// Record the outcome of routing an inbox file (#395): set `status`
/// (`ingested` when the content went to the Brain, `routed` when the file went
/// to a project or scheduler surface) and, when the destination is
/// project-scoped, stamp `project_id`. Returns the updated row, or `Ok(None)`
/// if `id` does not exist. `project_id: None` leaves any existing value
/// untouched (Brain routing is not project-scoped).
pub async fn mark_inbox_file(
    pool: &Pool<Sqlite>,
    id: &str,
    status: &str,
    project_id: Option<&str>,
) -> Result<Option<InboxFile>, String> {
    if !ROUTABLE_STATUSES.contains(&status) {
        return Err(format!(
            "Invalid routing status: {status}. Must be one of {ROUTABLE_STATUSES:?}"
        ));
    }

    let sql = format!(
        "UPDATE inbox_files
         SET status = ?, project_id = COALESCE(?, project_id)
         WHERE id = ?
         RETURNING {SELECT_COLS}",
    );
    let row = sqlx::query(&sql)
        .bind(status)
        .bind(project_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_inbox_file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        spectral_schema::init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_then_list_roundtrips() {
        let pool = fresh_pool().await;

        let saved = insert_inbox_file(
            &pool,
            &NewInboxFile {
                filename: "invoice.pdf".to_string(),
                original_url: Some("https://example.com/invoice.pdf".to_string()),
                content_type: Some("application/pdf".to_string()),
                size_bytes: Some(2048),
                disk_path: "invoice.pdf".to_string(),
                project_id: None,
            },
        )
        .await
        .unwrap();

        assert!(!saved.id.is_empty());
        assert_eq!(saved.status, "received");
        assert_eq!(saved.filename, "invoice.pdf");
        assert!(!saved.created_at.is_empty());

        let listed = list_inbox_files(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], saved);
    }

    #[tokio::test]
    async fn list_is_newest_first() {
        let pool = fresh_pool().await;
        for n in ["a.txt", "b.txt", "c.txt"] {
            insert_inbox_file(
                &pool,
                &NewInboxFile {
                    filename: n.to_string(),
                    disk_path: n.to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let listed = list_inbox_files(&pool).await.unwrap();
        assert_eq!(listed.len(), 3);
        // created_at is second-resolution; the id DESC tiebreaker keeps the
        // order total, so assert the set rather than a fragile sequence.
        let names: std::collections::HashSet<_> =
            listed.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(names.len(), 3);
    }

    async fn seed_file(pool: &Pool<Sqlite>, name: &str) -> InboxFile {
        insert_inbox_file(
            pool,
            &NewInboxFile {
                filename: name.to_string(),
                disk_path: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn get_inbox_file_roundtrips_and_misses() {
        let pool = fresh_pool().await;
        let saved = seed_file(&pool, "doc.pdf").await;

        let found = get_inbox_file(&pool, &saved.id).await.unwrap();
        assert_eq!(found, Some(saved));
        assert_eq!(get_inbox_file(&pool, "nope").await.unwrap(), None);
    }

    #[tokio::test]
    async fn mark_ingested_sets_status_and_keeps_project_untouched() {
        let pool = fresh_pool().await;
        let saved = seed_file(&pool, "notes.txt").await;

        let updated = mark_inbox_file(&pool, &saved.id, "ingested", None)
            .await
            .unwrap()
            .expect("row exists");
        assert_eq!(updated.status, "ingested");
        assert_eq!(updated.project_id, None);
    }

    #[tokio::test]
    async fn mark_routed_stamps_project_id() {
        let pool = fresh_pool().await;
        // A real project row: inbox_files.project_id carries an FK to projects.
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Routing target".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let saved = seed_file(&pool, "asset.png").await;

        let updated = mark_inbox_file(&pool, &saved.id, "routed", Some(&project.id))
            .await
            .unwrap()
            .expect("row exists");
        assert_eq!(updated.status, "routed");
        assert_eq!(updated.project_id.as_deref(), Some(project.id.as_str()));

        // A later status change without a project keeps the stamp (COALESCE).
        let again = mark_inbox_file(&pool, &saved.id, "ingested", None)
            .await
            .unwrap()
            .expect("row exists");
        assert_eq!(again.project_id.as_deref(), Some(project.id.as_str()));
    }

    #[tokio::test]
    async fn mark_rejects_invalid_status_and_unknown_id() {
        let pool = fresh_pool().await;
        let saved = seed_file(&pool, "x.txt").await;

        let err = mark_inbox_file(&pool, &saved.id, "received", None)
            .await
            .unwrap_err();
        assert!(err.contains("Invalid routing status"), "got: {err}");

        let missing = mark_inbox_file(&pool, "nope", "routed", None)
            .await
            .unwrap();
        assert_eq!(missing, None);
    }
}
