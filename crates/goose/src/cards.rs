//! Cards module — CRUD operations for the cards and board_columns tables.
//!
//! Goal-card hardening (Decision Inbox, S1): goal lifecycle state lives in
//! `column_id` + protected metadata keys, and every daemon path (HTTP, MCP
//! tools, background tasks) converges on the functions in this module. They
//! therefore REFUSE goal column changes and protected-metadata writes; the
//! sole legal mutator is [`crate::goal_transition::advance_goal_checked`]
//! (and its audited sibling paths), which performs its own guarded writes.

use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

use crate::goal_transition::PROTECTED_GOAL_METADATA_KEYS;
use crate::projects::PERSONAL_PROJECT_ID;

/// Check a metadata replacement against the protected-key set for goal cards.
/// Any change (add / remove / modify) to a protected key is refused. The
/// `verification` key is deliberately NOT protected (Lane L2 allowlist).
fn check_protected_metadata(
    existing: &serde_json::Value,
    proposed: &serde_json::Value,
) -> Result<(), String> {
    let null = serde_json::Value::Null;
    for key in PROTECTED_GOAL_METADATA_KEYS {
        let before = existing.get(*key).unwrap_or(&null);
        let after = proposed.get(*key).unwrap_or(&null);
        if before != after {
            return Err(format!(
                "Refusing to write protected goal metadata key '{}'. Goal state, attempts, \
                 budgets, and attention flags are managed by the goal-transition guard \
                 (decision inbox); they cannot be edited directly.",
                key
            ));
        }
    }
    Ok(())
}

const GOAL_MOVE_REFUSAL: &str =
    "Goal cards cannot be moved between columns directly. Goal lifecycle transitions go \
     through the decision inbox (goal_transition guard): use goal_advance for tier-0 steps \
     and answer the corresponding decision for approve/reject.";

// ── Data types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BoardColumn {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub position: i32,
    pub column_kind: String,
    pub state_binding: Option<String>,
    pub wip_limit: Option<i32>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct Card {
    pub id: String,
    pub project_id: String,
    pub card_type: String,
    pub title: String,
    pub description: String,
    pub column_id: String,
    pub position: i32,
    pub created_by: String,
    pub assigned_to: Option<String>,
    pub metadata_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

fn row_to_column(r: &sqlx::sqlite::SqliteRow) -> BoardColumn {
    BoardColumn {
        id: r.get("id"),
        project_id: r.get("project_id"),
        name: r.get("name"),
        position: r.get("position"),
        column_kind: r.get("column_kind"),
        state_binding: r.get("state_binding"),
        wip_limit: r.get("wip_limit"),
        created_at: r.get("created_at"),
    }
}

fn row_to_card(r: &sqlx::sqlite::SqliteRow) -> Card {
    let meta_str: String = r.get("metadata_json");
    let metadata_json =
        serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Object(Default::default()));
    Card {
        id: r.get("id"),
        project_id: r.get("project_id"),
        card_type: r.get("card_type"),
        title: r.get("title"),
        description: r.get("description"),
        column_id: r.get("column_id"),
        position: r.get("position"),
        created_by: r.get("created_by"),
        assigned_to: r.get("assigned_to"),
        metadata_json,
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        archived_at: r.get("archived_at"),
    }
}

// ── Default columns ────────────────────────────────────────────────────────

/// The three default columns seeded for every new project.
pub const DEFAULT_COLUMNS: &[(&str, &str)] =
    &[("backlog", "Backlog"), ("doing", "Doing"), ("done", "Done")];

/// Seed default columns (Backlog/Doing/Done) for a project.
/// Uses deterministic IDs for the Personal project, generated IDs for others.
/// Idempotent — skips if columns already exist for this project.
pub async fn seed_default_columns(pool: &Pool<Sqlite>, project_id: &str) -> Result<(), String> {
    let count: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM board_columns WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    if count > 0 {
        return Ok(());
    }

    for (i, (suffix, name)) in DEFAULT_COLUMNS.iter().enumerate() {
        let id = if project_id == PERSONAL_PROJECT_ID {
            format!("col-personal-{}", suffix)
        } else {
            Uuid::now_v7().to_string()
        };

        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind)
             VALUES (?, ?, ?, ?, 'manual')",
        )
        .bind(&id)
        .bind(project_id)
        .bind(name)
        .bind(i as i32)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// The five lifecycle columns seeded for projects using goal cards.
