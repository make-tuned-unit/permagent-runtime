//! Attachments CRUD — database operations for file upload/download (Phase 2 Track 1).
//!
//! Route handlers in `permagent-daemon` delegate to these functions so the
//! server crate does not need a direct `sqlx` dependency.

use sqlx::{Pool, Row, Sqlite};

pub struct AttachmentRecord {
    pub id: String,
    pub session_id: String,
    pub message_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub path: String,
    pub created_at: String,
}

/// Insert a new attachment row. Returns created_at.
pub async fn insert_attachment(
    pool: &Pool<Sqlite>,
    id: &str,
    session_id: &str,
    filename: &str,
    mime_type: &str,
    size_bytes: i64,
    path: &str,
) -> anyhow::Result<String> {
    sqlx::query(
        "INSERT INTO attachments (id, session_id, filename, mime_type, size_bytes, path)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(session_id)
    .bind(filename)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(path)
    .execute(pool)
    .await?;

    let created_at =
        sqlx::query_scalar::<_, String>("SELECT created_at FROM attachments WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;

    Ok(created_at)
}

/// Fetch an attachment by ID scoped to a session.
pub async fn get_attachment(
    pool: &Pool<Sqlite>,
    session_id: &str,
    attachment_id: &str,
) -> anyhow::Result<Option<AttachmentRecord>> {
    let row = sqlx::query(
        "SELECT id, session_id, message_id, filename, mime_type, size_bytes, path, created_at
         FROM attachments WHERE id = ? AND session_id = ?",
    )
    .bind(attachment_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AttachmentRecord {
        id: r.get("id"),
        session_id: r.get("session_id"),
        message_id: r.get("message_id"),
        filename: r.get("filename"),
        mime_type: r.get("mime_type"),
        size_bytes: r.get("size_bytes"),
        path: r.get("path"),
        created_at: r.get("created_at"),
    }))
}

/// Get attachment by ID only (no session scope, for agent tool use).
pub async fn get_attachment_by_id(
    pool: &Pool<Sqlite>,
    attachment_id: &str,
) -> anyhow::Result<Option<AttachmentRecord>> {
    let row = sqlx::query(
        "SELECT id, session_id, message_id, filename, mime_type, size_bytes, path, created_at
         FROM attachments WHERE id = ?",
    )
    .bind(attachment_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AttachmentRecord {
        id: r.get("id"),
        session_id: r.get("session_id"),
        message_id: r.get("message_id"),
        filename: r.get("filename"),
        mime_type: r.get("mime_type"),
        size_bytes: r.get("size_bytes"),
        path: r.get("path"),
        created_at: r.get("created_at"),
    }))
}

/// Check if a session exists.
pub async fn session_exists(pool: &Pool<Sqlite>, session_id: &str) -> anyhow::Result<bool> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM sessions WHERE id = ?)")
            .bind(session_id)
            .fetch_one(pool)
            .await?;

    Ok(exists)
}

/// Delete an attachment by ID. Returns the file path if it existed.
pub async fn delete_attachment(
    pool: &Pool<Sqlite>,
    session_id: &str,
    attachment_id: &str,
) -> anyhow::Result<Option<String>> {
    let path = sqlx::query_scalar::<_, String>(
        "SELECT path FROM attachments WHERE id = ? AND session_id = ?",
    )
    .bind(attachment_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    if path.is_some() {
        sqlx::query("DELETE FROM attachments WHERE id = ? AND session_id = ?")
            .bind(attachment_id)
            .bind(session_id)
            .execute(pool)
            .await?;
    }

    Ok(path)
}

/// Link an attachment to a message, scoped to the session that owns it.
///
/// Returns `false` when the attachment does not belong to `session_id`; this
/// keeps a guessed attachment UUID from being rebound across conversations.
pub async fn link_to_message_for_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
    attachment_id: &str,
    message_id: &str,
) -> anyhow::Result<bool> {
    let result =
        sqlx::query("UPDATE attachments SET message_id = ? WHERE id = ? AND session_id = ?")
            .bind(message_id)
            .bind(attachment_id)
            .bind(session_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() == 1)
}

/// Atomically link a batch of attachments to one message. If any ID is not
/// owned by the session, every update is rolled back.
pub async fn link_many_to_message_for_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
    attachment_ids: &[String],
    message_id: &str,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    for attachment_id in attachment_ids {
        let result =
            sqlx::query("UPDATE attachments SET message_id = ? WHERE id = ? AND session_id = ?")
                .bind(message_id)
                .bind(attachment_id)
                .bind(session_id)
                .execute(&mut *transaction)
                .await?;
        if result.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(false);
        }
    }
    transaction.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn message_link_cannot_cross_session_boundary() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        insert_attachment(
            &pool,
            "attachment-1",
            "session-owner",
            "story.png",
            "image/png",
            12,
            "/private/attachment",
        )
        .await
        .unwrap();

        assert!(!link_to_message_for_session(
            &pool,
            "session-other",
            "attachment-1",
            "message-wrong",
        )
        .await
        .unwrap());
        assert!(link_to_message_for_session(
            &pool,
            "session-owner",
            "attachment-1",
            "message-right",
        )
        .await
        .unwrap());

        let record = get_attachment(&pool, "session-owner", "attachment-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.message_id.as_deref(), Some("message-right"));
    }

    #[tokio::test]
    async fn batch_link_rolls_back_when_one_attachment_is_out_of_scope() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT,
                filename TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, session) in [("owned", "session-owner"), ("foreign", "session-other")] {
            insert_attachment(
                &pool,
                id,
                session,
                "story.png",
                "image/png",
                12,
                "/private/a",
            )
            .await
            .unwrap();
        }

        let ids = vec!["owned".to_string(), "foreign".to_string()];
        assert!(
            !link_many_to_message_for_session(&pool, "session-owner", &ids, "message-1",)
                .await
                .unwrap()
        );
        let owned = get_attachment(&pool, "session-owner", "owned")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owned.message_id, None);
    }
}
