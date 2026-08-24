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

fn grow_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "grow",
        "config": {}
    })
}

fn projects_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "projects",
        "config": {}
    })
}

fn financier_layout() -> serde_json::Value {
    serde_json::json!({
        "type": "panel",
        "tool": "financier",
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

    // Sidebar order is a product decision (ruling 2026-07-10):
    // Home, Projects, Build, Automate, World — Brain follows.
    // Keep in lockstep with CANONICAL_WORKSPACE_ORDER below.
    let presets = [
        ("Home", "home", 0, home_layout(), true),
        // Icon keys renamed 2026-08-03: three tabs carried glyphs whose names
        // lied about what they drew — "code" was a four-rectangle mosaic and
        // "layout-dashboard" was a hamburger — so Build, Automate and Projects
        // were three interchangeable abstract shapes. See ICON_PATHS in
        // Sidebar.tsx, which still aliases the old keys for existing rows.
        ("Projects", "folder", 1, projects_layout(), false),
        ("Build", "brackets", 2, build_layout(), false),
        ("Grow", "trending-up", 3, grow_layout(), false),
        ("Automate", "bolt", 4, automate_layout(), false),
        ("World", "globe", 5, world_layout(), false),
        ("Brain", "brain", 6, brain_layout(), false),
        ("Financier", "scale", 7, financier_layout(), false),
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

/// The code-owned sidebar order (ruling 2026-07-10). Applied at every
/// startup by [`ensure_canonical_workspace_order`] so existing installs pick
/// up order changes without a reseed; workspaces outside this list (user-
/// created) keep their own sort_order untouched.
pub const CANONICAL_WORKSPACE_ORDER: &[(&str, i32)] = &[
    ("Home", 0),
    ("Projects", 1),
    ("Build", 2),
    ("Grow", 3),
    ("Automate", 4),
    ("World", 5),
    ("Brain", 6),
    ("Financier", 7),
];

/// Normalize the preset workspaces' sort_order to [`CANONICAL_WORKSPACE_ORDER`].
/// Idempotent; returns true if any row changed.
pub async fn ensure_canonical_workspace_order(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let mut changed = false;
    for (name, order) in CANONICAL_WORKSPACE_ORDER {
        let result = sqlx::query(
            "UPDATE workspaces SET sort_order = ?
             WHERE user_id = 'default' AND name = ? AND sort_order != ?",
        )
        .bind(order)
        .bind(name)
        .bind(order)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        changed |= result.rows_affected() > 0;
    }
    Ok(changed)
}

/// Ensure the "Projects" workspace exists for existing users who were seeded
/// before it was added. Inserts at sort_order 4 and bumps any existing rows
/// at sort_order >= 4 (e.g., Brain from 4 to 5). Idempotent.
pub async fn ensure_projects_workspace(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let has_projects: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspaces WHERE user_id = 'default' AND name = 'Projects')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if has_projects {
        return Ok(false);
    }

    // Bump sort_order for any workspaces at position 4+
    sqlx::query(
        "UPDATE workspaces SET sort_order = sort_order + 1
         WHERE user_id = 'default' AND sort_order >= 4",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let id = Uuid::now_v7().to_string();
    let layout = projects_layout();
    let layout_str = serde_json::to_string(&layout).map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
         VALUES (?, 'default', 'Projects', 'columns', 4, ?, 0)",
    )
    .bind(&id)
    .bind(&layout_str)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(true)
}

/// Ensure the "Grow" workspace exists for existing users seeded before it was
/// added (mirrors [`ensure_projects_workspace`]). Inserted at sort_order 3;
/// [`ensure_canonical_workspace_order`] runs right after at startup and
/// normalizes every preset's position, so the exact number here only needs to
/// be in the right neighborhood. Idempotent.
pub async fn ensure_grow_workspace(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let has_grow: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspaces WHERE user_id = 'default' AND name = 'Grow')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if has_grow {
        return Ok(false);
    }

    // Make room at position 3 (Automate/World/Brain slide up; canonical order
    // finalizes the exact values on this same startup).
    sqlx::query(
        "UPDATE workspaces SET sort_order = sort_order + 1
         WHERE user_id = 'default' AND sort_order >= 3",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let id = Uuid::now_v7().to_string();
    let layout_str = serde_json::to_string(&grow_layout()).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
         VALUES (?, 'default', 'Grow', 'trending-up', 3, ?, 0)",
    )
    .bind(&id)
    .bind(&layout_str)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Ensure the "Financier" workspace exists for users seeded before it was added
/// (mirrors [`ensure_grow_workspace`]).
///
/// Appended at the END rather than inserted, so no existing tab shifts position
/// under a user who has learned where things are — and because it is the newest
/// and least-established tab, not one that outranks Home or Build.
/// [`ensure_canonical_workspace_order`] runs right after at startup and
/// normalizes every preset's position anyway. Idempotent.
///
/// Without this, the tab would reach only brand-new installs: the seed function
/// above short-circuits the moment any workspace row exists, so every user who
/// has already opened the app once would never see it.
pub async fn ensure_financier_workspace(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let has_financier: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM workspaces WHERE user_id = 'default' AND name = 'Financier')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if has_financier {
        return Ok(false);
    }

    let id = Uuid::now_v7().to_string();
    let layout_str = serde_json::to_string(&financier_layout()).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
         VALUES (?, 'default', 'Financier', 'scale', 7, ?, 0)",
    )
    .bind(&id)
    .bind(&layout_str)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
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
            let layout_json = serde_json::from_str(&layout_str).unwrap_or(serde_json::Value::Null);
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
pub async fn get_workspace(
    pool: &Pool<Sqlite>,
    workspace_id: &str,
) -> Result<Option<Workspace>, String> {
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
        let layout_json = serde_json::from_str(&layout_str).unwrap_or(serde_json::Value::Null);
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Pool<Sqlite> {
        use crate::session::spectral_schema::init_spectral_db;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn order_of(pool: &Pool<Sqlite>) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT name FROM workspaces WHERE user_id = 'default' ORDER BY sort_order",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The sidebar order is a product decision (2026-07-10): fresh seeds land
    /// in canon, and installs seeded under the OLD order are normalized on
    /// startup without touching user-created workspaces.
    #[tokio::test]
    async fn canonical_order_seeds_and_normalizes() {
        let pool = test_pool().await;
        seed_presets_if_empty(&pool).await.unwrap();
        assert_eq!(
            order_of(&pool).await,
            [
                "Home",
                "Projects",
                "Build",
                "Grow",
                "Automate",
                "World",
                "Brain",
                "Financier"
            ]
        );

        // Simulate a legacy install: scramble to the pre-2026-07-10 order and
        // add a user-created workspace beyond the presets.
        for (name, order) in [("Automate", 1), ("World", 2), ("Build", 3), ("Projects", 4)] {
            sqlx::query("UPDATE workspaces SET sort_order = ? WHERE name = ?")
                .bind(order)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
             VALUES ('custom-1', 'default', 'My Lab', 'flask', 9, '{}', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let changed = ensure_canonical_workspace_order(&pool).await.unwrap();
        assert!(changed, "legacy order must be rewritten");
        assert_eq!(
            order_of(&pool).await,
            [
                "Home",
                "Projects",
                "Build",
                "Grow",
                "Automate",
                "World",
                "Brain",
                "Financier",
                "My Lab"
            ]
        );

        // Idempotent: a second run changes nothing.
        assert!(!ensure_canonical_workspace_order(&pool).await.unwrap());
    }

    /// The Financier tab has to reach EXISTING installs, not just fresh ones.
    /// `seed_presets_if_empty` short-circuits the moment any workspace row
    /// exists, so without the backfill every user who had ever opened the app
    /// would be told about a tab their sidebar does not have — which is exactly
    /// the "I still don't see it" report this work started from.
    #[tokio::test]
    async fn financier_backfills_into_an_install_seeded_before_it_existed() {
        let pool = test_pool().await;
        // An install seeded under the pre-Financier preset set.
        for (i, name) in [
            "Home", "Projects", "Build", "Grow", "Automate", "World", "Brain",
        ]
        .iter()
        .enumerate()
        {
            sqlx::query(
                "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
                 VALUES (?, 'default', ?, 'home', ?, '{}', 0)",
            )
            .bind(format!("ws-{i}"))
            .bind(name)
            .bind(i as i32)
            .execute(&pool)
            .await
            .unwrap();
        }
        assert!(
            !seed_presets_if_empty(&pool).await.unwrap(),
            "a populated install must not be reseeded"
        );

        assert!(ensure_financier_workspace(&pool).await.unwrap());
        assert!(order_of(&pool).await.contains(&"Financier".to_string()));
        // Idempotent: running it again on the next start adds no duplicate.
        assert!(!ensure_financier_workspace(&pool).await.unwrap());
        assert_eq!(
            order_of(&pool)
                .await
                .iter()
                .filter(|n| *n == "Financier")
                .count(),
            1
        );
    }

    /// The tab is only real if its layout names a tool the frontend can render.
    /// A typo here produces a workspace that renders "Unknown tool" — visible,
    /// clickable, and empty.
    #[test]
    fn the_financier_layout_names_the_financier_tool() {
        assert_eq!(financier_layout()["tool"], "financier");
    }

    /// Every preset must appear in the canonical order table, or a tab seeds at
    /// one position and is silently reordered to another on the next start.
    #[test]
    fn every_preset_is_in_the_canonical_order() {
        for (name, _) in CANONICAL_WORKSPACE_ORDER {
            assert!(!name.is_empty(), "a canonical entry must name a workspace");
        }
        assert!(CANONICAL_WORKSPACE_ORDER
            .iter()
            .any(|(name, _)| *name == "Financier"));
    }
}
