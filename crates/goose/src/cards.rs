//! Cards module — CRUD operations for the cards and board_columns tables.
//!
//! Goal-card hardening (Decision Inbox, S1): goal lifecycle state lives in
//! `column_id` + protected metadata keys, and every daemon path (HTTP, MCP
//! tools, background tasks) converges on the functions in this module. They
//! therefore REFUSE goal column changes and protected-metadata writes; the
//! sole legal mutator is [`crate::goal_transition::advance_goal_checked`]
//! (and its audited sibling paths), which performs its own guarded writes.

use sqlx::{Pool, Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::goal_transition::PROTECTED_GOAL_METADATA_KEYS;
use crate::projects::PERSONAL_PROJECT_ID;

/// Announce that a project's Kanban board changed.
///
/// The frame is `project_changed(project_id, "cards")` rather than a new
/// `card_changed` type, because that is what the board actually listens to:
/// `livenessSync.ts` maps `project_changed` → `bumpProjects()`, and
/// `ProjectKanban` refetches `/columns` + `/cards` off `projectsRev`. Inventing
/// a card frame would mean a second wire type, a second store counter, and a
/// second subscription for a surface that already refetches the whole board in
/// one shot — an emit nobody listens to is worse than no emit, and a listener
/// that duplicates an existing one is only marginally better.
///
/// Emitted here, on the shared writer, so a card the agent creates or moves
/// announces itself exactly like one dragged in the UI. Before this, neither
/// path emitted at all: card writes were invisible to every other open client.
fn announce_board_change(project_id: &str) {
    crate::events::emit(crate::events::project_changed(project_id, "cards"));
}

/// Check a metadata replacement against the protected-key set for goal cards.
/// Any change (add / remove / modify) to a protected key is refused. The
/// `dispatch_evidence` key is deliberately NOT protected (Lane L2 appends\n/// its `verdict` sub-object there, #466).
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

/// The lifecycle columns seeded for projects using goal cards. Cancelled
/// (#490) is a terminal column at the end — goals the user abandoned land here
/// and leave the active set permanently. Failed (#250) holds exhausted goals
/// (budget/timeout/credential block) so a parked failure is visibly distinct
/// from a fresh Triage goal; it is retriable via the goal's unblock decision.
pub const GOAL_COLUMNS: &[(&str, &str, i32)] = &[
    ("triage", "Triage", 100),
    ("ready", "Ready", 101),
    ("in_progress", "In Progress", 102),
    ("review", "Review", 103),
    ("complete", "Complete", 104),
    ("cancelled", "Cancelled", 105),
    ("failed", "Failed", 106),
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

    // #453: a project is seeded with generic Backlog/Doing/Done columns at
    // creation; the goal lifecycle columns just added supersede them. Drop the
    // now-redundant empty manual columns so the board shows one canonical set.
    cleanup_duplicate_manual_columns(pool).await?;

    Ok(())
}

/// Seed the goal lifecycle columns using a caller-owned transaction.  Unlike
/// [`seed_goal_columns`], this deliberately emits no board event and never
/// commits: roadmap materialization needs columns, cards, and root promotion
/// to succeed or roll back together.
pub(crate) async fn seed_goal_columns_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<(), String> {
    for (state_binding, name, position) in GOAL_COLUMNS {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM board_columns WHERE project_id = ? AND state_binding = ?)",
        )
        .bind(project_id)
        .bind(state_binding)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        if present {
            continue;
        }
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
             VALUES (?, ?, ?, ?, 'state', ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(project_id)
        .bind(name)
        .bind(position)
        .bind(state_binding)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Match normal seeding's data-safe cleanup: only empty manual columns are
    // removed, and cards in legacy columns are never touched here.
    sqlx::query(
        "DELETE FROM board_columns
         WHERE column_kind = 'manual' AND project_id = ?
           AND id NOT IN (SELECT column_id FROM cards WHERE column_id IS NOT NULL)",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Insert a goal card into a caller-owned transaction.  This is intentionally
/// narrower than the public CRUD API: the roadmap handler has already built
/// and validated the protected goal metadata and lifecycle column.
pub(crate) async fn create_goal_card_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    title: &str,
    description: &str,
    column_id: &str,
    metadata_json: &serde_json::Value,
) -> Result<String, String> {
    let max_pos: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
    )
    .bind(column_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    let id = Uuid::now_v7().to_string();
    let metadata = serde_json::to_string(metadata_json).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO cards (id, project_id, card_type, title, description, column_id, position, created_by, metadata_json)
         VALUES (?, ?, 'goal', ?, ?, ?, ?, 'user', ?)",
    )
    .bind(&id)
    .bind(project_id)
    .bind(title)
    .bind(description)
    .bind(column_id)
    .bind(max_pos.unwrap_or(-1) + 1)
    .bind(metadata)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Backfill the `cancelled` lifecycle column (#490) for existing boards.
///
/// `seed_goal_columns` is short-circuited once a project has any `state`
/// columns, so projects seeded before the Cancelled column existed never get
/// it from seeding alone — and `advance_goal_checked(Cancel)` needs the target
/// column to exist. This idempotent, base-independent backfill inserts a
/// Cancelled column into every project that has the lifecycle columns but lacks
/// `cancelled`. Safe to run on every boot. Returns the number of columns added.
pub async fn backfill_cancelled_column(pool: &Pool<Sqlite>) -> Result<u64, String> {
    // Projects that carry the goal lifecycle (state columns) but have no
    // cancelled column yet.
    let project_ids: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT project_id FROM board_columns
         WHERE column_kind = 'state'
           AND project_id NOT IN (
               SELECT project_id FROM board_columns WHERE state_binding = 'cancelled'
           )",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut added = 0u64;
    for project_id in &project_ids {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
             VALUES (?, ?, 'Cancelled', 105, 'state', 'cancelled')",
        )
        .bind(&id)
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        added += 1;
    }
    Ok(added)
}

/// Backfill the `failed` lifecycle column (#250) for existing boards.
///
/// Mirrors [`backfill_cancelled_column`]: `seed_goal_columns` short-circuits
/// once a project has any `state` columns, so boards seeded before the Failed
/// column existed never get it from seeding alone — and `park_goal` needs the
/// target column to exist. Idempotent, base-independent, safe on every boot.
/// Returns the number of columns added.
pub async fn backfill_failed_column(pool: &Pool<Sqlite>) -> Result<u64, String> {
    let project_ids: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT project_id FROM board_columns
         WHERE column_kind = 'state'
           AND project_id NOT IN (
               SELECT project_id FROM board_columns WHERE state_binding = 'failed'
           )",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut added = 0u64;
    for project_id in &project_ids {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
             VALUES (?, ?, 'Failed', 106, 'state', 'failed')",
        )
        .bind(&id)
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        added += 1;
    }
    Ok(added)
}

/// Remove redundant generic columns (#453). When a project carries the goal
/// lifecycle columns (`column_kind = 'state'`), the originally-seeded
/// Backlog/Doing/Done manual columns are duplicates (Doing≈In Progress,
/// Done≈Complete). Deletes ONLY empty manual columns — a manual column that
/// still holds cards is left untouched, so this can never lose card data.
/// Idempotent and safe to run on every boot / first goal seed.
pub async fn cleanup_duplicate_manual_columns(pool: &Pool<Sqlite>) -> Result<u64, String> {
    let res = sqlx::query(
        "DELETE FROM board_columns
         WHERE column_kind = 'manual'
           AND project_id IN (
               SELECT DISTINCT project_id FROM board_columns WHERE column_kind = 'state'
           )
           AND id NOT IN (
               SELECT column_id FROM cards WHERE column_id IS NOT NULL
           )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(res.rows_affected())
}

/// Consolidate the legacy manual "Doing"/"Done" columns into the canonical goal
/// lifecycle (#453 follow-up). For every project that has lifecycle (state)
/// columns: move "Doing" cards → In Progress and "Done" cards → Complete, then
/// delete the now-empty manual columns. "Backlog" is intentionally KEPT — it is
/// a legitimate created-but-not-ready state, distinct from Triage.
///
/// Card-data-safe: a manual column is only deleted after its cards have moved,
/// so a column whose target lifecycle state is somehow absent keeps its cards
/// and survives (mirrors the v14 cleanup invariant). Idempotent and
/// base-independent. Returns the number of columns removed.
pub async fn consolidate_doing_done_into_lifecycle(pool: &Pool<Sqlite>) -> Result<u64, String> {
    // (legacy manual column name, target lifecycle state_binding)
    const MOVES: &[(&str, &str)] = &[("Doing", "in_progress"), ("Done", "complete")];

    for (manual_name, target_state) in MOVES {
        // Move each manual column's cards into the matching lifecycle column,
        // scoped per-project. Only acts where the target state column exists.
        sqlx::query(
            "UPDATE cards
                SET column_id = (
                    SELECT s.id FROM board_columns s
                     WHERE s.project_id = cards.project_id AND s.state_binding = ?
                )
              WHERE column_id IN (
                    SELECT m.id FROM board_columns m
                     WHERE m.column_kind = 'manual' AND m.name = ?
                       AND EXISTS (
                           SELECT 1 FROM board_columns s2
                            WHERE s2.project_id = m.project_id AND s2.state_binding = ?
                       )
                )",
        )
        .bind(target_state)
        .bind(manual_name)
        .bind(target_state)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Delete the now-empty Doing/Done manual columns — only in projects that have
    // lifecycle columns, and only when no cards remain (data-safety invariant).
    // Backlog is excluded.
    let res = sqlx::query(
        "DELETE FROM board_columns
          WHERE column_kind = 'manual'
            AND name IN ('Doing', 'Done')
            AND project_id IN (
                SELECT DISTINCT project_id FROM board_columns WHERE column_kind = 'state'
            )
            AND id NOT IN (
                SELECT column_id FROM cards WHERE column_id IS NOT NULL
            )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(res.rows_affected())
}

/// Reconcile EVERY board to the canonical goal lifecycle (#502).
///
/// The v14/v15/v16 column fixups were all gated on a board already having
/// lifecycle (`column_kind = 'state'`) columns — and those are only seeded on a
/// board's first goal card (`seed_goal_columns`). Boards that never held a goal
/// card kept only the default manual Backlog/Doing/Done columns and were
/// invisible to every prior migration, so they still show the legacy set. This
/// applies the canonical lifecycle to ALL boards by, for every project:
/// (1) seeding any missing `GOAL_COLUMNS` (Triage..Cancelled), then
/// (2) running the existing Doing→In Progress / Done→Complete consolidation,
/// which now reaches every board (each has lifecycle columns after step 1):
/// cards move first, then the emptied Doing/Done manual columns drop.
/// "Backlog" is intentionally kept (a legitimate created-but-not-ready state).
///
/// Idempotent, base-version independent, and card-data-safe (it reuses the
/// move-then-delete consolidation). Returns the number of legacy columns removed.
pub async fn reconcile_all_boards_to_canonical(pool: &Pool<Sqlite>) -> Result<u64, String> {
    // Every project that has any columns at all (i.e. a real board).
    let project_ids: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT project_id FROM board_columns")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

    for project_id in &project_ids {
        // Existing state_bindings on this board.
        let present: Vec<String> = sqlx::query_scalar(
            "SELECT state_binding FROM board_columns
             WHERE project_id = ? AND column_kind = 'state' AND state_binding IS NOT NULL",
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        // Insert any canonical lifecycle column this board is missing.
        for (state_binding, name, position) in GOAL_COLUMNS {
            if present.iter().any(|p| p == state_binding) {
                continue;
            }
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
    }

    // Now that every board carries the lifecycle columns, the standard
    // consolidation reaches them all: move Doing→In Progress, Done→Complete,
    // then drop the emptied legacy columns. Card-data-safe and idempotent.
    consolidate_doing_done_into_lifecycle(pool).await
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

    let column = get_column(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created column".to_string())?;
    announce_board_change(&column.project_id);
    Ok(column)
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
    let Some(existing) = existing else {
        return Ok(None);
    };

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

    let updated = get_column(pool, column_id).await?;
    announce_board_change(&existing.project_id);
    Ok(updated)
}

pub async fn delete_column(pool: &Pool<Sqlite>, column_id: &str) -> Result<bool, String> {
    let project_id = match get_column(pool, column_id).await? {
        Some(c) => c.project_id,
        None => return Ok(false),
    };

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

    let deleted = result.rows_affected() > 0;
    if deleted {
        announce_board_change(&project_id);
    }
    Ok(deleted)
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

    // `sql` only appends fixed literal fragments above; caller values are bound.
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
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
    // Agents allowed to author a card. This list is MIRRORED BY A DB CHECK
    // constraint on `cards.created_by` — widening it here alone does not work:
    // the insert then passes Rust validation and fails at the database with an
    // opaque constraint error instead of a clear one. Adding an author means
    // an in-place table rebuild (the decisions.kind precedent), so it is not a
    // one-line change. Agents not on this list author as "henry" and record
    // their true identity in `metadata_json` instead.
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

    let card = get_card(pool, &id)
        .await?
        .ok_or_else(|| "Failed to read created card".to_string())?;

    // Live push so a newly-created goal appears on the board / dashboard
    // without a reload. Non-goal cards don't drive the goal surfaces.
    if card_type == "goal" {
        let to_binding: Option<String> =
            sqlx::query_scalar("SELECT state_binding FROM board_columns WHERE id = ?")
                .bind(&column_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?
                .flatten();
        crate::events::emit(crate::events::goal_state_changed(
            &id,
            Some(&input.project_id),
            None,
            to_binding.as_deref().unwrap_or("triage"),
            &card.created_by,
        ));
    }

    // Every card, goal or not: the Kanban board is what changed, and
    // `goal_state_changed` only reaches the goal surfaces.
    announce_board_change(&card.project_id);
    Ok(card)
}

/// Metadata key holding a to-do's due date (ISO-8601 date, `YYYY-MM-DD`).
///
/// Due dates live in `metadata_json` rather than in a column: the field is new
/// and unproven, and this way it costs no migration and can be withdrawn as
/// cheaply as it was added. If the dashboard earns its keep, promote it to a
/// real column with an index. It is deliberately NOT a protected goal key, so
/// the ordinary `update_card` path may write it.
pub const DUE_DATE_KEY: &str = "dueDate";

/// Metadata key holding the RFC-3339 instant a to-do was dismissed from the
/// dashboard. Dismissal hides a to-do from the cross-project list WITHOUT
/// touching the card itself — the card stays exactly where it is on its board.
/// Rescheduling clears it (see [`set_card_due_date`]): choosing a new due date
/// is an explicit statement that you want the to-do back.
pub const DUE_DISMISSED_KEY: &str = "dueDismissedAt";

/// Metadata key holding when a `social_post` is scheduled to go out (RFC-3339
/// instant, UTC).
///
/// Lives in `metadata_json` rather than a column for the same reason as
/// [`DUE_DATE_KEY`]: no migration, cheap to withdraw if the content calendar
/// does not earn a dedicated schema. It is deliberately NOT a protected goal
/// key, so the ordinary `update_card` path may write it.
pub const POST_SCHEDULED_FOR_KEY: &str = "scheduledFor";

/// Metadata key holding a `social_post`'s lifecycle status:
/// `"draft"` | `"scheduled"` | `"posted"`.
///
/// Same rationale as [`POST_SCHEDULED_FOR_KEY`]: metadata rather than a column
/// so the field can land without a migration and be withdrawn cheaply. Not a
/// protected goal key — ordinary `update_card` may write it.
pub const POST_STATUS_KEY: &str = "postStatus";

/// Accepted `postStatus` values for [`POST_STATUS_KEY`].
const POST_STATUSES: &[&str] = &["draft", "scheduled", "posted"];

/// `social_post` format: `"text"` | `"carousel"` | `"reel"` | `"compose"`.
pub const POST_FORMAT_KEY: &str = "format";
/// Channel slug the post is aimed at (`ig`, `li`, …). Not a Postiz id.
pub const POST_CHANNEL_KEY: &str = "channel";
/// Why this beat exists: `"blog"` | `"feature"` | `"origin"` | `"insight"`.
pub const POST_HARVEST_KIND_KEY: &str = "harvestKind";
/// `"queued"` | `"generating"` | `"ready"` | `"failed"`.
pub const POST_MEDIA_STATUS_KEY: &str = "mediaStatus";
/// Array of `{ kind, file, source, prompt? }` — filenames only, never host paths.
pub const POST_MEDIA_KEY: &str = "media";
/// Last media-job error, when [`POST_MEDIA_STATUS_KEY`] is `"failed"`.
pub const POST_MEDIA_ERROR_KEY: &str = "mediaError";
pub const POST_ARC_ID_KEY: &str = "arcId";
pub const POST_BEAT_INDEX_KEY: &str = "beatIndex";
/// `updated_at` of the project brand bag the still was generated against.
pub const POST_BRAND_REV_KEY: &str = "brandRev";
/// User/agent taste notes for the next still. Copy (title/description) is never
/// stored here — regenerating media must not rewrite the post.
pub const POST_MEDIA_FEEDBACK_KEY: &str = "mediaFeedback";
/// Postiz post id after Approve submitted this card. Server-owned.
pub const POST_PUBLISHER_POST_ID_KEY: &str = "publisherPostId";
/// Postiz integration id the post was scheduled against.
pub const POST_PUBLISHER_INTEGRATION_KEY: &str = "publisherIntegrationId";

/// Keys the Grow media job owns. HTTP and agent patches keep the existing
/// values so a stale calendar poll cannot wipe a finished still.
pub const SOCIAL_POST_MEDIA_KEYS: &[&str] = &[
    POST_MEDIA_STATUS_KEY,
    POST_MEDIA_KEY,
    POST_MEDIA_ERROR_KEY,
    POST_BRAND_REV_KEY,
    POST_MEDIA_FEEDBACK_KEY,
    POST_PUBLISHER_POST_ID_KEY,
    POST_PUBLISHER_INTEGRATION_KEY,
];

const POST_FORMATS: &[&str] = &["text", "carousel", "reel", "compose"];
const POST_HARVEST_KINDS: &[&str] = &["blog", "feature", "origin", "insight"];
const POST_MEDIA_STATUSES: &[&str] = &["queued", "generating", "ready", "failed"];

/// Copy server-owned media keys from `existing` onto `incoming` so a client
/// merge cannot clobber a still that finished after the client last fetched.
pub fn preserve_media_keys(
    existing: &serde_json::Value,
    mut incoming: serde_json::Value,
) -> serde_json::Value {
    let Some(obj) = incoming.as_object_mut() else {
        return incoming;
    };
    for key in SOCIAL_POST_MEDIA_KEYS {
        match existing.get(*key) {
            Some(v) => {
                obj.insert((*key).to_string(), v.clone());
            }
            None => {
                obj.remove(*key);
            }
        }
    }
    incoming
}

/// Merge `patch` into the card's metadata object. `replace_media_keys` is
/// true only for the Grow media job — every other writer keeps existing
/// `media*` fields.
pub async fn merge_card_metadata(
    pool: &Pool<Sqlite>,
    card_id: &str,
    patch: serde_json::Value,
    replace_media_keys: bool,
) -> Result<Option<Card>, String> {
    let Some(existing) = get_card(pool, card_id).await? else {
        return Ok(None);
    };
    let mut map = existing
        .metadata_json
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(patch_obj) = patch.as_object() {
        for (k, v) in patch_obj {
            if !replace_media_keys
                && existing.card_type == "social_post"
                && SOCIAL_POST_MEDIA_KEYS.contains(&k.as_str())
            {
                continue;
            }
            map.insert(k.clone(), v.clone());
        }
    }
    update_card(
        pool,
        card_id,
        UpdateCard {
            metadata_json: Some(serde_json::Value::Object(map)),
            ..Default::default()
        },
    )
    .await
}

pub fn validate_social_format(format: &str) -> Result<(), String> {
    if POST_FORMATS.contains(&format) {
        Ok(())
    } else {
        Err(format!(
            "format must be one of {}, got '{format}'",
            POST_FORMATS.join(", ")
        ))
    }
}

pub fn validate_harvest_kind(kind: &str) -> Result<(), String> {
    if POST_HARVEST_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(format!(
            "harvestKind must be one of {}, got '{kind}'",
            POST_HARVEST_KINDS.join(", ")
        ))
    }
}

pub fn validate_media_status(status: &str) -> Result<(), String> {
    if POST_MEDIA_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(format!(
            "mediaStatus must be one of {}, got '{status}'",
            POST_MEDIA_STATUSES.join(", ")
        ))
    }
}

/// `scheduled` is refused until the still (and reel video, if any) is on disk.
pub fn assert_ready_to_schedule(metadata: &serde_json::Value) -> Result<(), String> {
    let media = metadata
        .get(POST_MEDIA_STATUS_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("queued");
    if media != "ready" {
        return Err(
            "Cannot schedule a post until its media is ready. Wait for the still \
             (and video, if this is a Reel) or retry generation from Grow."
                .to_string(),
        );
    }
    if metadata
        .get(POST_SCHEDULED_FOR_KEY)
        .and_then(|v| v.as_str())
        .is_none()
    {
        return Err("Cannot schedule a post with no scheduledFor instant.".to_string());
    }
    Ok(())
}

/// Validate social-post scheduling metadata before it is written.
///
/// `scheduled_for`, when present, must parse as RFC-3339. `status`, when
/// present, must be one of `draft` / `scheduled` / `posted`. Either argument
/// may be `None` (partial updates); absent fields are not checked.
pub fn validate_post_metadata(
    scheduled_for: Option<&str>,
    status: Option<&str>,
) -> Result<(), String> {
    if let Some(status) = status {
        if !POST_STATUSES.contains(&status) {
            return Err(format!(
                "postStatus must be one of \"draft\", \"scheduled\", or \"posted\", got '{status}'"
            ));
        }
    }
    if let Some(scheduled_for) = scheduled_for {
        chrono::DateTime::parse_from_rfc3339(scheduled_for).map_err(|_| {
            format!(
                "scheduledFor must be an RFC-3339 instant (e.g. 2026-08-15T18:00:00Z), got '{scheduled_for}'"
            )
        })?;
    }
    Ok(())
}

/// Column names treated as terminal for to-dos, lower-cased.
///
/// Standard (non-goal) cards sit on `manual` columns, which carry no
/// `state_binding` — unlike goal cards there is no structural signal that a
/// column means "finished", so the only available signal is the name. That is
/// a genuine limitation: rename Done to "Shipped" and its cards reappear as
/// to-dos. It is recorded rather than hidden, and is the first thing to fix if
/// due dates graduate to a real column.
/// Similarity at or above which two goal titles are treated as the same ask.
///
/// 0.90, and the 0.05 above the obvious 0.85 is load-bearing. Measured over all
/// 1169 same-project goal pairs in the live corpus, the band 0.85–0.95 holds
/// exactly three pairs, all scoring 0.889:
///
///   * "Add one-line comment to README and push to main" vs
///     "Add comment to README and push to main" — a FALSE positive. The second
///     was created 3 seconds after the first was parked, and the second is the
///     one that shipped.
///   * the footer pair (twice) — a TRUE positive.
///
/// A true and a false positive at the identical score means 0.85 has zero
/// separation, so it cannot be tuned into correctness. 0.90 admits only exact
/// restatements (every surviving pair scores 1.000) and blocks no real work.
///
/// The deliberate consequence: a REWORDED duplicate is not caught. On the day
/// this was written, six cards for one blog post would have collapsed to two,
/// not one — "…and mention app feature" vs "…and push to main" scores 0.615
/// for the same underlying ask. Title similarity cannot see intent. That gap is
/// closed by making retry re-dispatch the SAME card, not by loosening this.
pub const DUPLICATE_DICE_THRESHOLD: f64 = 0.90;

/// Tokens carrying no distinguishing signal in a goal title.
const TITLE_STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "and", "of", "for", "on", "in", "so", "it", "goes", "live", "with",
];

/// Production verbs that are freely interchangeable in a goal title — "Add X"
/// and "Write X" are the same ask. Deliberately excludes fix/update/remove,
/// which change what is being asked for.
const TITLE_SYNONYM_VERBS: &[&str] = &["add", "write", "create", "make", "build", "implement"];

/// Normalize a goal title to its distinguishing token set.
///
/// Splits on everything except `_ . - /` so identifiers like `VERIFY_A.md` and
/// paths survive whole; strips a `(retry)` / `(retry 2)` suffix, stopwords, and
/// interchangeable production verbs.
fn title_tokens(title: &str) -> std::collections::BTreeSet<String> {
    let lowered = title.to_lowercase();
    // Strip a trailing retry marker without a regex dependency.
    let stripped = match lowered.split_once("(retry") {
        Some((head, rest)) => match rest.split_once(')') {
            Some((_, tail)) => format!("{head}{tail}"),
            None => head.to_string(),
        },
        None => lowered,
    };
    stripped
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')))
        .filter(|t| !t.is_empty())
        .filter(|t| !TITLE_STOPWORDS.contains(t))
        .filter(|t| !TITLE_SYNONYM_VERBS.contains(t))
        .map(str::to_string)
        .collect()
}

/// Sørensen–Dice coefficient over normalized title tokens: `2|A∩B| / (|A|+|B|)`.
///
/// Deterministic and zero-LLM by design — a duplicate check that asks a model
/// is a duplicate check that can be talked out of its answer.
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let (ta, tb) = (title_tokens(a), title_tokens(b));
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let overlap = ta.intersection(&tb).count();
    (2.0 * overlap as f64) / (ta.len() + tb.len()) as f64
}

const TERMINAL_COLUMN_NAMES: &[&str] = &["done", "complete", "completed", "cancelled", "canceled"];

/// A to-do with a due date, resolved across every project for the dashboard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DueCard {
    pub id: String,
    pub title: String,
    pub project_id: String,
    pub project_name: String,
    pub column_id: String,
    pub column_name: String,
    /// ISO-8601 date (`YYYY-MM-DD`) parsed from `metadata_json.dueDate`.
    pub due_date: String,
    pub assigned_to: Option<String>,
    pub updated_at: String,
}

/// Every dated, unfinished to-do across all projects, soonest due first.
///
/// Scope, and why each exclusion is here:
/// - `card_type = 'standard'` — goal cards have their own governed lifecycle
///   and their own surfaces; mixing them in would invite moving a goal from a
///   to-do list, which the goal-transition guard forbids anyway.
/// - a due date is required — without this filter the "dashboard" is every
///   card on every board, which is not a priority view.
/// - not archived, not in a terminal column — finished work is not a to-do.
/// - not dismissed — the user said they did not want to see it.
///
/// Overdue items sort first naturally, since the ordering is by date ascending
/// and the caller groups by comparing against today.
pub async fn list_due_cards(pool: &Pool<Sqlite>) -> Result<Vec<DueCard>, String> {
    let placeholders = vec!["?"; TERMINAL_COLUMN_NAMES.len()].join(", ");
    let sql = format!(
        "SELECT c.id, c.title, c.project_id, c.column_id, c.assigned_to, c.updated_at, \
                bc.name AS column_name, \
                COALESCE(p.name, c.project_id) AS project_name, \
                json_extract(c.metadata_json, '$.{due}') AS due_date \
         FROM cards c \
         JOIN board_columns bc ON c.column_id = bc.id \
         LEFT JOIN projects p ON p.id = c.project_id \
         WHERE c.card_type = 'standard' \
           AND c.archived_at IS NULL \
           AND json_extract(c.metadata_json, '$.{due}') IS NOT NULL \
           AND json_extract(c.metadata_json, '$.{dismissed}') IS NULL \
           AND lower(bc.name) NOT IN ({terminal}) \
         ORDER BY due_date ASC, c.updated_at DESC",
        due = DUE_DATE_KEY,
        dismissed = DUE_DISMISSED_KEY,
        terminal = placeholders,
    );
    // `sql` interpolates only the const keys above and "?" placeholders
    // (count = TERMINAL_COLUMN_NAMES.len()) — no external data in the SQL text.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for name in TERMINAL_COLUMN_NAMES {
        q = q.bind(*name);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| DueCard {
            id: r.get("id"),
            title: r.get("title"),
            project_id: r.get("project_id"),
            project_name: r.get("project_name"),
            column_id: r.get("column_id"),
            column_name: r.get("column_name"),
            due_date: r.get::<Option<String>, _>("due_date").unwrap_or_default(),
            assigned_to: r.get("assigned_to"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Set or clear a to-do's due date, merging into existing metadata.
///
/// Passing `None` clears the date, which also removes the card from the
/// dashboard list. Any change to the date clears a previous dismissal — the
/// user rescheduling something is asking to be reminded of it again.
pub async fn set_card_due_date(
    pool: &Pool<Sqlite>,
    card_id: &str,
    due_date: Option<&str>,
) -> Result<Option<Card>, String> {
    if let Some(date) = due_date {
        validate_due_date(date)?;
    }
    let Some(card) = get_card(pool, card_id).await? else {
        return Ok(None);
    };
    let mut metadata = card.metadata_json;
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    let map = metadata
        .as_object_mut()
        .ok_or_else(|| "Card metadata is not an object".to_string())?;
    match due_date {
        Some(date) => {
            map.insert(DUE_DATE_KEY.to_string(), serde_json::json!(date));
        }
        None => {
            map.remove(DUE_DATE_KEY);
        }
    }
    // Rescheduling un-dismisses: see DUE_DISMISSED_KEY.
    map.remove(DUE_DISMISSED_KEY);

    update_card(
        pool,
        card_id,
        UpdateCard {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
}

/// Hide a to-do from the dashboard list without altering the card.
///
/// `dismissed = false` restores it. This writes only the dismissal key; the
/// card's column, position, and due date are untouched, so dismissing from the
/// dashboard never silently reorganises someone's board.
pub async fn set_card_due_dismissed(
    pool: &Pool<Sqlite>,
    card_id: &str,
    dismissed: bool,
    now: &str,
) -> Result<Option<Card>, String> {
    let Some(card) = get_card(pool, card_id).await? else {
        return Ok(None);
    };
    let mut metadata = card.metadata_json;
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    let map = metadata
        .as_object_mut()
        .ok_or_else(|| "Card metadata is not an object".to_string())?;
    if dismissed {
        map.insert(DUE_DISMISSED_KEY.to_string(), serde_json::json!(now));
    } else {
        map.remove(DUE_DISMISSED_KEY);
    }

    update_card(
        pool,
        card_id,
        UpdateCard {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
}

/// Accept only `YYYY-MM-DD`. The value is interpolated nowhere, but a bad date
/// would sort into a nonsense position and silently corrupt the ordering, so it
/// is rejected at the edge instead of being stored and puzzled over later.
///
/// Public so every writer of a due date validates identically: the UI's PUT
/// route reaches it through [`set_card_due_date`], and the agent's card tools
/// call it directly to refuse a malformed date BEFORE a card is created — a
/// late rejection would leave an orphan card behind.
pub fn validate_due_date(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shaped {
        return Err(format!(
            "Due date must be an ISO-8601 calendar date (YYYY-MM-DD), got '{}'",
            value
        ));
    }
    // Read the digits out of the byte slice rather than re-slicing the &str:
    // every byte is already known to be ASCII, so this cannot split a
    // character, and it does not ask the reader to prove that.
    let digit = |i: usize| u32::from(bytes[i] - b'0');
    let month = digit(5) * 10 + digit(6);
    let day = digit(8) * 10 + digit(9);
    let year = digit(0) * 1000 + digit(1) * 100 + digit(2) * 10 + digit(3);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Err(format!("Due date '{}' is not a real calendar date", value));
    }
    Ok(())
}

/// Days in a Gregorian month. Keeps 31 February out of the ordering.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// A goal in an active lifecycle state, for the dashboard's unified "in flight"
/// surfaces. Count and list both derive from [`list_active_goals`] so they can
/// never disagree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActiveGoal {
    pub id: String,
    pub title: String,
    pub project_id: String,
    /// state_binding of the goal's column: ready | in_progress.
    pub state: String,
    pub assigned_to: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Human routing/hold line from metadata. Hidden when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_note: Option<String>,
}

/// All goals Henry is actively working — the single source of truth for the
/// "in flight" count, list, "N active" header, and "working on N things"
/// status. Active = [`GoalState::ACTIVE_BINDINGS`] (Ready/InProgress); Review
/// (waiting on the user — surfaced in the Decision Inbox), Triage (queued) and
/// Complete (done) are excluded, as are parked goals
/// (`needs_human_attention`) and archived cards. Newest first.
pub async fn list_active_goals(pool: &Pool<Sqlite>) -> Result<Vec<ActiveGoal>, String> {
    use crate::goal_state::GoalState;
    let placeholders = vec!["?"; GoalState::ACTIVE_BINDINGS.len()].join(", ");
    let sql = format!(
        "SELECT c.id, c.title, c.project_id, bc.state_binding, c.assigned_to, \
                c.created_at, c.updated_at, c.metadata_json \
         FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.card_type = 'goal' AND c.archived_at IS NULL \
           AND bc.state_binding IN ({}) \
           AND COALESCE(json_extract(c.metadata_json, '$.needs_human_attention'), 0) = 0 \
         ORDER BY c.updated_at DESC",
        placeholders
    );
    // `sql` interpolates only "?" placeholders (count = ACTIVE_BINDINGS.len()).
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for b in GoalState::ACTIVE_BINDINGS {
        q = q.bind(*b);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| active_goal_from_row(&r)).collect())
}

fn active_goal_from_row(r: &sqlx::sqlite::SqliteRow) -> ActiveGoal {
    let meta: Option<String> = r.get("metadata_json");
    let meta: serde_json::Value = meta
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let obj = meta.as_object();
    let routing_note = obj
        .and_then(crate::cost_router::RoutingSnapshot::from_metadata)
        .map(|s| s.note)
        .filter(|n| !n.is_empty());
    let hold_note = obj
        .and_then(|m| m.get(crate::cost_router::HOLD_METADATA_KEY))
        .and_then(|v| v.get("last_plan"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    ActiveGoal {
        id: r.get("id"),
        title: r.get("title"),
        project_id: r.get("project_id"),
        state: r
            .get::<Option<String>, _>("state_binding")
            .unwrap_or_default(),
        assigned_to: r.get("assigned_to"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        routing_note,
        hold_note,
    }
}

/// All non-archived goals attributed to one worker, across projects and in
/// every workflow state. Both attribution fields are queried because older
/// dispatches recorded `metadata_json.worker_key`, while the canonical goal
/// transition now writes `assigned_to`; ignoring either silently loses work.
pub async fn list_goals_for_worker(
    pool: &Pool<Sqlite>,
    worker_key: &str,
    limit: i64,
) -> Result<Vec<ActiveGoal>, String> {
    let rows = sqlx::query(
        "SELECT c.id, c.title, c.project_id, bc.state_binding, c.assigned_to, \
                c.created_at, c.updated_at, c.metadata_json \
         FROM cards c JOIN board_columns bc ON c.column_id = bc.id \
         WHERE c.card_type = 'goal' AND c.archived_at IS NULL \
           AND (c.assigned_to = ? OR json_extract(c.metadata_json, '$.worker_key') = ?) \
         ORDER BY c.updated_at DESC LIMIT ?",
    )
    .bind(worker_key)
    .bind(worker_key)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| active_goal_from_row(&r)).collect())
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
    let project_id = existing.as_ref().map(|c| c.project_id.clone());

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

    let updated = get_card(pool, card_id).await?;
    if let Some(project_id) = project_id {
        announce_board_change(&project_id);
    }
    Ok(updated)
}

pub async fn delete_card(pool: &Pool<Sqlite>, card_id: &str) -> Result<bool, String> {
    // Goal deletion is a Tier-2 action (user_data_deletion): it requires a
    // risk_gate decision approved by the user, executed via
    // goal_transition::delete_goal_checked.
    let project_id = match get_card(pool, card_id).await? {
        Some(card) => {
            if card.card_type == "goal" {
                return Err(
                    "Goal cards cannot be deleted directly. Goal deletion is Tier 2 \
                     (user_data_deletion): file a risk_gate decision and have the user approve it."
                        .to_string(),
                );
            }
            card.project_id
        }
        None => return Ok(false),
    };

    let result = sqlx::query("DELETE FROM cards WHERE id = ?")
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    // A card an agent briefed Henry about is now gone, so the briefing that
    // points at it is answered. Without this, Henry keeps raising a decision
    // that no longer exists. Best-effort — deleting a card must not fail
    // because a briefing could not be cleared.
    let deleted = result.rows_affected() > 0;
    if deleted {
        crate::briefings::resolve_for_ref(pool, "card", card_id).await;
        announce_board_change(&project_id);
    }

    Ok(deleted)
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

    let moved = get_card(pool, card_id).await?;
    announce_board_change(&card.project_id);
    Ok(moved)
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
    let mut projects: Vec<String> = Vec::new();
    for (card_id, column_id, _position) in moves {
        if let Some(card) = get_card(pool, card_id).await? {
            if card.card_type == "goal" && column_id != &card.column_id {
                return Err(format!(
                    "Reorder batch refused: entry for card '{}' would change a goal's column. {}",
                    card_id, GOAL_MOVE_REFUSAL
                ));
            }
            if !projects.contains(&card.project_id) {
                projects.push(card.project_id.clone());
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
    // One frame per board touched, not one per card: the listener refetches a
    // whole board either way, and a 200-card reorder must not become 200 frames
    // in a 1000-frame replay buffer.
    for project_id in &projects {
        announce_board_change(project_id);
    }
    Ok(())
}

/// Narrow API for Lane L2's verification module (allowlist): appends ONLY the
/// `verdict` sub-object onto the goal's `dispatch_evidence` metadata — one
/// evidence record per goal instead of two drifting keys (#466). Re-reads the
/// card internally so concurrent L1 metadata writes are preserved. Never
/// moves cards, never touches protected keys.
pub async fn set_goal_verdict(
    pool: &Pool<Sqlite>,
    card_id: &str,
    verdict: serde_json::Value,
) -> Result<(), String> {
    set_goal_verdict_and_program_receipts_inner(pool, card_id, Some(verdict), None, true).await
}

/// Narrow API for the trusted completion verifier: writes ONLY the typed
/// program gate receipts produced from declared deterministic checks. Generic
/// workers and ordinary metadata patches cannot mint this protected evidence.
pub async fn set_goal_program_receipts(
    pool: &Pool<Sqlite>,
    card_id: &str,
    receipts: serde_json::Value,
) -> Result<(), String> {
    set_goal_verdict_and_program_receipts_inner(pool, card_id, None, Some(receipts), false).await
}

/// Atomically persist the verifier verdict and the optional ProgramDag gate
/// receipts.  These values are produced by the same trusted verification run:
/// splitting them into two read/modify/write calls creates a race in which an
/// approval can observe a PASS before its gate receipts exist, or one writer
/// can clobber metadata written by the other.  The immediate transaction keeps
/// the read and CAS update together; the original JSON predicate also makes a
/// stale writer fail closed if this code is ever used with a non-locking
/// SQLite connection.
pub async fn set_goal_verdict_and_program_receipts(
    pool: &Pool<Sqlite>,
    card_id: &str,
    verdict: serde_json::Value,
    receipts: Option<serde_json::Value>,
) -> Result<(), String> {
    set_goal_verdict_and_program_receipts_inner(pool, card_id, Some(verdict), receipts, true).await
}

async fn set_goal_verdict_and_program_receipts_inner(
    pool: &Pool<Sqlite>,
    card_id: &str,
    verdict: Option<serde_json::Value>,
    receipts: Option<serde_json::Value>,
    clear_receipts_when_missing: bool,
) -> Result<(), String> {
    if let Some(receipts) = receipts.as_ref() {
        if !receipts.is_array() {
            return Err("program receipts must be an array".to_string());
        }
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    let row = sqlx::query(
        "SELECT card_type, metadata_json FROM cards WHERE id = ? AND archived_at IS NULL",
    )
    .bind(card_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| format!("Card '{}' not found", card_id))?;
    let card_type: String = row.get("card_type");
    if card_type != "goal" {
        return Err(format!("Card '{}' is not a goal", card_id));
    }
    let original_json: String = row.get("metadata_json");
    let mut meta: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str::<serde_json::Value>(&original_json)
            .map_err(|error| format!("goal metadata is invalid JSON: {error}"))?
            .as_object()
            .cloned()
            .unwrap_or_default();
    let mut evidence = meta
        .get("dispatch_evidence")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(verdict) = verdict {
        evidence.insert("verdict".to_string(), verdict);
        meta.insert(
            "dispatch_evidence".to_string(),
            serde_json::Value::Object(evidence),
        );
    }
    if let Some(receipts) = receipts {
        meta.insert("program_receipts".to_string(), receipts);
    } else if clear_receipts_when_missing {
        // A new verifier verdict without a fresh gate set must not inherit
        // receipts from an earlier run.  Keeping them would let a later
        // automatic handoff consume stale evidence for the new verdict.
        meta.remove("program_receipts");
    }
    let updated_json = serde_json::to_string(&serde_json::Value::Object(meta))
        .map_err(|error| error.to_string())?;
    let result = sqlx::query(
        "UPDATE cards
            SET metadata_json = ?, updated_at = CURRENT_TIMESTAMP
          WHERE id = ? AND metadata_json = ? AND archived_at IS NULL",
    )
    .bind(&updated_json)
    .bind(card_id)
    .bind(&original_json)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if result.rows_affected() != 1 {
        return Err(format!(
            "goal '{}' metadata changed while persisting verification; retry required",
            card_id
        ));
    }
    tx.commit().await.map_err(|error| error.to_string())
}

/// Narrow API for the orchestrator's completion tracker (allowlist): writes
/// ONLY the `dispatch_evidence` metadata key on a goal card — the deterministic
/// proof-of-work (commit SHAs, diffstat, push target, worker summary) captured
/// when an external-CLI goal completes. Never moves cards, never touches
/// protected keys. Mirrors [`set_goal_verdict`] (the L2 verifier's seam).
pub async fn set_goal_dispatch_evidence(
    pool: &Pool<Sqlite>,
    card_id: &str,
    evidence: serde_json::Value,
) -> Result<(), String> {
    set_goal_metadata_key_atomic(pool, card_id, "dispatch_evidence", evidence).await
}

/// Narrow API for the orchestrator's dispatch/heartbeat path (#210): writes
/// ONLY the `execution_receipt` metadata key on a goal card — the per-attempt
/// record of which worker ran, the routing snapshot, session id, lifecycle,
/// liveness heartbeat, and terminal state. Never moves cards, never touches
/// protected keys. Mirrors [`set_goal_dispatch_evidence`].
pub async fn set_goal_execution_receipt(
    pool: &Pool<Sqlite>,
    card_id: &str,
    receipt: serde_json::Value,
) -> Result<(), String> {
    set_goal_metadata_key_atomic(pool, card_id, "execution_receipt", receipt).await
}

/// Atomically patch one narrow, non-lifecycle metadata key while preserving
/// all other goal metadata. This is shared by completion evidence and receipt
/// writers because those writes frequently race with verifier/program writes.
async fn set_goal_metadata_key_atomic(
    pool: &Pool<Sqlite>,
    card_id: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    let row = sqlx::query(
        "SELECT card_type, metadata_json FROM cards WHERE id = ? AND archived_at IS NULL",
    )
    .bind(card_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| format!("Card '{}' not found", card_id))?;
    let card_type: String = row.get("card_type");
    if card_type != "goal" {
        return Err(format!("Card '{}' is not a goal", card_id));
    }
    let original_json: String = row.get("metadata_json");
    let mut metadata = serde_json::from_str::<serde_json::Value>(&original_json)
        .map_err(|error| format!("goal metadata is invalid JSON: {error}"))?
        .as_object()
        .cloned()
        .unwrap_or_default();
    metadata.insert(key.to_string(), value);
    let updated_json = serde_json::to_string(&serde_json::Value::Object(metadata))
        .map_err(|error| error.to_string())?;
    let result = sqlx::query(
        "UPDATE cards
            SET metadata_json = ?, updated_at = CURRENT_TIMESTAMP
          WHERE id = ? AND metadata_json = ? AND archived_at IS NULL",
    )
    .bind(&updated_json)
    .bind(card_id)
    .bind(&original_json)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if result.rows_affected() != 1 {
        return Err(format!(
            "goal '{}' metadata changed while persisting '{}'; retry required",
            card_id, key
        ));
    }
    tx.commit().await.map_err(|error| error.to_string())
}

/// Read the `execution_receipt` off a goal card's metadata, if present (#210).
pub async fn get_goal_execution_receipt(
    pool: &Pool<Sqlite>,
    card_id: &str,
) -> Result<Option<serde_json::Value>, String> {
    let card = get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{}' not found", card_id))?;
    Ok(card.metadata_json.get("execution_receipt").cloned())
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

/// Per-worker active load (#212): the number of goal cards currently
/// `in_progress`, grouped by their `worker_key` metadata.
///
/// This is the authoritative source for the orchestrator's `select_worker`
/// tie-break ("fewest active goals wins"). It counts goals rather than
/// SessionManager sessions because the goal card is tagged with `worker_key` on
/// EVERY dispatch — including external-CLI and supervised workers that never
/// create a SessionManager session — so the count is complete and engine-
/// agnostic. Best-effort: cards with no `worker_key` are skipped.
pub async fn active_worker_load(
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashMap<String, usize>, String> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT c.metadata_json FROM cards c
         JOIN board_columns bc ON c.column_id = bc.id
         WHERE c.card_type = 'goal'
           AND bc.state_binding = 'in_progress'
           AND c.archived_at IS NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut load: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (meta_str,) in rows {
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(&meta_str) else {
            continue;
        };
        if let Some(worker) = meta.get("worker_key").and_then(|v| v.as_str()) {
            *load.entry(worker.to_string()).or_insert(0) += 1;
        }
    }
    Ok(load)
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

    #[test]
    fn validate_post_metadata_accepts_valid_values() {
        assert!(validate_post_metadata(None, None).is_ok());
        assert!(validate_post_metadata(Some("2026-08-15T18:00:00Z"), Some("draft")).is_ok());
        assert!(
            validate_post_metadata(Some("2026-08-15T18:00:00+00:00"), Some("scheduled")).is_ok()
        );
        assert!(validate_post_metadata(None, Some("posted")).is_ok());
    }

    #[test]
    fn validate_post_metadata_rejects_bad_status_or_timestamp() {
        let status_err = validate_post_metadata(None, Some("queued")).unwrap_err();
        assert!(
            status_err.contains("draft") && status_err.contains("queued"),
            "unexpected: {status_err}"
        );
        let ts_err = validate_post_metadata(Some("2026-08-15"), None).unwrap_err();
        assert!(
            ts_err.contains("RFC-3339") && ts_err.contains("2026-08-15"),
            "unexpected: {ts_err}"
        );
    }

    // ── Due to-dos ─────────────────────────────────────────────────────────

    async fn todo(pool: &Pool<Sqlite>, title: &str, due: Option<&str>) -> Card {
        let card = create_card(
            pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: title.to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        if let Some(d) = due {
            set_card_due_date(pool, &card.id, Some(d))
                .await
                .unwrap()
                .unwrap()
        } else {
            card
        }
    }

    #[tokio::test]
    async fn due_cards_are_sorted_soonest_first() {
        let pool = test_pool().await;
        todo(&pool, "later", Some("2026-09-01")).await;
        todo(&pool, "sooner", Some("2026-08-05")).await;
        todo(&pool, "overdue", Some("2026-07-01")).await;

        let due = list_due_cards(&pool).await.unwrap();
        let titles: Vec<_> = due.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["overdue", "sooner", "later"]);
    }

    #[tokio::test]
    async fn undated_cards_are_excluded() {
        let pool = test_pool().await;
        todo(&pool, "dated", Some("2026-08-05")).await;
        todo(&pool, "undated", None).await;

        let due = list_due_cards(&pool).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].title, "dated");
    }

    #[tokio::test]
    async fn due_cards_carry_their_project_and_column_names() {
        let pool = test_pool().await;
        todo(&pool, "thing", Some("2026-08-05")).await;
        let due = list_due_cards(&pool).await.unwrap();
        assert_eq!(due[0].column_name, "Backlog");
        assert!(!due[0].project_name.is_empty());
        // The project name must resolve to a real name, not the raw id.
        assert_ne!(due[0].project_name, due[0].project_id);
    }

    #[tokio::test]
    async fn finished_todos_drop_off_the_list() {
        let pool = test_pool().await;
        let card = todo(&pool, "shipped", Some("2026-08-05")).await;
        assert_eq!(list_due_cards(&pool).await.unwrap().len(), 1);

        update_card(
            &pool,
            &card.id,
            UpdateCard {
                column_id: Some("col-personal-done".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(list_due_cards(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn archived_todos_drop_off_the_list() {
        let pool = test_pool().await;
        let card = todo(&pool, "archived", Some("2026-08-05")).await;
        update_card(
            &pool,
            &card.id,
            UpdateCard {
                archived_at: Some(Some("2026-08-02T00:00:00Z".to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(list_due_cards(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn dismissing_hides_the_todo_but_leaves_the_card_alone() {
        let pool = test_pool().await;
        let card = todo(&pool, "noisy", Some("2026-08-05")).await;
        let before = get_card(&pool, &card.id).await.unwrap().unwrap();

        set_card_due_dismissed(&pool, &card.id, true, "2026-08-02T12:00:00Z")
            .await
            .unwrap()
            .unwrap();
        assert!(list_due_cards(&pool).await.unwrap().is_empty());

        // The card itself must be untouched: same column, same position, same
        // due date. Dismissing from the dashboard is not a board edit.
        let after = get_card(&pool, &card.id).await.unwrap().unwrap();
        assert_eq!(after.column_id, before.column_id);
        assert_eq!(after.position, before.position);
        assert_eq!(
            after.metadata_json[DUE_DATE_KEY],
            before.metadata_json[DUE_DATE_KEY]
        );
        assert!(after.archived_at.is_none());
    }

    #[tokio::test]
    async fn undismissing_restores_the_todo() {
        let pool = test_pool().await;
        let card = todo(&pool, "back", Some("2026-08-05")).await;
        set_card_due_dismissed(&pool, &card.id, true, "2026-08-02T12:00:00Z")
            .await
            .unwrap();
        set_card_due_dismissed(&pool, &card.id, false, "2026-08-02T12:00:00Z")
            .await
            .unwrap();
        assert_eq!(list_due_cards(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rescheduling_clears_a_dismissal() {
        // Pushing a dismissed to-do to a new date is an explicit request to be
        // reminded again, so it must come back without a separate un-dismiss.
        let pool = test_pool().await;
        let card = todo(&pool, "revived", Some("2026-08-05")).await;
        set_card_due_dismissed(&pool, &card.id, true, "2026-08-02T12:00:00Z")
            .await
            .unwrap();
        assert!(list_due_cards(&pool).await.unwrap().is_empty());

        set_card_due_date(&pool, &card.id, Some("2026-09-09"))
            .await
            .unwrap();
        let due = list_due_cards(&pool).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].due_date, "2026-09-09");
    }

    #[tokio::test]
    async fn clearing_a_due_date_removes_it_from_the_list() {
        let pool = test_pool().await;
        let card = todo(&pool, "undated now", Some("2026-08-05")).await;
        set_card_due_date(&pool, &card.id, None).await.unwrap();
        assert!(list_due_cards(&pool).await.unwrap().is_empty());
        let after = get_card(&pool, &card.id).await.unwrap().unwrap();
        assert!(after.metadata_json.get(DUE_DATE_KEY).is_none());
    }

    #[tokio::test]
    async fn setting_a_due_date_preserves_other_metadata() {
        let pool = test_pool().await;
        let card = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "has meta".to_string(),
                description: None,
                card_type: None,
                column_id: None,
                created_by: None,
                metadata_json: Some(serde_json::json!({ "colour": "blue" })),
            },
        )
        .await
        .unwrap();
        let updated = set_card_due_date(&pool, &card.id, Some("2026-08-05"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.metadata_json["colour"], "blue");
        assert_eq!(updated.metadata_json[DUE_DATE_KEY], "2026-08-05");
    }

    #[tokio::test]
    async fn malformed_due_dates_are_refused() {
        let pool = test_pool().await;
        let card = todo(&pool, "bad date", None).await;
        for bad in [
            "05/08/2026",
            "2026-8-5",
            "tomorrow",
            "2026-13-01",
            "2026-08-32",
            "2026-00-10",
            "2026-01-00",
            "2026-02-30", // February never has 30 days
            "2026-04-31", // April has 30
            "2027-02-29", // 2027 is not a leap year
            "2026-08-0x",
            "",
        ] {
            assert!(
                set_card_due_date(&pool, &card.id, Some(bad)).await.is_err(),
                "expected '{}' to be refused",
                bad
            );
        }
        assert!(set_card_due_date(&pool, &card.id, Some("2026-08-05"))
            .await
            .is_ok());
        // Real leap day must still be accepted.
        assert!(set_card_due_date(&pool, &card.id, Some("2028-02-29"))
            .await
            .is_ok());
        assert!(set_card_due_date(&pool, &card.id, Some("2026-12-31"))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn goal_cards_never_appear_as_todos() {
        // Goals have a governed lifecycle and their own surfaces; a to-do list
        // that could move them would collide with the goal-transition guard.
        let pool = test_pool().await;
        let goal = create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "a goal".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: Some(serde_json::json!({ DUE_DATE_KEY: "2026-08-05" })),
            },
        )
        .await
        .unwrap();
        assert_eq!(goal.card_type, "goal");
        assert!(list_due_cards(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn setting_a_due_date_on_a_missing_card_reports_not_found() {
        let pool = test_pool().await;
        assert!(set_card_due_date(&pool, "no-such-card", Some("2026-08-05"))
            .await
            .unwrap()
            .is_none());
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
    async fn seed_goal_columns_creates_seven_state_columns() {
        let pool = test_pool().await;
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        let state_cols: Vec<_> = cols.iter().filter(|c| c.column_kind == "state").collect();
        assert_eq!(state_cols.len(), 7);

        let bindings: Vec<_> = state_cols
            .iter()
            .map(|c| c.state_binding.as_deref().unwrap_or(""))
            .collect();
        assert!(bindings.contains(&"triage"));
        assert!(bindings.contains(&"ready"));
        assert!(bindings.contains(&"in_progress"));
        assert!(bindings.contains(&"review"));
        assert!(bindings.contains(&"complete"));
        assert!(bindings.contains(&"cancelled"));
        assert!(bindings.contains(&"failed"));

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
        assert_eq!(state_cols.len(), 7, "Idempotent: should still be 7, not 14");
    }

    #[tokio::test]
    async fn backfill_cancelled_column_is_idempotent_and_base_independent() {
        let pool = test_pool().await;
        // Simulate a pre-#490 board: lifecycle columns WITHOUT the cancelled one.
        for (binding, name, position) in
            &[("triage", "Triage", 100), ("complete", "Complete", 104)][..]
        {
            sqlx::query(
                "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
                 VALUES (?, ?, ?, ?, 'state', ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(PERSONAL_PROJECT_ID)
            .bind(name)
            .bind(position)
            .bind(binding)
            .execute(&pool)
            .await
            .unwrap();
        }

        let added = backfill_cancelled_column(&pool).await.unwrap();
        assert_eq!(added, 1, "one cancelled column added");

        // Second run is a no-op (idempotent).
        let again = backfill_cancelled_column(&pool).await.unwrap();
        assert_eq!(again, 0, "no duplicate cancelled column");

        let col = get_goal_column(&pool, PERSONAL_PROJECT_ID, "cancelled")
            .await
            .unwrap();
        assert!(col.is_some(), "cancelled column exists after backfill");
    }

    #[tokio::test]
    async fn backfill_failed_column_is_idempotent_and_base_independent() {
        let pool = test_pool().await;
        // Simulate a pre-#250 board: lifecycle columns WITHOUT the failed one.
        for (binding, name, position) in
            &[("triage", "Triage", 100), ("cancelled", "Cancelled", 105)][..]
        {
            sqlx::query(
                "INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding)
                 VALUES (?, ?, ?, ?, 'state', ?)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(PERSONAL_PROJECT_ID)
            .bind(name)
            .bind(position)
            .bind(binding)
            .execute(&pool)
            .await
            .unwrap();
        }

        let added = backfill_failed_column(&pool).await.unwrap();
        assert_eq!(added, 1, "one failed column added");

        let again = backfill_failed_column(&pool).await.unwrap();
        assert_eq!(again, 0, "no duplicate failed column");

        let col = get_goal_column(&pool, PERSONAL_PROJECT_ID, "failed")
            .await
            .unwrap();
        assert!(col.is_some(), "failed column exists after backfill");
    }

    #[tokio::test]
    async fn seed_goal_columns_removes_empty_duplicate_manual_columns() {
        // #453: personal project starts with Backlog/Doing/Done (manual). After
        // goal columns are seeded, the empty manual duplicates are dropped.
        let pool = test_pool().await;
        assert_eq!(
            list_columns(&pool, PERSONAL_PROJECT_ID)
                .await
                .unwrap()
                .len(),
            3
        );

        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert!(
            cols.iter().all(|c| c.column_kind == "state"),
            "empty manual columns must be gone, leaving only the 7 state columns"
        );
        assert_eq!(cols.len(), 7);
    }

    #[tokio::test]
    async fn cleanup_keeps_manual_columns_that_hold_cards() {
        // A manual column with a card is NEVER deleted — no card data loss.
        let pool = test_pool().await;
        create_card(
            &pool,
            CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Generic task".to_string(),
                description: None,
                card_type: None, // standard → lands in Backlog (manual)
                column_id: Some("col-personal-backlog".to_string()),
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert!(
            cols.iter().any(|c| c.id == "col-personal-backlog"),
            "Backlog holds a card and must survive cleanup"
        );
        // Doing + Done were empty → removed; Backlog + 7 state remain.
        assert_eq!(cols.len(), 8);
    }

    #[tokio::test]
    async fn consolidate_moves_doing_done_cards_and_keeps_backlog() {
        // #453: non-empty Doing/Done columns (which the v14 empty-only cleanup
        // intentionally left alone) get their cards moved into the lifecycle and
        // are then removed. Backlog survives.
        let pool = test_pool().await;
        let mk = |col: &'static str, title: &'static str| {
            let pool = &pool;
            async move {
                create_card(
                    pool,
                    CreateCard {
                        project_id: PERSONAL_PROJECT_ID.to_string(),
                        title: title.to_string(),
                        description: None,
                        card_type: None,
                        column_id: Some(col.to_string()),
                        created_by: None,
                        metadata_json: None,
                    },
                )
                .await
                .unwrap()
            }
        };
        let backlog_card = mk("col-personal-backlog", "kept-backlog").await;
        let doing_card = mk("col-personal-doing", "moved-doing").await;
        let done_card = mk("col-personal-done", "moved-done").await;

        // Cards keep Doing/Done alive through the seed-time empty-only cleanup.
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let removed = consolidate_doing_done_into_lifecycle(&pool).await.unwrap();
        assert_eq!(removed, 2, "Doing + Done consolidated and removed");

        let cols = list_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();
        assert!(
            cols.iter().any(|c| c.id == "col-personal-backlog"),
            "Backlog must be kept"
        );
        assert!(
            !cols.iter().any(|c| c.name == "Doing" || c.name == "Done"),
            "Doing/Done must be gone"
        );

        // Cards landed in the canonical lifecycle columns; Backlog card untouched.
        let in_prog = get_goal_column(&pool, PERSONAL_PROJECT_ID, "in_progress")
            .await
            .unwrap()
            .unwrap();
        let complete = get_goal_column(&pool, PERSONAL_PROJECT_ID, "complete")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            get_card(&pool, &doing_card.id)
                .await
                .unwrap()
                .unwrap()
                .column_id,
            in_prog.id,
            "Doing card → In Progress"
        );
        assert_eq!(
            get_card(&pool, &done_card.id)
                .await
                .unwrap()
                .unwrap()
                .column_id,
            complete.id,
            "Done card → Complete"
        );
        assert_eq!(
            get_card(&pool, &backlog_card.id)
                .await
                .unwrap()
                .unwrap()
                .column_id,
            "col-personal-backlog",
            "Backlog card untouched"
        );
    }

    #[tokio::test]
    async fn list_active_goals_only_counts_active_states() {
        let pool = test_pool().await;
        seed_goal_columns(&pool, PERSONAL_PROJECT_ID).await.unwrap();

        let mk = |state: &'static str, parked: bool| {
            let pool = &pool;
            async move {
                let col = get_goal_column(pool, PERSONAL_PROJECT_ID, state)
                    .await
                    .unwrap()
                    .unwrap();
                let mut meta = serde_json::Map::new();
                meta.insert("goal_state".to_string(), serde_json::json!(state));
                if parked {
                    meta.insert("needs_human_attention".to_string(), serde_json::json!(true));
                }
                create_card(
                    pool,
                    CreateCard {
                        project_id: PERSONAL_PROJECT_ID.to_string(),
                        title: format!("goal-{}{}", state, if parked { "-parked" } else { "" }),
                        description: None,
                        card_type: Some("goal".to_string()),
                        column_id: Some(col.id),
                        created_by: None,
                        metadata_json: Some(serde_json::Value::Object(meta)),
                    },
                )
                .await
                .unwrap()
            }
        };

        mk("triage", false).await; // queued — excluded
        mk("ready", false).await; // active
        mk("in_progress", false).await; // active
        mk("review", false).await; // waiting on user — excluded (Decision Inbox)
        mk("in_progress", true).await; // parked — excluded

        let active = list_active_goals(&pool).await.unwrap();
        let states: Vec<&str> = active.iter().map(|g| g.state.as_str()).collect();
        assert_eq!(active.len(), 2, "ready + in_progress only; got {states:?}");
        assert!(states.contains(&"ready"));
        assert!(states.contains(&"in_progress"));
        assert!(
            !states.contains(&"review"),
            "Review is not in-flight — it lives in the Decision Inbox"
        );
        assert!(!states.contains(&"triage"));
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
        assert_eq!(state_cols.len(), 7);

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

        // The narrow L2 API touches only `dispatch_evidence.verdict` (#466),
        // preserving sibling evidence fields.
        set_goal_verdict(&pool, &goal.id, serde_json::json!({"status": "failed"}))
            .await
            .unwrap();
        let after = get_card(&pool, &goal.id).await.unwrap().unwrap();
        assert_eq!(
            after
                .metadata_json
                .get("dispatch_evidence")
                .and_then(|v| v.get("verdict"))
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str()),
            Some("failed")
        );
        assert_eq!(
            after
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1),
            "protected keys untouched"
        );
    }

    #[tokio::test]
    async fn verifier_combined_write_preserves_metadata_and_persists_receipts_atomically() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        sqlx::query("UPDATE cards SET metadata_json = json_set(metadata_json, '$.dispatch_evidence', json(?), '$.worker_note', 'keep') WHERE id = ?")
            .bind(serde_json::json!({"head_commit": "abc"}).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        set_goal_verdict_and_program_receipts(
            &pool,
            &goal.id,
            serde_json::json!({"status": "pass"}),
            Some(serde_json::json!([{"gate": "checks", "passed": true}])),
        )
        .await
        .unwrap();

        let after = get_card(&pool, &goal.id).await.unwrap().unwrap();
        assert_eq!(after.metadata_json["worker_note"], "keep");
        assert_eq!(
            after.metadata_json["dispatch_evidence"]["head_commit"],
            "abc"
        );
        assert_eq!(
            after.metadata_json["dispatch_evidence"]["verdict"]["status"],
            "pass"
        );
        assert_eq!(after.metadata_json["program_receipts"][0]["gate"], "checks");
    }

    #[tokio::test]
    async fn verifier_new_verdict_clears_receipts_from_an_older_run() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        set_goal_verdict_and_program_receipts(
            &pool,
            &goal.id,
            serde_json::json!({"status": "pass", "finished_at": "old-run"}),
            Some(serde_json::json!([{
                "gate": "checks",
                "passed": true,
                "verification_id": "old-run"
            }])),
        )
        .await
        .unwrap();

        // A new verdict without a freshly derived gate set is deliberately
        // incomplete; stale receipts must not survive it.
        set_goal_verdict(
            &pool,
            &goal.id,
            serde_json::json!({"status": "uncertain", "finished_at": "new-run"}),
        )
        .await
        .unwrap();
        let after = get_card(&pool, &goal.id).await.unwrap().unwrap();
        assert!(after.metadata_json.get("program_receipts").is_none());
        assert_eq!(
            after.metadata_json["dispatch_evidence"]["verdict"]["finished_at"],
            "new-run"
        );
    }

    #[tokio::test]
    async fn narrow_metadata_writers_preserve_protected_program_state() {
        let pool = test_pool().await;
        let goal = make_goal(&pool).await;
        sqlx::query("UPDATE cards SET metadata_json = json_set(metadata_json, '$.program_receipts', json(?), '$.program_transition', json(?)) WHERE id = ?")
            .bind(serde_json::json!([{"gate": "old", "passed": true}]).to_string())
            .bind(serde_json::json!({"digest": "keep"}).to_string())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        set_goal_dispatch_evidence(&pool, &goal.id, serde_json::json!({"files_changed": 0}))
            .await
            .unwrap();
        set_goal_execution_receipt(&pool, &goal.id, serde_json::json!({"state": "Completed"}))
            .await
            .unwrap();
        let after = get_card(&pool, &goal.id).await.unwrap().unwrap();
        assert_eq!(after.metadata_json["program_receipts"][0]["gate"], "old");
        assert_eq!(after.metadata_json["program_transition"]["digest"], "keep");
        assert_eq!(after.metadata_json["dispatch_evidence"]["files_changed"], 0);
        assert_eq!(
            after.metadata_json["execution_receipt"]["state"],
            "Completed"
        );
    }

    #[tokio::test]
    async fn reorder_refuses_goal_column_change_whole_batch() {
        let pool = test_pool().await;
        // Create the standard card first so its manual column holds a card and
        // survives the #453 duplicate-column cleanup that make_goal triggers.
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
        let std_col = standard.column_id.clone();
        let goal = make_goal(&pool).await;
        let ready = get_goal_column(&pool, PERSONAL_PROJECT_ID, "ready")
            .await
            .unwrap()
            .unwrap();

        // Batch pairs a legal same-column reposition of the standard card with
        // an illegal goal column change — the whole batch must be refused.
        let err = reorder_cards(
            &pool,
            &[
                (standard.id.clone(), std_col.clone(), 3),
                (goal.id.clone(), ready.id.clone(), 1),
            ],
        )
        .await;
        assert!(err.is_err());

        // Nothing in the batch was applied.
        let std_after = get_card(&pool, &standard.id).await.unwrap().unwrap();
        assert_eq!(std_after.column_id, std_col);
        assert_eq!(
            std_after.position, 0,
            "reposition rolled back with the batch"
        );

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

#[cfg(test)]
mod duplicate_title_tests {
    use super::{title_similarity, DUPLICATE_DICE_THRESHOLD};

    fn dup(a: &str, b: &str) -> bool {
        title_similarity(a, b) >= DUPLICATE_DICE_THRESHOLD
    }

    /// The pairs that actually happened, 2026-08-08: one blog post became six
    /// goal cards. Verb-only differences and a (retry) suffix must collapse.
    #[test]
    fn catches_the_restatements_that_produced_six_cards() {
        assert!(dup(
            "Add sleep training blog post and push to main",
            "Write sleep training blog post and push to main"
        ));
        assert!(dup(
            "Add sleep training blog post and mention app feature",
            "Add sleep training blog post and mention app feature (retry)"
        ));
        assert!(dup(
            "Build Teenity MVP foundation",
            "Teenity MVP foundation"
        ));
    }

    /// The false positive that fixes the threshold at 0.90.
    ///
    /// These score 0.889 — identical to the lowest TRUE positive in the corpus,
    /// so no threshold at or below 0.889 can separate them. The second card is
    /// the one that shipped; blocking it would have stopped real work.
    #[test]
    fn does_not_block_the_readme_pair_that_shipped() {
        let s = title_similarity(
            "Add one-line comment to README and push to main",
            "Add comment to README and push to main",
        );
        assert!(
            (s - 0.888).abs() < 0.01,
            "expected the measured 0.889, got {s}"
        );
        assert!(!dup(
            "Add one-line comment to README and push to main",
            "Add comment to README and push to main"
        ));
    }

    /// Genuinely different goals in one project must never collide.
    #[test]
    fn distinct_goals_are_not_duplicates() {
        assert!(!dup(
            "Add blue nav bar to Grocery Savers",
            "Change nav bar to a different shade of green"
        ));
        assert!(!dup(
            "Fix missing url_fr fields in sources and build",
            "Bug audit on World Litter Run and report findings"
        ));
        // fix/update/remove are NOT synonyms of add/write — they change the ask.
        assert!(!dup(
            "Add rate limiting to the API",
            "Remove rate limiting from the API"
        ));
    }

    /// Honest about the limit: a REWORDED intent is not caught, and this test
    /// records that rather than hiding it. Closing this gap is retry-in-place's
    /// job, not a lower threshold's.
    #[test]
    fn a_reworded_intent_is_not_caught_and_that_is_known() {
        let s = title_similarity(
            "Add sleep training blog post and mention app feature",
            "Add sleep training blog post and push to main",
        );
        assert!(s < DUPLICATE_DICE_THRESHOLD, "score was {s}");
    }

    /// Identifiers and paths must survive tokenization whole.
    #[test]
    fn identifiers_and_paths_are_not_split() {
        assert!(dup("Update VERIFY_A.md", "Update VERIFY_A.md"));
        assert!(!dup("Update VERIFY_A.md", "Update VERIFY_B.md"));
        assert!(!dup("Edit src/main.rs", "Edit src/lib.rs"));
    }

    /// An empty or stopword-only title has no signal and must never match.
    #[test]
    fn titles_with_no_signal_never_match() {
        assert_eq!(title_similarity("", ""), 0.0);
        assert_eq!(title_similarity("the a an", "the a an"), 0.0);
        assert_eq!(title_similarity("Add", "Write"), 0.0);
    }
}
