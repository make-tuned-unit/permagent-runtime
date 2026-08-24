//! Project stack organizer (#512) — which services a project is built on and
//! which login identity is used for each.
//!
//! One [`project_stack_entries`] row per service per project: the service name
//! ("Vercel", "Neon"), a category for grouping, the login identity used for
//! THIS project on THIS service ("jesse+kinrows@…"), free-text notes (where the
//! free-tier reality lives), and an optional dashboard URL.
//!
//! REFERENCE-ONLY, BY DESIGN: this module stores NO passwords, tokens, or
//! secrets of any kind, and must never grow a field for one. It answers "which
//! account do I use for Railway on Kinrows?" — the credential itself stays in
//! the user's password manager. #512 researched and ruled out autofill/secret
//! storage (WKWebView Associated-Domains limitation); do not reintroduce it
//! here.
//!
//! v1 keeps `identity` as a plain string per row (an identity reused across
//! projects duplicates — conscious choice, normalize later if it hurts) and
//! stays OUT of the Brain graph (isolated table, no entity entanglement).

use sqlx::{Pool, Row, Sqlite};

/// Valid `category` values. MUST stay in sync with the SQL CHECK constraint in
/// [`crate::session::spectral_schema::apply_project_stack_schema`] — widen BOTH
/// when adding a category, or writes go red at the DB while Rust says yes.
pub const VALID_CATEGORIES: &[&str] = &[
    "hosting",
    "database",
    "backend",
    "auth",
    "analytics",
    "social",
    "domain",
    "other",
];

/// One stack entry: the DB row. `identity` is the account label used to log in
/// (email/username/handle) — never a credential. `dashboard_url` optionally
/// deep-links the service's console (opened via the in-app browser, #506).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StackEntry {
    pub id: String,
    pub project_id: String,
    pub service_name: String,
    pub category: String,
    pub identity: Option<String>,
    pub notes: String,
    pub dashboard_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Field updates for [`update_entry`]. Single `Option` = "leave unchanged";
/// double `Option` = "unchanged / clear to NULL / set" (the same semantics
/// `UpdateProject` uses for `site_url`/`repo_url`).
#[derive(Debug, Default)]
pub struct UpdateStackEntry {
    pub service_name: Option<String>,
    pub category: Option<String>,
    pub identity: Option<Option<String>>,
    pub notes: Option<String>,
    pub dashboard_url: Option<Option<String>>,
}

fn validate_category(category: &str) -> Result<(), String> {
    if VALID_CATEGORIES.contains(&category) {
        Ok(())
    } else {
        Err(format!(
            "Invalid category '{}'. Must be one of: {}",
            category,
            VALID_CATEGORIES.join(", ")
        ))
    }
}

/// Insert a new stack-entry row. Returns the full [`StackEntry`] (with the
/// DB-assigned timestamps) so the caller can return it directly.
#[allow(clippy::too_many_arguments)] // one arg per column, mirrors insert_note
pub async fn insert_entry(
    pool: &Pool<Sqlite>,
    id: &str,
    project_id: &str,
    service_name: &str,
    category: &str,
    identity: Option<&str>,
    notes: &str,
    dashboard_url: Option<&str>,
) -> Result<StackEntry, String> {
    let service_name = service_name.trim();
    if service_name.is_empty() {
        return Err("service_name is empty".to_string());
    }
    validate_category(category)?;

    sqlx::query(
        "INSERT INTO project_stack_entries \
         (id, project_id, service_name, category, identity, notes, dashboard_url) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(project_id)
    .bind(service_name)
    .bind(category)
    .bind(identity)
    .bind(notes)
    .bind(dashboard_url)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get_entry(pool, project_id, id)
        .await?
        .ok_or_else(|| "stack entry vanished immediately after insert".to_string())
}

const SELECT_COLS: &str = "id, project_id, service_name, category, identity, notes, \
                           dashboard_url, created_at, updated_at";

/// List a project's stack entries, grouped stably: category (in the
/// [`VALID_CATEGORIES`] display order via the index), then service name.
pub async fn list_entries(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<StackEntry>, String> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM project_stack_entries WHERE project_id = ? \
         ORDER BY category ASC, service_name COLLATE NOCASE ASC, id ASC"
    )))
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(row_to_entry).collect())
}

/// Fetch one entry scoped to its project.
pub async fn get_entry(
    pool: &Pool<Sqlite>,
    project_id: &str,
    entry_id: &str,
) -> Result<Option<StackEntry>, String> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM project_stack_entries WHERE id = ? AND project_id = ?"
    )))
    .bind(entry_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_entry))
}

