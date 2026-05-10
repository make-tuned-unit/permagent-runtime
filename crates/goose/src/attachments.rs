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

/// Link an attachment to a message (set message_id after sending).
pub async fn link_to_message(
    pool: &Pool<Sqlite>,
    attachment_id: &str,
    message_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE attachments SET message_id = ? WHERE id = ?")
        .bind(message_id)
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(())
}
