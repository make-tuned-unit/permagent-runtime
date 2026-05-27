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
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: String,
}

fn row_to_project(r: &sqlx::sqlite::SqliteRow) -> Project {
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

    get_project(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created project".to_string())
}

pub async fn get_project(pool: &Pool<Sqlite>, id: &str) -> Result<Option<Project>, String> {
    let row = sqlx::query(
        "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, created_at, updated_at, last_opened_at
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
        "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, created_at, updated_at, last_opened_at
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
            "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, created_at, updated_at, last_opened_at
             FROM projects WHERE user_id = 'default' AND status = ?
             ORDER BY last_opened_at DESC",
        )
        .bind(status)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, user_id, slug, name, description, status, root_path, site_url, repo_url, notes, created_at, updated_at, last_opened_at
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