/// Update an entry (project-scoped, per-field like `update_project`). Returns
/// `Ok(None)` when no such entry exists in this project.
pub async fn update_entry(
    pool: &Pool<Sqlite>,
    project_id: &str,
    entry_id: &str,
    input: UpdateStackEntry,
) -> Result<Option<StackEntry>, String> {
    if get_entry(pool, project_id, entry_id).await?.is_none() {
        return Ok(None);
    }

    if let Some(ref service_name) = input.service_name {
        let service_name = service_name.trim();
        if service_name.is_empty() {
            return Err("service_name is empty".to_string());
        }
        sqlx::query("UPDATE project_stack_entries SET service_name = ? WHERE id = ?")
            .bind(service_name)
            .bind(entry_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref category) = input.category {
        validate_category(category)?;
        sqlx::query("UPDATE project_stack_entries SET category = ? WHERE id = ?")
            .bind(category)
            .bind(entry_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref identity) = input.identity {
        sqlx::query("UPDATE project_stack_entries SET identity = ? WHERE id = ?")
            .bind(identity.as_deref())
            .bind(entry_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref notes) = input.notes {
        sqlx::query("UPDATE project_stack_entries SET notes = ? WHERE id = ?")
            .bind(notes)
            .bind(entry_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref dashboard_url) = input.dashboard_url {
        sqlx::query("UPDATE project_stack_entries SET dashboard_url = ? WHERE id = ?")
            .bind(dashboard_url.as_deref())
            .bind(entry_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    get_entry(pool, project_id, entry_id).await
}

/// Delete an entry (project-scoped). Returns `false` when no such entry.
pub async fn delete_entry(
    pool: &Pool<Sqlite>,
    project_id: &str,
    entry_id: &str,
) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM project_stack_entries WHERE id = ? AND project_id = ?")
        .bind(entry_id)
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

fn row_to_entry(r: &sqlx::sqlite::SqliteRow) -> StackEntry {
    StackEntry {
        id: r.get("id"),
        project_id: r.get("project_id"),
        service_name: r.get("service_name"),
        category: r.get("category"),
        identity: r.get("identity"),
        notes: r.get("notes"),
        dashboard_url: r.get("dashboard_url"),
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

    /// Acceptance: "which account do I use for Railway on Kinrows?" — a service
    /// + identity row attaches to a project and lists back.
    #[tokio::test]
    async fn insert_and_list_back() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Kinrows").await;

        let entry = insert_entry(
            &pool,
            "se-1",
            &proj,
            "Railway",
            "hosting",
            Some("jesse+kinrows@gmail.com"),
            "free tier, 2 services max",
            Some("https://railway.app/dashboard"),
        )
        .await
        .unwrap();
        assert_eq!(entry.service_name, "Railway");
        assert_eq!(entry.category, "hosting");
        assert_eq!(entry.identity.as_deref(), Some("jesse+kinrows@gmail.com"));
        assert_eq!(entry.notes, "free tier, 2 services max");
        assert_eq!(
            entry.dashboard_url.as_deref(),
            Some("https://railway.app/dashboard")
        );
        assert!(!entry.created_at.is_empty());

        let entries = list_entries(&pool, &proj).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], entry);
    }

    /// The schema is reference-only: the table has NO column that could hold a
    /// password/secret. Guards against a column sneaking in via a later
    /// migration without this contract being revisited.
    #[tokio::test]
    async fn table_has_no_secret_columns() {
        let pool = test_pool().await;
        let cols: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('project_stack_entries')")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            cols,
            vec![
                "id",
                "project_id",
                "service_name",
                "category",
                "identity",
                "notes",
                "dashboard_url",
                "created_at",
                "updated_at"
            ]
        );
        for col in &cols {
            let c = col.to_lowercase();
            assert!(
                !c.contains("password") && !c.contains("secret") && !c.contains("token"),
                "reference-only contract violated: column {col}"
            );
        }
    }

    /// Invalid categories are rejected in Rust AND by the SQL CHECK.
    #[tokio::test]
    async fn invalid_category_rejected_both_layers() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;

        // Rust-side validation.
        let err = insert_entry(&pool, "se-1", &proj, "Vercel", "cloud", None, "", None)
            .await
            .unwrap_err();
        assert!(err.contains("Invalid category"), "got: {err}");

        // SQL CHECK backstop (bypassing the Rust validation).
        let db_err = sqlx::query(
            "INSERT INTO project_stack_entries (id, project_id, service_name, category) \
             VALUES ('se-raw', ?, 'Vercel', 'cloud')",
        )
        .bind(&proj)
        .execute(&pool)
        .await;
        assert!(db_err.is_err(), "SQL CHECK should reject unknown category");

        // Every Rust-valid category is SQL-valid (the two lists are in sync).
        for (i, cat) in VALID_CATEGORIES.iter().enumerate() {
            insert_entry(
                &pool,
                &format!("se-cat-{i}"),
                &proj,
                "Svc",
                cat,
                None,
                "",
                None,
            )
            .await
            .unwrap();
        }
    }

    /// Empty/whitespace service names are rejected on insert and update.
    #[tokio::test]
    async fn empty_service_name_rejected() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        assert!(
            insert_entry(&pool, "se-1", &proj, "   ", "other", None, "", None)
                .await
                .is_err()
        );

        insert_entry(&pool, "se-1", &proj, "Neon", "database", None, "", None)
            .await
            .unwrap();
        assert!(update_entry(
            &pool,
            &proj,
            "se-1",
            UpdateStackEntry {
                service_name: Some("  ".to_string()),
                ..Default::default()
            },
        )
        .await
        .is_err());
    }

    /// Update semantics: set fields, clear a nullable field with the double
    /// Option, leave the rest untouched; `updated_at` advances via the trigger.
    #[tokio::test]
    async fn update_roundtrip_and_clear() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let created = insert_entry(
            &pool,
            "se-1",
            &proj,
            "Supabase",
            "database",
            Some("old@x.com"),
            "starter",
            Some("https://supabase.com/dashboard"),
        )
        .await
        .unwrap();

        let updated = update_entry(
            &pool,
            &proj,
            "se-1",
            UpdateStackEntry {
                identity: Some(Some("new@x.com".to_string())),
                notes: Some("upgraded to pro".to_string()),
                dashboard_url: Some(None), // explicit clear
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.service_name, "Supabase"); // untouched
        assert_eq!(updated.category, "database"); // untouched
        assert_eq!(updated.identity.as_deref(), Some("new@x.com"));
        assert_eq!(updated.notes, "upgraded to pro");
        assert_eq!(updated.dashboard_url, None);
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at >= created.updated_at);

        // Updating a nonexistent entry is None, not an error.
        assert!(
            update_entry(&pool, &proj, "nope", UpdateStackEntry::default())
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Reads, updates, and deletes are project-scoped: another project can't
    /// touch the row.
    #[tokio::test]
    async fn crud_is_project_scoped() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let other = a_project(&pool, "Other").await;
        insert_entry(&pool, "se-1", &proj, "Vercel", "hosting", None, "", None)
            .await
            .unwrap();

        assert!(get_entry(&pool, &other, "se-1").await.unwrap().is_none());
        assert!(
            update_entry(&pool, &other, "se-1", UpdateStackEntry::default())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!delete_entry(&pool, &other, "se-1").await.unwrap());

        assert!(delete_entry(&pool, &proj, "se-1").await.unwrap());
        assert!(!delete_entry(&pool, &proj, "se-1").await.unwrap()); // idempotent
        assert!(list_entries(&pool, &proj).await.unwrap().is_empty());
    }

    /// Listing is grouped: category, then service name (case-insensitive).
    #[tokio::test]
    async fn list_orders_by_category_then_service() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        for (id, svc, cat) in [
            ("se-1", "vercel", "hosting"),
            ("se-2", "Neon", "database"),
            ("se-3", "Railway", "hosting"),
            ("se-4", "X", "social"),
        ] {
            insert_entry(&pool, id, &proj, svc, cat, None, "", None)
                .await
                .unwrap();
        }
        let names: Vec<(String, String)> = list_entries(&pool, &proj)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.category, e.service_name))
            .collect();
        assert_eq!(
            names,
            vec![
                ("database".into(), "Neon".into()),
                ("hosting".into(), "Railway".into()),
                ("hosting".into(), "vercel".into()),
                ("social".into(), "X".into()),
            ]
        );
    }

    /// Deleting the project cascades its stack entries away.
    #[tokio::test]
    async fn deleting_project_cascades_entries() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        insert_entry(&pool, "se-1", &proj, "Vercel", "hosting", None, "", None)
            .await
            .unwrap();

        crate::projects::delete_project(&pool, &proj).await.unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_stack_entries WHERE project_id = ?")
                .bind(&proj)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    /// Re-running the apply/migrate steps on an initialized DB must not error.
    #[tokio::test]
    async fn migration_is_idempotent() {
        let pool = test_pool().await;
        crate::session::spectral_schema::apply_project_stack_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v30_to_v31(&pool)
            .await
            .unwrap();
    }
}
