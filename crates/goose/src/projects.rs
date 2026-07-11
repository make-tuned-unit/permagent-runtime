//! Projects module — CRUD operations for the projects table.

use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

pub const PERSONAL_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub user_id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub root_path: Option<String>,
    pub site_url: Option<String>,
    pub repo_url: Option<String>,
    pub notes: String,
    /// General project metadata bag (schema v25; mirrors `cards.metadata_json`
    /// — ruling 3 in GOAL_COMPLETION_AND_VERIFICATION.md §3d). Known keys:
    /// `build_command` (string) — project build check the orchestrator seeds
    /// onto code-flavored goals as a `command_exit_zero` completion check;
    /// `build_timeout_secs` (number) — optional timeout for it.
    pub metadata_json: serde_json::Value,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: String,
}

fn row_to_project(r: &sqlx::sqlite::SqliteRow) -> Project {
    let metadata_raw: String = r.get("metadata_json");
    Project {
        id: r.get("id"),
        user_id: r.get("user_id"),
        slug: r.get("slug"),
        name: r.get("name"),
        description: r.get("description"),
        status: r.get("status"),
        root_path: r.get("root_path"),
        site_url: r.get("site_url"),
        repo_url: r.get("repo_url"),
        notes: r.get("notes"),
        metadata_json: serde_json::from_str(&metadata_raw)
            .unwrap_or_else(|_| serde_json::json!({})),
        tags: Vec::new(),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        last_opened_at: r.get("last_opened_at"),
    }
}

async fn load_tags(pool: &Pool<Sqlite>, project_id: &str) -> Result<Vec<String>, String> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT tag FROM project_tags WHERE project_id = ? ORDER BY tag",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Generate a slug from a project name.
pub fn slugify(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let mut result = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    result.trim_matches('-').to_string()
}

/// Generate a unique slug for the given user, appending -2, -3, etc. on collision.
async fn unique_slug(pool: &Pool<Sqlite>, user_id: &str, base: &str) -> Result<String, String> {
    let check = |slug: String| async move {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE user_id = ? AND slug = ?)",
        )
        .bind(user_id)
        .bind(&slug)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
    };

    if !check(base.to_string()).await? {
        return Ok(base.to_string());
    }

    for n in 2..100 {
        let candidate = format!("{}-{}", base, n);
        if !check(candidate.clone()).await? {
            return Ok(candidate);
        }
    }

    Err("Could not generate unique slug after 99 attempts".to_string())
}

#[derive(Debug, Default)]
pub struct CreateProject {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub root_path: Option<String>,
    pub site_url: Option<String>,
    pub repo_url: Option<String>,
    pub notes: Option<String>,
    pub tags: Option<Vec<String>>,
}

