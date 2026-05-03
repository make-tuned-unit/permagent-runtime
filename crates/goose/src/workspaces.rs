//! Workspaces module — CRUD and preset seeding for Command Center workspaces.

use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

// ── Data types ─────────────────────────────────────────────────────────────

pub struct Workspace {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub icon: String,
    pub sort_order: i32,
    pub layout_json: serde_json::Value,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

// ── Preset layouts ─────────────────────────────────────────────────────────

fn automate_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "automate",
        "config": {}
    })
}

fn world_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "world",
        "config": {}
    })
}

fn build_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "build",
        "config": {}
    })
}

fn brain_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "memory",
        "config": {}
    })
}

fn home_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "dashboard",
        "config": {}
    })
}

// ── Operations ─────────────────────────────────────────────────────────────

/// Seed the three preset workspaces if the user has none.
/// Returns true if workspaces were seeded.
pub async fn seed_presets_if_empty(pool: &Pool<Sqlite>) -> Result<bool, String> {
    // Use BEGIN IMMEDIATE to avoid lock-upgrade contention with concurrent readers.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;

    let count: i32 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE user_id = 'default'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

    if count > 0 {
        // Nothing to do — drop the transaction (implicit rollback, no writes).
        return Ok(false);
    }

    let presets = [
        ("Home", "home", 0, home_layout(), true),
        ("Automate", "layout-dashboard", 1, automate_layout(), false),
        ("World", "globe", 2, world_layout(), false),
        ("Build", "code", 3, build_layout(), false),
        ("Brain", "brain", 4, brain_layout(), false),
    ];

    let mut first_id = String::new();
    for (name, icon, sort_order, layout, is_default) in &presets {
        let id = Uuid::now_v7().to_string();
        if first_id.is_empty() {
            first_id = id.clone();
        }
        let layout_str = serde_json::to_string(layout).map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
             VALUES (?, 'default', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(icon)
        .bind(sort_order)
        .bind(&layout_str)
        .bind(is_default)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Set active workspace to Work (first preset)
    sqlx::query("UPDATE users SET active_workspace_id = ? WHERE id = 'default'")
        .bind(&first_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(true)
}

/// List all workspaces for the default user, ordered by sort_order.
pub async fn list_workspaces(pool: &Pool<Sqlite>) -> Result<Vec<Workspace>, String> {
    let rows = sqlx::query(
        "SELECT id, user_id, name, icon, sort_order, layout_json, is_default, created_at, updated_at
         FROM workspaces
         WHERE user_id = 'default'
         ORDER BY sort_order ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let layout_str: String = r.get("layout_json");
            let layout_json =
                serde_json::from_str(&layout_str).unwrap_or(serde_json::Value::Null);
            Workspace {
                id: r.get("id"),
                user_id: r.get("user_id"),
                name: r.get("name"),
                icon: r.get("icon"),
                sort_order: r.get("sort_order"),
                layout_json,
                is_default: r.get("is_default"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            }
        })
        .collect())
}

/// Get a single workspace by ID.
pub async fn get_workspace(pool: &Pool<Sqlite>, workspace_id: &str) -> Result<Option<Workspace>, String> {
    let row = sqlx::query(
        "SELECT id, user_id, name, icon, sort_order, layout_json, is_default, created_at, updated_at
         FROM workspaces
         WHERE id = ? AND user_id = 'default'",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|r| {
        let layout_str: String = r.get("layout_json");
        let layout_json =
            serde_json::from_str(&layout_str).unwrap_or(serde_json::Value::Null);
        Workspace {
            id: r.get("id"),
            user_id: r.get("user_id"),
            name: r.get("name"),
            icon: r.get("icon"),
            sort_order: r.get("sort_order"),
            layout_json,
            is_default: r.get("is_default"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }
    }))
}

/// Update the layout_json for a workspace (used after splitter resize).
pub async fn update_layout(
    pool: &Pool<Sqlite>,
    workspace_id: &str,
    layout_json: &serde_json::Value,
) -> Result<bool, String> {
    let layout_str = serde_json::to_string(layout_json).map_err(|e| e.to_string())?;

    let result = sqlx::query(
        "UPDATE workspaces SET layout_json = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ? AND user_id = 'default'",
    )
    .bind(&layout_str)
    .bind(workspace_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

/// Get the active workspace ID for the default user.
pub async fn get_active_workspace_id(pool: &Pool<Sqlite>) -> Result<Option<String>, String> {
    let id: Option<String> =
        sqlx::query_scalar("SELECT active_workspace_id FROM users WHERE id = 'default'")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;

    Ok(id)
}

/// Set the active workspace ID for the default user.
pub async fn set_active_workspace(pool: &Pool<Sqlite>, workspace_id: &str) -> Result<bool, String> {
    // Verify workspace exists
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = ? AND user_id = 'default')",
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if !exists {
        return Ok(false);
    }

    sqlx::query("UPDATE users SET active_workspace_id = ? WHERE id = 'default'")
        .bind(workspace_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(true)
}