pub const GOAL_COLUMNS: &[(&str, &str, i32)] = &[
    ("triage", "Triage", 100),
    ("ready", "Ready", 101),
    ("in_progress", "In Progress", 102),
    ("review", "Review", 103),
    ("complete", "Complete", 104),
];

/// Seed goal lifecycle columns (Triage/Ready/InProgress/Review/Complete) for a project.
/// Idempotent — skips if state-bound columns already exist for this project.
/// Called on the first card_create with card_type='goal' for a project.
pub async fn seed_goal_columns(pool: &Pool<Sqlite>, project_id: &str) -> Result<(), String> {
    let has_state_cols: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM board_columns WHERE project_id = ? AND column_kind = 'state')",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if has_state_cols {
        return Ok(());
    }

    for (state_binding, name, position) in GOAL_COLUMNS {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
             VALUES (?, ?, ?, ?, 'state', ?)",
        )
        .bind(&id)
        .bind(project_id)
        .bind(name)
        .bind(position)
        .bind(state_binding)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Find the goal lifecycle column for a given state_binding in a project.
pub async fn get_goal_column(
    pool: &Pool<Sqlite>,
    project_id: &str,
    state_binding: &str,
) -> Result<Option<BoardColumn>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, name, position, column_kind, state_binding, wip_limit, created_at
         FROM board_columns WHERE project_id = ? AND state_binding = ?",
    )
    .bind(project_id)
    .bind(state_binding)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_column))
}

// ── Column operations ──────────────────────────────────────────────────────

pub async fn list_columns(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<BoardColumn>, String> {
    let rows = sqlx::query(
        "SELECT id, project_id, name, position, column_kind, state_binding, wip_limit, created_at
         FROM board_columns WHERE project_id = ? ORDER BY position ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(row_to_column).collect())
}

pub async fn get_column(
    pool: &Pool<Sqlite>,
    column_id: &str,
) -> Result<Option<BoardColumn>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, name, position, column_kind, state_binding, wip_limit, created_at
         FROM board_columns WHERE id = ?",
    )
    .bind(column_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_column))
}

/// Find a column by name within a project.
pub async fn get_column_by_name(
    pool: &Pool<Sqlite>,
    project_id: &str,
    name: &str,
) -> Result<Option<BoardColumn>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, name, position, column_kind, state_binding, wip_limit, created_at
         FROM board_columns WHERE project_id = ? AND name = ? COLLATE NOCASE",
    )
    .bind(project_id)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_column))
}

#[derive(Debug, Default)]
pub struct CreateColumn {
    pub project_id: String,
    pub name: String,
    pub position: Option<i32>,
}

pub async fn create_column(
    pool: &Pool<Sqlite>,
    input: CreateColumn,
) -> Result<BoardColumn, String> {
    let position = match input.position {
        Some(p) => p,
        None => {
            let max: Option<i32> =
                sqlx::query_scalar("SELECT MAX(position) FROM board_columns WHERE project_id = ?")
                    .bind(&input.project_id)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            max.unwrap_or(-1) + 1
        }
    };

    let id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO board_columns (id, project_id, name, position, column_kind)
         VALUES (?, ?, ?, ?, 'manual')",
    )
    .bind(&id)
    .bind(&input.project_id)
    .bind(&input.name)
    .bind(position)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get_column(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created column".to_string())
}

#[derive(Debug, Default)]
pub struct UpdateColumn {
    pub name: Option<String>,
    pub wip_limit: Option<Option<i32>>,
}