pub async fn create_project(pool: &Pool<Sqlite>, input: CreateProject) -> Result<Project, String> {
    let id = Uuid::now_v7().to_string();
    let user_id = "default";
    let base_slug = input
        .slug
        .as_deref()
        .map(slugify)
        .unwrap_or_else(|| slugify(&input.name));

    if base_slug.is_empty() {
        return Err("Name must contain at least one alphanumeric character".to_string());
    }

    let slug = unique_slug(pool, user_id, &base_slug).await?;

    sqlx::query(
        "INSERT INTO projects (id, user_id, slug, name, description, root_path, site_url, repo_url, notes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(&slug)
    .bind(&input.name)
    .bind(input.description.as_deref().unwrap_or(""))
    .bind(input.root_path.as_deref())
    .bind(input.site_url.as_deref())
    .bind(input.repo_url.as_deref())
    .bind(input.notes.as_deref().unwrap_or(""))
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(tags) = &input.tags {
        for tag in tags {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            sqlx::query("INSERT OR IGNORE INTO project_tags (project_id, tag) VALUES (?, ?)")
                .bind(&id)
                .bind(tag)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // Seed default board columns for the new project
    crate::cards::seed_default_columns(pool, &id).await?;

    get_project(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created project".to_string())
}

pub async fn get_project(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Project>, String> {
    let row = sqlx::query(
        "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, metadata_json, created_at, updated_at, last_opened_at
         FROM projects WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let mut p = row_to_project(&r);
            p.tags = load_tags(pool, &p.id).await?;
            Ok(Some(p))
        }
        None => Ok(None),
    }
}

/// Resolve a project by ID (UUID) or slug (for the default user).
pub async fn get_project_by_id_or_slug(
    pool: &Pool<Sqlite>,
    id_or_slug: &str,
) -> Result<Option<Project>, String> {
    if let Some(p) = get_project(pool, id_or_slug).await? {
        return Ok(Some(p));
    }
    let row = sqlx::query(
        "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, metadata_json, created_at, updated_at, last_opened_at
         FROM projects WHERE user_id = 'default' AND slug = ?",
    )
    .bind(id_or_slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let mut p = row_to_project(&r);
            p.tags = load_tags(pool, &p.id).await?;
            Ok(Some(p))
        }
        None => Ok(None),
    }
}

pub async fn list_projects(
    pool: &Pool<Sqlite>,
    status_filter: Option<&str>,
) -> Result<Vec<Project>, String> {
    let rows = if let Some(status) = status_filter {
        sqlx::query(
            "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, metadata_json, created_at, updated_at, last_opened_at
             FROM projects WHERE user_id = 'default' AND status = ?
             ORDER BY last_opened_at DESC",
        )
        .bind(status)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, metadata_json, created_at, updated_at, last_opened_at
             FROM projects WHERE user_id = 'default'
             ORDER BY last_opened_at DESC",
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    let mut projects: Vec<Project> = rows.iter().map(row_to_project).collect();
    for p in &mut projects {
        p.tags = load_tags(pool, &p.id).await?;
    }
    Ok(projects)
}

#[derive(Debug, Default)]
pub struct UpdateProject {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub root_path: Option<Option<String>>,
    pub site_url: Option<Option<String>>,
    pub repo_url: Option<Option<String>>,
    pub notes: Option<String>,
    /// Full replacement of the project metadata bag (must be a JSON object).
    pub metadata_json: Option<serde_json::Value>,
}

pub async fn update_project(
    pool: &Pool<Sqlite>,
    id: &str,
    input: UpdateProject,
) -> Result<Option<Project>, String> {
    let existing = get_project(pool, id).await?;
    let existing = match existing {
        Some(p) => p,
        None => return Ok(None),
    };

    let is_personal = id == PERSONAL_PROJECT_ID;
    if is_personal {
        if input.slug.is_some() {
            return Err("Cannot change slug of the Personal project".to_string());
        }
        if input.status.is_some() {
            return Err("Cannot change status of the Personal project".to_string());
        }
    }

    if let Some(ref status) = input.status {
        if !["active", "paused", "archived"].contains(&status.as_str()) {
            return Err(format!(
                "Invalid status: {}. Must be active, paused, or archived",
                status
            ));
        }
    }

    // Apply each field update individually (sqlx doesn't support dynamic bind lists)
    if let Some(ref name) = input.name {
        sqlx::query("UPDATE projects SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref slug) = input.slug {
        let new_slug = slugify(slug);
        let collision = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE user_id = ? AND slug = ? AND id != ?)",
        )
        .bind(&existing.user_id)
        .bind(&new_slug)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
        if collision {
            return Err(format!("Slug '{}' already exists", new_slug));
        }
        sqlx::query("UPDATE projects SET slug = ? WHERE id = ?")
            .bind(&new_slug)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref desc) = input.description {
        sqlx::query("UPDATE projects SET description = ? WHERE id = ?")
            .bind(desc)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref status) = input.status {
        sqlx::query("UPDATE projects SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref notes) = input.notes {
        sqlx::query("UPDATE projects SET notes = ? WHERE id = ?")
            .bind(notes)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref root_path) = input.root_path {
        sqlx::query("UPDATE projects SET root_path = ? WHERE id = ?")
            .bind(root_path.as_deref())
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref site_url) = input.site_url {
        sqlx::query("UPDATE projects SET site_url = ? WHERE id = ?")
            .bind(site_url.as_deref())
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref repo_url) = input.repo_url {
        sqlx::query("UPDATE projects SET repo_url = ? WHERE id = ?")
            .bind(repo_url.as_deref())
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref metadata) = input.metadata_json {
        if !metadata.is_object() {
            return Err("metadata_json must be a JSON object".to_string());
        }
        let raw = serde_json::to_string(metadata).map_err(|e| e.to_string())?;
        sqlx::query("UPDATE projects SET metadata_json = ? WHERE id = ?")
            .bind(&raw)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    get_project(pool, id).await
}

pub async fn delete_project(pool: &Pool<Sqlite>, id: &str) -> Result<bool, String> {
    if id == PERSONAL_PROJECT_ID {
        return Err("Cannot delete the Personal project".to_string());
    }

    let result = sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

pub async fn touch_project(pool: &Pool<Sqlite>, id: &str) -> Result<bool, String> {
    let result = sqlx::query(
        "UPDATE projects SET last_opened_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

pub async fn add_tag(pool: &Pool<Sqlite>, project_id: &str, tag: &str) -> Result<bool, String> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err("Tag cannot be empty".to_string());
    }

    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
            .bind(project_id)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    if !exists {
        return Ok(false);
    }

    sqlx::query("INSERT OR IGNORE INTO project_tags (project_id, tag) VALUES (?, ?)")
        .bind(project_id)
        .bind(tag)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(true)
}

pub async fn remove_tag(pool: &Pool<Sqlite>, project_id: &str, tag: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM project_tags WHERE project_id = ? AND tag = ?")
        .bind(project_id)
        .bind(tag)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_tags(pool: &Pool<Sqlite>, project_id: &str) -> Result<Vec<String>, String> {
    load_tags(pool, project_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Permagent Runtime"), "permagent-runtime");
        assert_eq!(slugify("  My Cool App  "), "my-cool-app");
        assert_eq!(slugify("hello---world"), "hello-world");
        assert_eq!(slugify("UPPER CASE"), "upper-case");
        assert_eq!(slugify("a.b.c"), "a-b-c");
        assert_eq!(slugify("---"), "");
    }

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn personal_project_seeded() {
        let pool = test_pool().await;
        let personal = get_project(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert!(personal.is_some());
        let p = personal.unwrap();
        assert_eq!(p.slug, "personal");
        assert_eq!(p.name, "Personal");
        assert_eq!(p.status, "active");
    }

    #[tokio::test]
    async fn create_and_get() {
        let pool = test_pool().await;
        let input = CreateProject {
            name: "Test Project".to_string(),
            ..Default::default()
        };
        let created = create_project(&pool, input).await.unwrap();
        assert_eq!(created.slug, "test-project");
        assert_eq!(created.status, "active");

        let fetched = get_project(&pool, &created.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "Test Project");
    }

    #[tokio::test]
    async fn slug_collision_appends_suffix() {
        let pool = test_pool().await;
        let input1 = CreateProject {
            name: "Dup".to_string(),
            ..Default::default()
        };
        let p1 = create_project(&pool, input1).await.unwrap();
        assert_eq!(p1.slug, "dup");

        let input2 = CreateProject {
            name: "Dup".to_string(),
            ..Default::default()
        };
        let p2 = create_project(&pool, input2).await.unwrap();
        assert_eq!(p2.slug, "dup-2");
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let pool = test_pool().await;
        let _ = create_project(
            &pool,
            CreateProject {
                name: "Active".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let p2 = create_project(
            &pool,
            CreateProject {
                name: "Paused".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        update_project(
            &pool,
            &p2.id,
            UpdateProject {
                status: Some("paused".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let active = list_projects(&pool, Some("active")).await.unwrap();
        // Personal + Active
        assert!(active.iter().any(|p| p.name == "Active"));
        assert!(!active.iter().any(|p| p.name == "Paused"));

        let paused = list_projects(&pool, Some("paused")).await.unwrap();
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].name, "Paused");
    }

    #[tokio::test]
    async fn update_fields() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "Orig".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let updated = update_project(
            &pool,
            &p.id,
            UpdateProject {
                name: Some("New Name".to_string()),
                root_path: Some(Some("/dev/myproject".to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.root_path.as_deref(), Some("/dev/myproject"));
    }

    #[tokio::test]
    async fn metadata_json_defaults_empty_and_roundtrips() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "Meta".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(p.metadata_json, serde_json::json!({}));

        let updated = update_project(
            &pool,
            &p.id,
            UpdateProject {
                metadata_json: Some(serde_json::json!({
                    "build_command": "npm run build",
                    "build_timeout_secs": 300
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            updated.metadata_json.get("build_command").unwrap(),
            "npm run build"
        );

        // Non-object metadata is refused.
        let err = update_project(
            &pool,
            &p.id,
            UpdateProject {
                metadata_json: Some(serde_json::json!(["not", "an", "object"])),
                ..Default::default()
            },
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn personal_project_protections() {
        let pool = test_pool().await;
        // Cannot delete
        let err = delete_project(&pool, PERSONAL_PROJECT_ID).await;
        assert!(err.is_err());

        // Cannot change slug
        let err = update_project(
            &pool,
            PERSONAL_PROJECT_ID,
            UpdateProject {
                slug: Some("new-slug".to_string()),
                ..Default::default()
            },
        )
        .await;
        assert!(err.is_err());

        // Cannot change status
        let err = update_project(
            &pool,
            PERSONAL_PROJECT_ID,
            UpdateProject {
                status: Some("archived".to_string()),
                ..Default::default()
            },
        )
        .await;
        assert!(err.is_err());

        // CAN change description
        let updated = update_project(
            &pool,
            PERSONAL_PROJECT_ID,
            UpdateProject {
                description: Some("My personal space".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.description, "My personal space");
    }

    #[tokio::test]
    async fn delete_cascades_tags() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "Tagged".to_string(),
                tags: Some(vec!["rust".to_string(), "saas".to_string()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(p.tags, vec!["rust", "saas"]);

        let deleted = delete_project(&pool, &p.id).await.unwrap();
        assert!(deleted);

        let tags = list_tags(&pool, &p.id).await.unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn touch_updates_last_opened() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "Touchable".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let before = p.last_opened_at.clone();

        // Small delay to ensure timestamp differs
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        touch_project(&pool, &p.id).await.unwrap();

        let after = get_project(&pool, &p.id).await.unwrap().unwrap();
        assert!(after.last_opened_at >= before);
    }

    #[tokio::test]
    async fn tag_add_remove() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "Taggable".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        add_tag(&pool, &p.id, "rust").await.unwrap();
        add_tag(&pool, &p.id, "saas").await.unwrap();
        // Duplicate add is idempotent
        add_tag(&pool, &p.id, "rust").await.unwrap();

        let tags = list_tags(&pool, &p.id).await.unwrap();
        assert_eq!(tags, vec!["rust", "saas"]);

        remove_tag(&pool, &p.id, "rust").await.unwrap();
        let tags = list_tags(&pool, &p.id).await.unwrap();
        assert_eq!(tags, vec!["saas"]);
    }

    #[tokio::test]
    async fn resolve_by_id_or_slug() {
        let pool = test_pool().await;
        let p = create_project(
            &pool,
            CreateProject {
                name: "My App".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // By ID
        let by_id = get_project_by_id_or_slug(&pool, &p.id).await.unwrap();
        assert!(by_id.is_some());

        // By slug
        let by_slug = get_project_by_id_or_slug(&pool, "my-app").await.unwrap();
        assert!(by_slug.is_some());
        assert_eq!(by_slug.unwrap().id, p.id);

        // Not found
        let missing = get_project_by_id_or_slug(&pool, "nonexistent")
            .await
            .unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn migration_idempotent() {
        let pool = test_pool().await;
        // Run migration again on already-initialized DB — should not error
        crate::session::spectral_schema::migrate_v6_to_v7(&pool)
            .await
            .unwrap();
        // Personal project still exists, no duplicate
        let projects = list_projects(&pool, None).await.unwrap();
        let personal_count = projects
            .iter()
            .filter(|p| p.id == PERSONAL_PROJECT_ID)
            .count();
        assert_eq!(personal_count, 1);
    }
}