pub async fn update_column(
    pool: &Pool<Sqlite>,
    column_id: &str,
    input: UpdateColumn,
) -> Result<Option<BoardColumn>, String> {
    let existing = get_column(pool, column_id).await?;
    if existing.is_none() {
        return Ok(None);
    }

    if let Some(ref name) = input.name {
        sqlx::query("UPDATE board_columns SET name = ? WHERE id = ?")
            .bind(name)
            .bind(column_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    if let Some(ref wip) = input.wip_limit {
        sqlx::query("UPDATE board_columns SET wip_limit = ? WHERE id = ?")
            .bind(wip.as_ref())
            .bind(column_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    get_column(pool, column_id).await
}

pub async fn delete_column(pool: &Pool<Sqlite>, column_id: &str) -> Result<bool, String> {
    // Refuse if cards are present
    let card_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cards WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(column_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if card_count > 0 {
        return Err(format!(
            "Cannot delete column: {} active card(s) present. Move or archive them first.",
            card_count
        ));
    }

    let result = sqlx::query("DELETE FROM board_columns WHERE id = ?")
        .bind(column_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

// ── Card operations ────────────────────────────────────────────────────────

pub async fn list_cards(
    pool: &Pool<Sqlite>,
    project_id: &str,
    card_type: Option<&str>,
    column_id: Option<&str>,
) -> Result<Vec<Card>, String> {
    let mut sql = String::from(
        "SELECT id, project_id, card_type, title, description, column_id, position,
                created_by, assigned_to, metadata_json, created_at, updated_at, archived_at
         FROM cards WHERE project_id = ? AND archived_at IS NULL",
    );
    let mut binds: Vec<String> = vec![project_id.to_string()];

    if let Some(ct) = card_type {
        sql.push_str(" AND card_type = ?");
        binds.push(ct.to_string());
    }
    if let Some(cid) = column_id {
        sql.push_str(" AND column_id = ?");
        binds.push(cid.to_string());
    }

    sql.push_str(" ORDER BY column_id, position ASC");

    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = query.fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_card).collect())
}

pub async fn get_card(pool: &Pool<Sqlite>, card_id: &str) -> Result<Option<Card>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, card_type, title, description, column_id, position,
                created_by, assigned_to, metadata_json, created_at, updated_at, archived_at
         FROM cards WHERE id = ?",
    )
    .bind(card_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.as_ref().map(row_to_card))
}

#[derive(Debug)]
pub struct CreateCard {
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub card_type: Option<String>,
    pub column_id: Option<String>,
    pub created_by: Option<String>,
    pub metadata_json: Option<serde_json::Value>,
}

pub async fn create_card(pool: &Pool<Sqlite>, input: CreateCard) -> Result<Card, String> {
    let card_type = input.card_type.as_deref().unwrap_or("standard");
    if !["standard", "goal", "social_post"].contains(&card_type) {
        return Err(format!(
            "Invalid card_type: {}. Must be standard, goal, or social_post",
            card_type
        ));
    }

    let created_by = input.created_by.as_deref().unwrap_or("user");
    if ![
        "user",
        "henry",
        "hermes",
        "codex",
        "claude-code",
        "librarian",
    ]
    .contains(&created_by)
    {
        return Err(format!("Invalid created_by: {}", created_by));
    }

    // For goal cards: seed lifecycle columns if absent, default to Triage
    if card_type == "goal" {
        seed_goal_columns(pool, &input.project_id).await?;
    }

    // Resolve column: use provided, or fall back based on card type
    let column_id = match input.column_id {
        Some(cid) => cid,
        None if card_type == "goal" => {
            // Goal cards default to the Triage column
            get_goal_column(pool, &input.project_id, "triage")
                .await?
                .map(|c| c.id)
                .ok_or("Triage column not found after seeding goal columns")?
        }
        None => {
            let first: Option<String> = sqlx::query_scalar(
                "SELECT id FROM board_columns WHERE project_id = ? ORDER BY position ASC LIMIT 1",
            )
            .bind(&input.project_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
            first.ok_or("No columns exist for this project. Create columns first.")?
        }
    };

    // Next position in that column
    let max_pos: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(&column_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let position = max_pos.unwrap_or(-1) + 1;

    let id = Uuid::now_v7().to_string();
    let meta_str = serde_json::to_string(
        &input
            .metadata_json
            .unwrap_or(serde_json::Value::Object(Default::default())),
    )
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO cards (id, project_id, card_type, title, description, column_id, position, created_by, metadata_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.project_id)
    .bind(card_type)
    .bind(&input.title)
    .bind(input.description.as_deref().unwrap_or(""))
    .bind(&column_id)
    .bind(position)
    .bind(created_by)
    .bind(&meta_str)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get_card(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created card".to_string())
}

#[derive(Debug, Default)]
pub struct UpdateCard {
    pub title: Option<String>,
    pub description: Option<String>,
    pub column_id: Option<String>,
    pub position: Option<i32>,
    pub assigned_to: Option<Option<String>>,
    pub metadata_json: Option<serde_json::Value>,
    pub archived_at: Option<Option<String>>,
}

pub async fn update_card(
    pool: &Pool<Sqlite>,
    card_id: &str,
    input: UpdateCard,
) -> Result<Option<Card>, String> {
    let existing = get_card(pool, card_id).await?;
    if existing.is_none() {
        return Ok(None);
    }

    if let Some(ref title) = input.title {
        sqlx::query("UPDATE cards SET title = ? WHERE id = ?")
            .bind(title)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref desc) = input.description {
        sqlx::query("UPDATE cards SET description = ? WHERE id = ?")
            .bind(desc)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref col) = input.column_id {
        // Verify target column exists and is in the same project
        let card = existing.as_ref().unwrap();
        if card.card_type == "goal" && col != &card.column_id {
            return Err(GOAL_MOVE_REFUSAL.to_string());
        }
        let target_col = get_column(pool, col).await?;
        match target_col {
            Some(c) if c.project_id == card.project_id => {}
            Some(_) => return Err("Target column belongs to a different project".to_string()),
            None => return Err(format!("Column '{}' not found", col)),
        }
        sqlx::query("UPDATE cards SET column_id = ? WHERE id = ?")
            .bind(col)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(pos) = input.position {
        sqlx::query("UPDATE cards SET position = ? WHERE id = ?")
            .bind(pos)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref assigned) = input.assigned_to {
        sqlx::query("UPDATE cards SET assigned_to = ? WHERE id = ?")
            .bind(assigned.as_deref())
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref meta) = input.metadata_json {
        let card = existing.as_ref().unwrap();
        if card.card_type == "goal" {
            check_protected_metadata(&card.metadata_json, meta)?;
        }
        let meta_str = serde_json::to_string(meta).map_err(|e| e.to_string())?;
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(&meta_str)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref archived) = input.archived_at {
        sqlx::query("UPDATE cards SET archived_at = ? WHERE id = ?")
            .bind(archived.as_deref())
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    get_card(pool, card_id).await
}

pub async fn delete_card(pool: &Pool<Sqlite>, card_id: &str) -> Result<bool, String> {
    // Goal deletion is a Tier-2 action (user_data_deletion): it requires a
    // risk_gate decision approved by Jesse, executed via
    // goal_transition::delete_goal_checked.
    if let Some(card) = get_card(pool, card_id).await? {
        if card.card_type == "goal" {
            return Err(
                "Goal cards cannot be deleted directly. Goal deletion is Tier 2 \
                 (user_data_deletion): file a risk_gate decision and have Jesse approve it."
                    .to_string(),
            );
        }
    }

    let result = sqlx::query("DELETE FROM cards WHERE id = ?")
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.rows_affected() > 0)
}

/// Move a card to a different column (and optionally reposition).
pub async fn move_card(
    pool: &Pool<Sqlite>,
    card_id: &str,
    column_id: &str,
    position: Option<i32>,
) -> Result<Option<Card>, String> {
    let card = get_card(pool, card_id).await?;
    let card = match card {
        Some(c) => c,
        None => return Ok(None),
    };

    // Goal lifecycle state is positional: refuse goal column changes here;
    // they must go through the goal-transition guard.
    if card.card_type == "goal" && column_id != card.column_id {
        return Err(GOAL_MOVE_REFUSAL.to_string());
    }

    // Verify target column is in the same project
    let target_col = get_column(pool, column_id)
        .await?
        .ok_or_else(|| format!("Column '{}' not found", column_id))?;
    if target_col.project_id != card.project_id {
        return Err("Target column belongs to a different project".to_string());
    }

    let pos = match position {
        Some(p) => p,
        None => {
            let max: Option<i32> = sqlx::query_scalar(
                "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
            )
            .bind(column_id)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
            max.unwrap_or(-1) + 1
        }
    };

    sqlx::query("UPDATE cards SET column_id = ?, position = ? WHERE id = ?")
        .bind(column_id)
        .bind(pos)
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    get_card(pool, card_id).await
}

/// Batch reorder: accepts a list of (card_id, column_id, position) tuples.
///
/// Refuses the entire batch if any entry would change a goal card's column —
/// goal lifecycle moves must go through the goal-transition guard.
pub async fn reorder_cards(
    pool: &Pool<Sqlite>,
    moves: &[(String, String, i32)],
) -> Result<(), String> {
    // Validate before applying anything.
    for (card_id, column_id, _position) in moves {
        if let Some(card) = get_card(pool, card_id).await? {
            if card.card_type == "goal" && column_id != &card.column_id {
                return Err(format!(
                    "Reorder batch refused: entry for card '{}' would change a goal's column. {}",
                    card_id, GOAL_MOVE_REFUSAL
                ));
            }
        }
    }

    for (card_id, column_id, position) in moves {
        sqlx::query("UPDATE cards SET column_id = ?, position = ? WHERE id = ?")
            .bind(column_id)
            .bind(position)
            .bind(card_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Narrow API for Lane L2's verification module (allowlist): writes ONLY the
/// `verification` metadata key on a goal card. Never moves cards, never
/// touches protected keys.
pub async fn set_goal_verification(
    pool: &Pool<Sqlite>,
    card_id: &str,
    verification: serde_json::Value,
) -> Result<(), String> {
    let card = get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{}' not found", card_id))?;
    if card.card_type != "goal" {
        return Err(format!("Card '{}' is not a goal", card_id));
    }
    let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
    meta.insert("verification".to_string(), verification);
    let meta_str = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(&meta_str)
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Count cards in a project, optionally filtered by card_type.
pub async fn count_cards(
    pool: &Pool<Sqlite>,
    project_id: &str,
    card_type: Option<&str>,
) -> Result<i32, String> {
    if let Some(ct) = card_type {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards WHERE project_id = ? AND card_type = ? AND archived_at IS NULL",
        )
        .bind(project_id)
        .bind(ct)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards WHERE project_id = ? AND archived_at IS NULL",
        )
        .bind(project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
    }
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

    #[tokio::test]
    async fn personal_project_columns_seeded() {
        let pool = test_pool().await;
        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].name, "Backlog");
        assert_eq!(cols[0].id, "col-personal-backlog");
        assert_eq!(cols[1].name, "Doing");
        assert_eq!(cols[2].name, "Done");
    }

    #[tokio::test]
    async fn seed_columns_idempotent() {
        let pool = test_pool().await;
        // Already seeded by init; re-seed should be no-op
        seed_default_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert_eq!(cols.len(), 3);
    }

    #[tokio::test]
    async fn create_and_get_card() {
        let pool = test_pool().await;
        let card = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Test Card".to_string(),
                description: Some("A test".to_string()),
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(card.title, "Test Card");
        assert_eq!(card.card_type, "standard");
        assert_eq!(card.column_id, "col-personal-backlog"); // first column
        assert_eq!(card.position, 0);
        assert_eq!(card.created_by, "user");

        let fetched = get_card(&pool, &card.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, "Test Card");
    }

    #[tokio::test]
    async fn card_type_validation() {
        let pool = test_pool().await;
        let err = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Bad".to_string(),
                card_type: Some("invalid".to_string()),
                description: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn move_card_between_columns() {
        let pool = test_pool().await;
        let card = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Movable".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(card.column_id, "col-personal-backlog");

        let moved = move_card(&pool, &card.id, "col-personal-doing", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved.column_id, "col-personal-doing");
    }

    #[tokio::test]
    async fn delete_column_refuses_with_cards() {
        let pool = test_pool().await;
        create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Blocker".to_string(),
                description: None,
                card_type: None,
                column_id: Some("col-personal-backlog".to_string()),
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let err = delete_column(&pool, "col-personal-backlog").await;
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("active card(s) present"));
    }

    #[tokio::test]
    async fn delete_column_succeeds_when_empty() {
        let pool = test_pool().await;
        // col-personal-done has no cards
        let ok = delete_column(&pool, "col-personal-done").await.unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn cascade_delete_project_removes_cards_and_columns() {
        let pool = test_pool().await;
        // Create a non-Personal project
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Deletable".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        seed_default_columns(&pool, &project.id).await.unwrap();
        let cols = list_columns(&pool, &project.id).await.unwrap();
        assert_eq!(cols.len(), 3);

        create_card(
            &pool,
            CreateCard {
                project_id: project.id.clone(),
                title: "Will die".to_string(),
                description: None,
                card_type: None,
                column_id: Some(cols[0].id.clone()),
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        // Delete project — should cascade
        crate::projects::delete_project(&pool, &project.id)
            .await
            .unwrap();

        let cols_after = list_columns(&pool, &project.id).await.unwrap();
        assert!(cols_after.is_empty());

        let cards_after = list_cards(&pool, &project.id, None, None).await.unwrap();
        assert!(cards_after.is_empty());
    }

    #[tokio::test]
    async fn update_card_fields() {
        let pool = test_pool().await;
        let card = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Original".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let updated = update_card(
            &pool,
            &card.id,
            UpdateCard {
                title: Some("Updated".to_string()),
                description: Some("New desc".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.description, "New desc");
    }

    #[tokio::test]
    async fn list_cards_filters() {
        let pool = test_pool().await;
        create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Standard 1".to_string(),
                description: None,
                card_type: Some("standard".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Goal 1".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let all = list_cards(&pool, PERSONAL_PROJECT_ID, None, None)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        let standards = list_cards(&pool, PERSONAL_PROJECT_ID, Some("standard"), None)
            .await
            .unwrap();
        assert_eq!(standards.len(), 1);
        assert_eq!(standards[0].title, "Standard 1");
    }

    #[tokio::test]
    async fn count_cards_works() {
        let pool = test_pool().await;
        create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "C1".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let total = count_cards(&pool, PERSONAL_PROJECT_ID, None).await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn create_column_and_delete() {
        let pool = test_pool().await;
        let col = create_column(
            &pool,
            CreateColumn {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                name: "Review".to_string(),
                position: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(col.name, "Review");
        assert_eq!(col.position, 3); // after Backlog(0), Doing(1), Done(2)

        let deleted = delete_column(&pool, &col.id).await.unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn reorder_cards_batch() {
        let pool = test_pool().await;
        let c1 = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "A".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let c2 = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "B".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        // Move both to "doing" in reversed order
        reorder_cards(
            &pool,
            &[
                (c2.id.clone(), "col-personal-doing".to_string(), 0),
                (c1.id.clone(), "col-personal-doing".to_string(), 1),
            ],
        )
        .await
        .unwrap();

        let cards = list_cards(&pool, PERSONAL_PROJECT_ID, None, Some("col-personal-doing"))
            .await
            .unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].title, "B");
        assert_eq!(cards[1].title, "A");
    }

    #[tokio::test]
    async fn seed_goal_columns_creates_five_state_columns() {
        let pool = test_pool().await;
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        let state_cols: Vec<_> = cols.iter().filter(|c| c.column_kind == "state").collect();
        assert_eq!(state_cols.len(), 5);

        let bindings: Vec<_> = state_cols
            .iter()
            .map(|c| c.state_binding.as_deref().unwrap_or(""))
            .collect();
        assert!(bindings.contains(&"triage"));
        assert!(bindings.contains(&"ready"));
        assert!(bindings.contains(&"in_progress"));
        assert!(bindings.contains(&"review"));
        assert!(bindings.contains(&"complete"));

        // Verify positions are 100+
        for col in &state_cols {
            assert!(
                col.position >= 100,
                "State column position should be >= 100"
            );
        }
    }

    #[tokio::test]
    async fn seed_goal_columns_idempotent() {
        let pool = test_pool().await;
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        let state_cols: Vec<_> = cols.iter().filter(|c| c.column_kind == "state").collect();
        assert_eq!(state_cols.len(), 5, "Idempotent: should still be 5, not 10");
    }

    #[tokio::test]
    async fn create_goal_card_seeds_columns_and_places_in_triage() {
        let pool = test_pool().await;

        // No state columns before first goal
        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert!(
            cols.iter().all(|c| c.column_kind != "state"),
            "No state columns should exist before first goal"
        );

        let card = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "My Goal".to_string(),
                description: Some("Build something".to_string()),
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(card.card_type, "goal");

        // State columns should now exist
        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        let state_cols: Vec<_> = cols.iter().filter(|c| c.column_kind == "state").collect();
        assert_eq!(state_cols.len(), 5);

        // Card should be in the Triage column
        let triage = cols
            .iter()
            .find(|c| c.state_binding.as_deref() == Some("triage"))
            .expect("Triage column should exist");
        assert_eq!(card.column_id, triage.id);
    }

    // ── Goal hardening (Decision Inbox S1) ──

    async fn make_goal(pool: &Pool<Sqlite>) -> Card {
        create_card(
            pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Hardened goal".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: Some(serde_json::json!({"attempt_count": 1})),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn move_card_refuses_goal_column_change() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        let ready = get_goal_column(&pool, PERSONAL_PROJECT_ID, "ready")
            .await
            .unwrap()
            .unwrap();
        let err = move_card(&pool, &goal.id, &ready.id, None).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("decision inbox"));

        // Repositioning within the same column is still allowed.
        let ok = move_card(&pool, &goal.id, &goal.column_id, Some(5)).await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn update_card_refuses_goal_column_change() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        let complete = get_goal_column(&pool, PERSONAL_PROJECT_ID, "complete")
            .await
            .unwrap()
            .unwrap();
        let err = update_card(
            &pool,
            &goal.id,
            UpdateCard {
                column_id: Some(complete.id),
                ..Default::default()
            },
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn update_card_refuses_protected_metadata_writes() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;

        for (key, value) in [
            ("goal_state", serde_json::json!("complete")),
            ("needs_human_attention", serde_json::json!(false)),
            ("attempt_count", serde_json::json!(0)),
            ("last_error", serde_json::json!("forged")),
            ("budget", serde_json::json!({"attempt_cap": 999})),
            ("completed_at", serde_json::json!("2026-01-01T00:00:00Z")),
        ] {
            let mut meta = goal.metadata_json.as_object().cloned().unwrap();
            meta.insert(key.to_string(), value);
            let err = update_card(
                &pool,
                &goal.id,
                UpdateCard {
                    metadata_json: Some(serde_json::Value::Object(meta)),
                    ..Default::default()
                },
            )
            .await;
            assert!(err.is_err(), "protected key '{}' must be refused", key);
            assert!(err.unwrap_err().contains(key));
        }
    }

    #[tokio::test]
    async fn update_card_allows_unprotected_and_verification_metadata() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;

        let mut meta = goal.metadata_json.as_object().cloned().unwrap();
        meta.insert("tags".to_string(), serde_json::json!(["rust"]));
        meta.insert(
            "verification".to_string(),
            serde_json::json!({"status": "passed"}),
        );
        let updated = update_card(
            &pool,
            &goal.id,
            UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            updated
                .metadata_json
                .get("verification")
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str()),
            Some("passed")
        );

        // The narrow L2 API also works and touches only `verification`.
        set_goal_verification(&pool, &goal.id, serde_json::json!({"status": "failed"}))
            .await
            .unwrap();
        let after = get_card(&pool, &goal.id).await.unwrap().unwrap();
        assert_eq!(
            after
                .metadata_json
                .get("verification")
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str()),
            Some("failed")
        );
        assert_eq!(
            after.metadata_json.get("attempt_count").and_then(|v| v.as_u64()),
            Some(1),
            "protected keys untouched"
        );
    }

    #[tokio::test]
    async fn reorder_refuses_goal_column_change_whole_batch() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        let standard = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Standard".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let ready = get_goal_column(&pool, PERSONAL_PROJECT_ID, "ready")
            .await
            .unwrap()
            .unwrap();

        let err = reorder_cards(
            &pool,
            &[
                (standard.id.clone(), "col-personal-doing".to_string(), 0),
                (goal.id.clone(), ready.id.clone(), 1),
            ],
        )
        .await;
        assert!(err.is_err());

        // Nothing in the batch was applied.
        let std_after = get_card(&pool, &standard.id).await.unwrap().unwrap();
        assert_eq!(std_after.column_id, "col-personal-backlog");

        // Goal repositioning within its own column is fine.
        reorder_cards(&pool, &[(goal.id.clone(), goal.column_id.clone(), 7)])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_card_refuses_goals() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        let err = delete_card(&pool, &goal.id).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Tier 2"));
        assert!(get_card(&pool, &goal.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn get_goal_column_returns_correct_column() {
        let pool = test_pool().await;
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let triage = get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap();
        assert!(triage.is_some());
        assert_eq!(triage.unwrap().name, "Triage");

        let review = get_goal_column(&pool, PERSONAL_PROJECT_ID, "review")
            .await
            .unwrap();
        assert!(review.is_some());
        assert_eq!(review.unwrap().name, "Review");

        let nonexistent = get_goal_column(&pool, PERSONAL_PROJECT_ID, "nonexistent")
            .await
            .unwrap();
        assert!(nonexistent.is_none());
    }
}
