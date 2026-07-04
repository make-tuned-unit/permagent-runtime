//! Spectral schema initializer for permagent.db
//!
//! Creates the full Spectral schema (Phase 1 Architecture, Section B) including:
//! users, sessions, messages, memories, knowledge_graph, tasks, skills,
//! skill_executions, skill_triggers, integrations, plus FTS virtual tables,
//! triggers, views, and schema versioning.

use anyhow::Result;
use sqlx::{Pool, Sqlite};
use tracing::{info, warn};

/// Current Spectral schema version. Bump when adding migrations.
///
/// v10 = decision inbox (decisions, decision_audit, risk_policy), assigned by
/// Jesse 2026-06-15. v9 is reserved by the session-list-perf branch (committed
/// but unmerged: migrate_v8_to_v9 + idx_sessions_type_updated), so on THIS
/// branch the chain steps straight from v8 to v10 via `migrate_v9_to_v10` —
/// the absent v9 is intentional. Whichever of the two branches merges second
/// renumbers to sit directly above the first (see the integration-PR
/// sequencing note); `migrate_v9_to_v10` is base-independent so it is correct
/// over either a v8 or a v9 base.
///
/// v11 = recognition instrumentation (recognition_events, recognition_set_members),
/// the AmbientFrame emit-side substrate. New-tables-only, additive. Landed on
/// main via the Recognition branch; `migrate_v10_to_v11` applies it.
///
/// v12 = CRM people table. New-tables-only, additive. The chain is
/// v10 -> v11 -> v12, each step base-independent (idempotent additive
/// `CREATE TABLE IF NOT EXISTS`), so a v10 DB runs v11 then v12 and a v11 DB
/// runs only v12. `migrate_v11_to_v12` applies it.
///
/// v13 = file-intake inbox (inbox_files). New-tables-only, additive and
/// base-independent; browser downloads land as a file on disk plus a metadata
/// row here (epic #392 / #393). `migrate_v12_to_v13` applies it.
///
/// v14 = duplicate-column cleanup (#453). A data fixup, not new schema: existing
/// goal boards carry both the generic Backlog/Doing/Done columns (seeded at
/// project creation) and the goal lifecycle columns, showing 8 columns with
/// duplicates. `migrate_v13_to_v14` deletes the redundant EMPTY manual columns
/// (never one holding cards). Idempotent and base-independent.
///
/// NOTE on the version drift: this constant intentionally stays at 14 even though
/// the migration chain now runs to v19 (`migrate_v14_to_v15` … `migrate_v18_to_v19`).
/// It is the stamp `init_spectral_db` applies to a *fresh* DB — those later steps
/// are all idempotent, base-independent data fixups / drops that `init` already
/// reflects directly, so they re-run harmlessly on the next boot to advance an
/// existing DB. The const is therefore the fresh-init base stamp, NOT "latest";
/// migration gating in `SessionStorage::pool` uses hardcoded `version < N`
/// literals, not this value. Bumping it would skip the idempotent v15–v19 steps
/// on fresh installs — `verify_schema_version` only *warns* on the mismatch.
pub const SPECTRAL_SCHEMA_VERSION: i32 = 14;

/// Initialize the Spectral database schema from scratch.
/// Creates all tables, indexes, FTS virtual tables, triggers, and views.
/// Inserts the default user row for Phase 1 single-user operation.
pub async fn init_spectral_db(pool: &Pool<Sqlite>) -> Result<()> {
    // Enable WAL mode for crash safety
    sqlx::query("PRAGMA journal_mode=WAL").execute(pool).await?;

    let mut tx = pool.begin().await?;

    // ── Schema version table ──
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
        .bind(SPECTRAL_SCHEMA_VERSION)
        .execute(&mut *tx)
        .await?;

    // ── USERS ──
    sqlx::query(
        "CREATE TABLE users (
            id                TEXT PRIMARY KEY,
            display_name      TEXT NOT NULL,
            email             TEXT,
            provider_name     TEXT,
            model_config_json TEXT,
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    // Insert default user for Phase 1 single-user operation (Section B.0)
    sqlx::query("INSERT INTO users (id, display_name) VALUES ('default', 'Default User')")
        .execute(&mut *tx)
        .await?;

    // ── SESSIONS ──
    // Preserves all columns from the original Goose sessions table,
    // plus adds user_id for Phase 2 multi-user support.
    sqlx::query(
        "CREATE TABLE sessions (
            id                TEXT PRIMARY KEY,
            user_id           TEXT NOT NULL DEFAULT 'default' REFERENCES users(id),
            name              TEXT NOT NULL DEFAULT '',
            description       TEXT NOT NULL DEFAULT '',
            user_set_name     BOOLEAN DEFAULT FALSE,
            session_type      TEXT NOT NULL DEFAULT 'user',
            working_dir       TEXT NOT NULL,
            created_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            extension_data    TEXT DEFAULT '{}',
            total_tokens      INTEGER,
            input_tokens      INTEGER,
            output_tokens     INTEGER,
            accumulated_total_tokens  INTEGER,
            accumulated_input_tokens  INTEGER,
            accumulated_output_tokens INTEGER,
            schedule_id       TEXT,
            recipe_json       TEXT,
            user_recipe_values_json TEXT,
            provider_name     TEXT,
            model_config_json TEXT,
            goose_mode        TEXT NOT NULL DEFAULT 'auto',
            thread_id         TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_sessions_user ON sessions(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_sessions_type ON sessions(session_type)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_sessions_thread ON sessions(thread_id)")
        .execute(&mut *tx)
        .await?;

    // ── MESSAGES ──
    sqlx::query(
        "CREATE TABLE messages (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id        TEXT,
            session_id        TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            role              TEXT NOT NULL,
            content_json      TEXT NOT NULL,
            metadata_json     TEXT,
            tokens            INTEGER DEFAULT 0,
            created_timestamp INTEGER NOT NULL,
            timestamp         TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_messages_session ON messages(session_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_messages_timestamp ON messages(timestamp)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_messages_message_id ON messages(message_id)")
        .execute(&mut *tx)
        .await?;

    // ── THREADS (preserved from Goose for thread_manager compatibility) ──
    sqlx::query(
        "CREATE TABLE threads (
            id             TEXT PRIMARY KEY,
            name           TEXT NOT NULL DEFAULT 'New Chat',
            user_set_name  BOOLEAN DEFAULT FALSE,
            working_dir    TEXT,
            created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            archived_at    TIMESTAMP,
            metadata_json  TEXT DEFAULT '{}'
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE thread_messages (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id         TEXT NOT NULL REFERENCES threads(id),
            session_id        TEXT,
            message_id        TEXT,
            role              TEXT NOT NULL,
            content_json      TEXT NOT NULL,
            created_timestamp INTEGER NOT NULL,
            metadata_json     TEXT DEFAULT '{}'
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_thread_messages_thread ON thread_messages(thread_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_thread_messages_message_id ON thread_messages(message_id)")
        .execute(&mut *tx)
        .await?;

    // ── MEMORIES / KNOWLEDGE GRAPH (removed) ──
    // permagent.db once carried a dormant copy of the Spectral Phase-1 `memories`
    // and `knowledge_graph` tables (plus their FTS, triggers, and current_* views).
    // The live Brain lives in a separate file, `~/.permagent/brain/memory.db`
    // (via `read_only_brain_conn()` / `SafeBrain`), so these were never read or
    // written by the daemon/agent/GUI — only the now-removed `permagent memory`
    // CLI subcommand touched them, as a dead-end loop. They are dropped from
    // existing DBs by `migrate_v18_to_v19` and are no longer created here, leaving
    // one unambiguous `memories` table in the system (Spectral's).

    // ── TASKS ──
    sqlx::query(
        "CREATE TABLE tasks (
            id                  TEXT PRIMARY KEY,
            user_id             TEXT NOT NULL REFERENCES users(id),
            session_id          TEXT REFERENCES sessions(id),
            description         TEXT NOT NULL,
            tool_used           TEXT,
            argument_shape_hash TEXT,
            steps_json          TEXT,
            status              TEXT NOT NULL DEFAULT 'pending',
            input_json          TEXT,
            output_json         TEXT,
            error_message       TEXT,
            started_at          TEXT,
            completed_at        TEXT,
            created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_tasks_user ON tasks(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_tasks_status ON tasks(status)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_tasks_tool ON tasks(tool_used)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_tasks_completed ON tasks(completed_at DESC)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE INDEX idx_tasks_shape_repetition ON tasks(user_id, tool_used, argument_shape_hash, status, completed_at)",
    )
    .execute(&mut *tx)
    .await?;

    // ── SKILLS ──
    sqlx::query(
        "CREATE TABLE skills (
            id                TEXT PRIMARY KEY,
            user_id           TEXT NOT NULL REFERENCES users(id),
            name              TEXT NOT NULL,
            description       TEXT,
            definition_json   TEXT NOT NULL,
            trigger_type      TEXT NOT NULL DEFAULT 'manual',
            trigger_value     TEXT,
            status            TEXT NOT NULL DEFAULT 'active',
            version           INTEGER NOT NULL DEFAULT 1,
            source_task_id    TEXT REFERENCES tasks(id),
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_skills_user ON skills(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_skills_status ON skills(status)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_skills_trigger ON skills(trigger_type)")
        .execute(&mut *tx)
        .await?;

    // ── SKILL EXECUTIONS ──
    sqlx::query(
        "CREATE TABLE skill_executions (
            id                TEXT PRIMARY KEY,
            skill_id          TEXT NOT NULL REFERENCES skills(id),
            user_id           TEXT NOT NULL REFERENCES users(id),
            session_id        TEXT REFERENCES sessions(id),
            status            TEXT NOT NULL DEFAULT 'running',
            input_json        TEXT,
            output_json       TEXT,
            error_message     TEXT,
            started_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            completed_at      TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_skill_exec_skill ON skill_executions(skill_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_skill_exec_user ON skill_executions(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_skill_exec_status ON skill_executions(status)")
        .execute(&mut *tx)
        .await?;

    // ── SKILL TRIGGERS ──
    sqlx::query(
        "CREATE TABLE skill_triggers (
            id                TEXT PRIMARY KEY,
            skill_id          TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
            trigger_type      TEXT NOT NULL,
            trigger_config    TEXT,
            last_triggered_at TEXT,
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_skill_triggers_skill ON skill_triggers(skill_id)")
        .execute(&mut *tx)
        .await?;

    // ── SKILL DISMISSALS ──
    sqlx::query(
        "CREATE TABLE skill_dismissals (
            id                  TEXT PRIMARY KEY,
            user_id             TEXT NOT NULL REFERENCES users(id),
            tool_used           TEXT NOT NULL DEFAULT '',
            argument_shape_hash TEXT NOT NULL,
            dismissed_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX idx_skill_dismissals_lookup ON skill_dismissals(user_id, argument_shape_hash, dismissed_at)",
    )
    .execute(&mut *tx)
    .await?;

    // ── INTEGRATIONS ──
    sqlx::query(
        "CREATE TABLE integrations (
            id                TEXT PRIMARY KEY,
            user_id           TEXT NOT NULL REFERENCES users(id),
            provider          TEXT NOT NULL,
            status            TEXT NOT NULL DEFAULT 'pending',
            scopes_json       TEXT,
            last_sync_at      TEXT,
            error_message     TEXT,
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_integrations_user ON integrations(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_integrations_provider ON integrations(provider)")
        .execute(&mut *tx)
        .await?;

    // ── PROVIDER INVENTORY (preserved from Goose) ──
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS provider_inventory_entries (
            inventory_key TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            provider_family TEXT NOT NULL,
            last_updated_at TEXT,
            last_refresh_attempt_at TEXT,
            last_refresh_error TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS provider_inventory_models (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            inventory_key TEXT NOT NULL REFERENCES provider_inventory_entries(inventory_key) ON DELETE CASCADE,
            model_id TEXT NOT NULL,
            display_name TEXT,
            context_window INTEGER,
            supports_streaming BOOLEAN DEFAULT TRUE,
            supports_tools BOOLEAN DEFAULT TRUE,
            supports_images BOOLEAN DEFAULT FALSE,
            preferred BOOLEAN DEFAULT FALSE,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(inventory_key, model_id)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_inventory_models_key ON provider_inventory_models(inventory_key)")
        .execute(&mut *tx)
        .await?;

    // ── WORKSPACES ──
    sqlx::query(
        "CREATE TABLE workspaces (
            id              TEXT PRIMARY KEY,
            user_id         TEXT NOT NULL REFERENCES users(id),
            name            TEXT NOT NULL,
            icon            TEXT NOT NULL DEFAULT 'layout-dashboard',
            sort_order      INTEGER NOT NULL DEFAULT 0,
            layout_json     TEXT NOT NULL,
            is_default      BOOLEAN NOT NULL DEFAULT FALSE,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_workspaces_user ON workspaces(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_workspaces_sort ON workspaces(user_id, sort_order)")
        .execute(&mut *tx)
        .await?;

    // Add active_workspace_id to users table
    sqlx::query("ALTER TABLE users ADD COLUMN active_workspace_id TEXT REFERENCES workspaces(id)")
        .execute(&mut *tx)
        .await
        .ok(); // Ignore if column already exists

    // ── ATTACHMENTS ──
    sqlx::query(
        "CREATE TABLE attachments (
            id              TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            message_id      TEXT,
            filename        TEXT NOT NULL,
            mime_type       TEXT NOT NULL,
            size_bytes      INTEGER NOT NULL,
            path            TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_attachments_session ON attachments(session_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_attachments_message ON attachments(message_id)")
        .execute(&mut *tx)
        .await?;

    // ── PROJECTS ──
    sqlx::query(
        "CREATE TABLE projects (
            id              TEXT PRIMARY KEY,
            user_id         TEXT NOT NULL DEFAULT 'default' REFERENCES users(id),
            slug            TEXT NOT NULL,
            name            TEXT NOT NULL,
            description     TEXT NOT NULL DEFAULT '',
            status          TEXT NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'paused', 'archived')),
            root_path       TEXT,
            site_url        TEXT,
            repo_url        TEXT,
            notes           TEXT NOT NULL DEFAULT '',
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            last_opened_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            UNIQUE(user_id, slug)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX idx_projects_recency ON projects(user_id, status, last_opened_at DESC)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX idx_projects_slug ON projects(user_id, slug)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_projects_name ON projects(user_id, name)")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "CREATE TRIGGER trg_projects_updated_at
            AFTER UPDATE ON projects
            FOR EACH ROW
            BEGIN
                UPDATE projects SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                WHERE id = NEW.id;
            END",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE project_tags (
            project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            tag             TEXT NOT NULL,
            PRIMARY KEY (project_id, tag)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_project_tags_tag ON project_tags(tag)")
        .execute(&mut *tx)
        .await?;

    // Seed the implicit "personal" project
    sqlx::query(
        "INSERT INTO projects (id, user_id, slug, name, description, status)
         VALUES ('00000000-0000-0000-0000-000000000001', 'default', 'personal', 'Personal', 'Default project for unscoped activity.', 'active')",
    )
    .execute(&mut *tx)
    .await?;

    // ── BOARD COLUMNS ──
    sqlx::query(
        "CREATE TABLE board_columns (
            id              TEXT PRIMARY KEY,
            project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            name            TEXT NOT NULL,
            position        INTEGER NOT NULL,
            column_kind     TEXT NOT NULL DEFAULT 'manual'
                            CHECK (column_kind IN ('manual', 'state')),
            state_binding   TEXT,
            wip_limit       INTEGER,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_columns_project ON board_columns(project_id, position)")
        .execute(&mut *tx)
        .await?;

    // ── CARDS ──
    sqlx::query(
        "CREATE TABLE cards (
            id              TEXT PRIMARY KEY,
            project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            card_type       TEXT NOT NULL DEFAULT 'standard'
                            CHECK (card_type IN ('standard', 'goal', 'social_post')),
            title           TEXT NOT NULL,
            description     TEXT NOT NULL DEFAULT '',
            column_id       TEXT NOT NULL REFERENCES board_columns(id),
            position        INTEGER NOT NULL DEFAULT 0,
            created_by      TEXT NOT NULL DEFAULT 'user'
                            CHECK (created_by IN ('user', 'henry', 'hermes', 'codex', 'claude-code', 'librarian')),
            assigned_to     TEXT,
            metadata_json   TEXT NOT NULL DEFAULT '{}',
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            archived_at     TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_cards_project ON cards(project_id, column_id, position)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_cards_type ON cards(project_id, card_type)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_cards_archived ON cards(archived_at) WHERE archived_at IS NULL")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "CREATE TRIGGER trg_cards_updated_at
            AFTER UPDATE ON cards
            FOR EACH ROW
            BEGIN
                UPDATE cards SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                WHERE id = NEW.id;
            END",
    )
    .execute(&mut *tx)
    .await?;

    // Seed default columns for Personal project
    sqlx::query(
        "INSERT INTO board_columns (id, project_id, name, position, column_kind) VALUES
            ('col-personal-backlog', '00000000-0000-0000-0000-000000000001', 'Backlog', 0, 'manual'),
            ('col-personal-doing',   '00000000-0000-0000-0000-000000000001', 'Doing',   1, 'manual'),
            ('col-personal-done',    '00000000-0000-0000-0000-000000000001', 'Done',    2, 'manual')",
    )
    .execute(&mut *tx)
    .await?;

    // ── VIEWS ──
    // (current_memories / current_knowledge removed with the dead memories +
    // knowledge_graph tables — see the MEMORIES / KNOWLEDGE GRAPH note above.)
    sqlx::query(
        "CREATE VIEW recent_tasks AS
        SELECT * FROM tasks WHERE status = 'completed' ORDER BY completed_at DESC LIMIT 100",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE VIEW repetition_candidates AS
        SELECT
            user_id,
            tool_used,
            argument_shape_hash,
            COUNT(*) as occurrence_count,
            MIN(completed_at) as first_seen,
            MAX(completed_at) as last_seen,
            (SELECT t2.description FROM tasks t2
             WHERE t2.user_id = tasks.user_id
               AND t2.tool_used = tasks.tool_used
               AND t2.argument_shape_hash = tasks.argument_shape_hash
               AND t2.status = 'completed'
             ORDER BY t2.completed_at DESC LIMIT 1) as latest_description
        FROM tasks
        WHERE status = 'completed'
          AND completed_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-7 days')
        GROUP BY user_id, tool_used, argument_shape_hash
        HAVING COUNT(*) >= 2",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Decision-inbox tables (decisions, decision_audit, risk_policy) + guard triggers.
    // Idempotent; shared with migrate_v9_to_v10 for existing installs.
    apply_decision_inbox_schema(pool).await?;

    // Recognition instrumentation tables (v11). Idempotent; shared with
    // migrate_v10_to_v11 for existing installs.
    apply_recognition_schema(pool).await?;

    // CRM people table (schema v12). Idempotent; shared with migrate_v11_to_v12.
    apply_people_schema(pool).await?;

    // File-intake inbox table (schema v13). Idempotent; shared with
    // migrate_v12_to_v13.
    apply_inbox_schema(pool).await?;

    // Project association join tables (schema v20): project_people +
    // project_memories. Idempotent; shared with migrate_v19_to_v20.
    apply_project_association_schema(pool).await?;

    // Recognition tool-event feed table (schema v22). Idempotent; shared with
    // migrate_v21_to_v22. Fresh installs get the recognition_verdict /
    // familiarity columns from apply_recognition_schema's CREATE directly.
    apply_recognition_feed_schema(pool).await?;

    // Entity provenance side table (schema v23, people-in-graph v1 #583).
    // Idempotent; shared with migrate_v22_to_v23.
    apply_entity_provenance_schema(pool).await?;

    info!(
        "Spectral schema v{} initialized successfully",
        SPECTRAL_SCHEMA_VERSION
    );
    Ok(())
}

/// Apply the recognition-instrumentation schema (v11): `recognition_events`
/// (one row per recall, persisted unconditionally — the falsifiable AmbientFrame
/// substrate) and `recognition_set_members` (the retrieved set, one row per hit).
///
/// Outcome columns are nullable and filled later by async write-back keyed on
/// `retrieval_id` (task-resolution + decision approve/bounce). Distinct names
/// avoid collision with Spectral's own precursor `retrieval_events` table.
///
/// `recognition_verdict` + `familiarity` (schema v22, spectral-recognition
/// prep) are nullable and stay NULL until the Spectral recognize() path is
/// wired; existing DBs gain them via `migrate_v21_to_v22`'s guarded ALTERs.
///
/// Fully idempotent — every statement uses IF NOT EXISTS — so it is safe on
/// fresh installs and on every boot.
pub async fn apply_recognition_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    // ── RECOGNITION EVENTS ──
    // One row per recall. retrieval_id is minted (UUID) at recall time and is
    // the join key for later outcome write-back. The outcome_* wing is all-NULL
    // until a task resolves or a decision is answered.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS recognition_events (
            retrieval_id        TEXT PRIMARY KEY,
            session_id          TEXT NOT NULL,
            query               TEXT NOT NULL,
            retrieved_at        TEXT NOT NULL,
            rc_persona          TEXT NOT NULL,
            rc_session_id       TEXT,
            rc_focus_wing       TEXT,
            strategy            TEXT NOT NULL,
            outcome_kind        TEXT,
            outcome_polarity    TEXT,
            outcome_source      TEXT,
            outcome_observed_at TEXT,
            cited_memory_ids    TEXT NOT NULL DEFAULT '[]',
            recognition_verdict TEXT,
            familiarity         REAL
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recognition_strategy ON recognition_events(strategy)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recognition_session ON recognition_events(session_id)",
    )
    .execute(&mut *tx)
    .await?;

    // ── RECOGNITION SET MEMBERS ──
    // The whole retrieved set for a recall (outcome scores vs the set, not per
    // memory). Child of recognition_events; relational so the null-baseline
    // recompute (per-memory frequency/co-occurrence) stays cheap on every pass.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS recognition_set_members (
            retrieval_id  TEXT NOT NULL REFERENCES recognition_events(retrieval_id) ON DELETE CASCADE,
            memory_id     TEXT NOT NULL,
            signal_score  REAL,
            rank          INTEGER,
            PRIMARY KEY (retrieval_id, memory_id)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recognition_members_memory ON recognition_set_members(memory_id)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Apply the CRM people schema: a typed person table keyed on an opaque,
/// immutable `entity_uuid` (the persona_id pattern), with `canonical_id` as a
/// mutable UNIQUE lookup column. Carries CRM attributes the Brain graph entity
/// lacks (role/company/email/phone/last_contact/notes).
///
/// Fully idempotent — every statement uses `IF NOT EXISTS` so it is safe to run
/// on every boot and on fresh installs. See [`crate::people`] for the access
/// layer and the opaque-id rationale.
pub async fn apply_people_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS people (
            entity_uuid     TEXT PRIMARY KEY,
            canonical_id    TEXT NOT NULL UNIQUE,
            display_name    TEXT NOT NULL,
            role            TEXT,
            company         TEXT,
            email           TEXT,
            phone           TEXT,
            notes           TEXT,
            last_contact_at TEXT,
            graph_entity_id TEXT,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_company ON people(company)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_graph_entity ON people(graph_entity_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_role ON people(role)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_email ON people(email)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_people_last_contact ON people(last_contact_at DESC)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Apply the entity-provenance schema (v23, people-in-graph v1 #583): a
/// permagent.db side table recording where each graph entity came from
/// (`ontology` | `runtime` | `extracted`), keyed on the bare 64-hex `EntityId`.
///
/// This is what makes runtime person-creation durable: the daemon reconciler
/// (`sync_graph_with_ontology`) prunes only `ontology`-sourced entities, so
/// `runtime`/`extracted` entities survive across restarts. See
/// [`crate::people_provenance`]. Fully idempotent (`CREATE TABLE IF NOT EXISTS`).
pub async fn apply_entity_provenance_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS entity_provenance (
            entity_id_hex TEXT PRIMARY KEY,
            source        TEXT NOT NULL CHECK (source IN ('ontology','runtime','extracted')),
            created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Migrate an existing database to the entity-provenance schema (schema v23).
///
/// Purely additive and base-version independent (`CREATE TABLE IF NOT EXISTS`),
/// so it applies cleanly over any earlier base. Records v23 in `schema_version`.
pub async fn migrate_v22_to_v23(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v22 -> v23 (entity provenance)");

    apply_entity_provenance_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (23)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v23 (entity provenance)");

    Ok(())
}

/// Migrate an existing database to the recognition-instrumentation schema (v11).
///
/// Purely additive (CREATE TABLE / INDEX IF NOT EXISTS), base-version
/// independent. Records v11 in `schema_version` (hardcoded, so it stays correct
/// as SPECTRAL_SCHEMA_VERSION advances; the v11 -> v12 step is applied separately
/// by migrate_v11_to_v12 — mirrors the migrate_v9_to_v10 precedent).
pub async fn migrate_v10_to_v11(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v10 -> v11 (recognition instrumentation)");

    apply_recognition_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (11)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v11 (recognition instrumentation)");

    Ok(())
}

/// Migrate an existing database to the CRM people schema (schema v12).
///
/// Additive and base-version independent (purely `CREATE TABLE IF NOT EXISTS` /
/// `CREATE INDEX IF NOT EXISTS`), so it applies cleanly over either a v10 or a
/// v11 base. Records v12 in `schema_version`.
pub async fn migrate_v11_to_v12(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v11 -> v12 (CRM people)");

    apply_people_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (12)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v12 (CRM people)");

    Ok(())
}

/// Migrate an existing database to the file-intake inbox schema (schema v13).
///
/// Additive and base-version independent (purely `CREATE TABLE IF NOT EXISTS` /
/// `CREATE INDEX IF NOT EXISTS`), so it applies cleanly over any earlier base.
/// Records v13 in `schema_version`.
pub async fn migrate_v12_to_v13(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v12 -> v13 (file-intake inbox)");

    apply_inbox_schema(pool).await?;

    // Hardcoded (v13), matching the migrate_v10_to_v11 / v11_to_v12 precedent,
    // so it stays correct now that SPECTRAL_SCHEMA_VERSION has advanced past 13.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (13)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v13 (file-intake inbox)");

    Ok(())
}

/// Migrate an existing database by removing duplicate board columns (schema
/// v14, #453). A data cleanup — no DDL — so it is base-version independent and
/// idempotent: it deletes only EMPTY generic manual columns in projects that
/// also carry the goal lifecycle (`state`) columns. Records v14.
pub async fn migrate_v13_to_v14(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v13 -> v14 (duplicate-column cleanup, #453)");

    let removed = crate::cards::cleanup_duplicate_manual_columns(pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Hardcoded (v14) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (14)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v14 ({removed} duplicate columns removed)");

    Ok(())
}

pub async fn migrate_v14_to_v15(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v14 -> v15 (consolidate Doing/Done into lifecycle, #453)");

    let removed = crate::cards::consolidate_doing_done_into_lifecycle(pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Hardcoded (v15) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (15)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v15 ({removed} consolidated columns removed)");

    Ok(())
}

/// Backfill the Cancelled goal-lifecycle column (schema v16, #490). Boards
/// seeded before the Cancelled column existed lack the target column that
/// `advance_goal_checked(Cancel)` writes into; this adds it. A data fixup — no
/// DDL — so it is base-version independent and idempotent (only inserts where
/// the lifecycle columns exist but `cancelled` is absent). Records v16.
pub async fn migrate_v15_to_v16(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v15 -> v16 (backfill Cancelled column, #490)");

    let added = crate::cards::backfill_cancelled_column(pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Hardcoded (v16) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (16)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v16 ({added} Cancelled columns added)");

    Ok(())
}

/// Reconcile EVERY board to the canonical goal lifecycle (schema v17, #502).
///
/// v14/v15/v16 only fixed boards that already had lifecycle (`state`) columns —
/// seeded on a board's first goal card. Boards that never held a goal card kept
/// only the default manual Backlog/Doing/Done and were skipped by every prior
/// migration. This seeds the canonical lifecycle columns on all boards then runs
/// the standard Doing→In Progress / Done→Complete consolidation everywhere. A
/// data fixup — no DDL — base-version independent, idempotent, card-data-safe
/// (moves precede deletes). Records v17.
pub async fn migrate_v16_to_v17(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v16 -> v17 (canonical columns on ALL boards, #502)");

    let removed = crate::cards::reconcile_all_boards_to_canonical(pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Hardcoded (v17) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (17)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v17 ({removed} legacy columns removed across all boards)");

    Ok(())
}

/// Reconcile the `risk_policy` seed onto existing DBs (schema v18, #514).
///
/// The decision-inbox seed (`apply_decision_inbox_schema`) uses INSERT OR IGNORE
/// and only runs when a DB first crosses v9→v10. Rows added to that seed list
/// AFTER a user's table was created therefore never land on existing installs —
/// the table already exists, so the seed step is skipped on later boots. The
/// `goal_cancel` row was added in #500, AFTER the v10 table creation (the seed
/// originally shipped 16 rows at v10; `goal_cancel` is the only one added since),
/// so every pre-#500 DB is missing it. An unknown `action_class` fails closed to
/// Tier 2 in the decision engine, so Cancel always 409s on those installs (#514).
/// This is the same class of seed-vs-migration gap as #502/#507.
///
/// Fix: force-set `goal_cancel` to Tier 0 via UPSERT (covers both the absent and
/// the wrong-value case — the bug is the missing/elevated tier), and defensively
/// INSERT-OR-IGNORE the full original seed so a partially-seeded table self-heals
/// without clobbering any deliberate user/Henry tier customization on the other
/// rows. Base-version independent and idempotent. Records v18. SPECTRAL_SCHEMA_VERSION
/// stays 14 — fresh installs get the corrected seed from `apply_decision_inbox_schema`
/// directly; this migration targets only existing DBs.
pub async fn migrate_v17_to_v18(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v17 -> v18 (reconcile risk_policy seed, #514)");

    let mut tx = pool.begin().await?;

    // Defensive: restore any seed row that is absent (never clobbers an existing
    // row, so user/Henry tier customizations survive). All 17 rows were present
    // at v10 except goal_cancel, which this also inserts if missing.
    sqlx::query(
        "INSERT OR IGNORE INTO risk_policy (action_class, tier, rationale) VALUES
            ('goal_ready', 0, 'Triage->Ready promotion is reversible'),
            ('goal_dispatch', 0, 'Dispatching a ready goal to a worker is reversible'),
            ('goal_review', 0, 'Worker reporting completion is informational'),
            ('goal_complete_confined', 0, 'Completion check passed, diff confined to declared paths, reversible class'),
            ('goal_approve_standard', 1, 'Review->Complete requires a recorded decision (Henry or Jesse)'),
            ('goal_retry_within_budget', 1, 'Reject/retry requires a recorded decision with rationale'),
            ('goal_cancel', 0, 'User-initiated cancellation of a goal is immediate; the worker is killed'),
            ('merge_to_main', 2, 'Irreversible publication'),
            ('push_main', 2, 'Irreversible publication'),
            ('schema_migration', 2, 'Data-shape change'),
            ('user_data_deletion', 2, 'Destructive, includes goal-card deletion'),
            ('network_external', 2, 'Side effects outside the machine'),
            ('spend', 2, 'Costs money'),
            ('secrets_access', 2, 'Credential exposure'),
            ('permission_change', 2, 'Expands capability surface'),
            ('orchestrator_edit', 2, 'Self-modification of the control loop'),
            ('policy_edit', 2, 'Changes to this table are themselves Tier 2')",
    )
    .execute(&mut *tx)
    .await?;

    // Force goal_cancel to Tier 0: this is the #514 bug — it must be reversible/
    // immediate. UPSERT (not OR IGNORE) so a DB where it exists at a wrong tier is
    // corrected too, not just one where it is absent.
    sqlx::query(
        "INSERT INTO risk_policy (action_class, tier, rationale)
         VALUES ('goal_cancel', 0, 'User-initiated cancellation of a goal is immediate; the worker is killed')
         ON CONFLICT(action_class) DO UPDATE SET tier = excluded.tier",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Hardcoded (v18) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (18)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v18 (risk_policy goal_cancel reconciled to Tier 0)");

    Ok(())
}

/// Migrate v18 -> v19: drop the dead `memories` and `knowledge_graph` tables.
///
/// permagent.db carried a dormant copy of the Spectral Phase-1 schema. The live
/// Brain (knowledge graph + distilled memories) lives in a SEPARATE file,
/// `~/.permagent/brain/memory.db`, reached via `read_only_brain_conn()` /
/// `SafeBrain` — never these tables. Both `memories` and `knowledge_graph` here
/// held 0 rows and had no readers; the only code touching `memories` was the
/// `permagent memory` CLI subcommand, a dead-end loop removed alongside this
/// migration. `knowledge_graph.source_memory_id REFERENCES memories(id)` made the
/// two a single co-dead unit, so they are dropped together to leave one
/// unambiguous `memories` table in the system (Spectral's), period.
///
/// Idempotent (`DROP ... IF EXISTS`) and base-version independent. Dependent
/// objects are dropped before their base tables, and `knowledge_graph` (the FK
/// referencer) before `memories`. Records v19. SPECTRAL_SCHEMA_VERSION stays 14 —
/// fresh installs never create these tables (removed from `init_spectral_db`), so
/// the `DROP`s are no-ops there; this migration targets only existing DBs.
pub async fn migrate_v18_to_v19(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v18 -> v19 (drop dead memories + knowledge_graph tables)");

    let mut tx = pool.begin().await?;

    // Views and triggers first (they reference the base tables).
    sqlx::query("DROP VIEW IF EXISTS current_memories")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP VIEW IF EXISTS current_knowledge")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS memories_ai")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS memories_ad")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP TRIGGER IF EXISTS memories_au")
        .execute(&mut *tx)
        .await?;

    // FTS virtual tables (drops their shadow tables automatically).
    sqlx::query("DROP TABLE IF EXISTS memories_fts")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS knowledge_graph_fts")
        .execute(&mut *tx)
        .await?;

    // knowledge_graph references memories(id), so drop the referencer first.
    sqlx::query("DROP TABLE IF EXISTS knowledge_graph")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS memories")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Hardcoded (v19) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (19)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v19 (dead memories + knowledge_graph tables dropped)");

    Ok(())
}

/// Apply the file-intake inbox schema: the `inbox_files` table, one metadata row
/// per file that lands in the Permagent inbox (`~/.permagent/inbox/`).
///
/// Idempotent — every statement uses IF NOT EXISTS — so it is safe on fresh
/// installs (via `init_spectral_db`) and as a migration step alike. `disk_path`
/// is stored relative to [`crate::config::paths::Paths::inbox_dir`]. `project_id`
/// is nullable: FK propagation (epic #70) is deferred, so v1 inbox rows are
/// unscoped and a later pass can attribute them to a project.
pub async fn apply_inbox_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS inbox_files (
            id            TEXT PRIMARY KEY,
            filename      TEXT NOT NULL,
            original_url  TEXT,
            content_type  TEXT,
            size_bytes    INTEGER,
            disk_path     TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'received'
                          CHECK (status IN ('received','ingested','routed','deleted')),
            project_id    TEXT REFERENCES projects(id) ON DELETE SET NULL,
            created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_inbox_files_created ON inbox_files(created_at DESC)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_inbox_files_status ON inbox_files(status)")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Apply the project association schema (v20): two additive join tables that
/// scope global entities to a project.
///
/// * `project_people` — many-to-many between `projects` and the live `people`
///   table (a person can belong to many projects and vice versa). Both columns
///   are real FKs into permagent.db with `ON DELETE CASCADE`, so deleting a
///   project or a person reaps the link. `role` is the role *within this
///   project* (nullable), distinct from the person's global CRM `role`.
///
/// * `project_memories` — scopes Brain memories to a project. `memory_id` is the
///   **Spectral `memory.db` id** (the stable `blake3(key)[..8]` id that recall
///   hits and the browse route expose), stored as plain TEXT with **no FK**:
///   Spectral's Brain is a separate database outside SQLite FK reach. It is
///   deliberately NOT joined to the permagent.db `memories` table — that table
///   is dead weight (written only by the CLI; the live `remember_with` path and
///   every read route go to `memory.db` via `read_only_brain_conn`). Reads
///   resolve `memory_id`s against the live Brain and INNER-JOIN, so an id whose
///   memory was deleted in Spectral simply does not render (orphan prune is a
///   deferred follow-up).
///
/// Idempotent — every statement uses IF NOT EXISTS — so it is safe on fresh
/// installs (via `init_spectral_db`) and as a migration step (`migrate_v19_to_v20`)
/// alike.
pub async fn apply_project_association_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_people (
            project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            entity_uuid  TEXT NOT NULL REFERENCES people(entity_uuid) ON DELETE CASCADE,
            role         TEXT,
            added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            PRIMARY KEY (project_id, entity_uuid)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_people_entity ON project_people(entity_uuid)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_memories (
            project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            memory_id    TEXT NOT NULL,
            added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            PRIMARY KEY (project_id, memory_id)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_memories_mem ON project_memories(memory_id)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the project association schema (v20).
///
/// Purely additive (CREATE TABLE / INDEX IF NOT EXISTS), base-version
/// independent and idempotent. Records v20 in `schema_version` (hardcoded per the
/// migration precedent in this file, so it stays correct as SPECTRAL_SCHEMA_VERSION
/// advances). SPECTRAL_SCHEMA_VERSION stays 14 — fresh installs get these tables
/// from `apply_project_association_schema` directly via `init_spectral_db`.
pub async fn migrate_v19_to_v20(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v19 -> v20 (project association join tables)");
    apply_project_association_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (20)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v20 (project_people + project_memories)");
    Ok(())
}

/// Migrate an existing database to the people↔graph bridge schema (v21, #255/B).
///
/// Adds the immutable `graph_entity_id` column to `people` — the bridge key that
/// carries an identity-only person row to its attributes in the Spectral graph
/// (`entity_fields`). The value is the bare 64-hex blake3 `EntityId`, NOT derived
/// from the mutable `canonical_id` (which would mis-join — see
/// `identity::canonical::graph_canonical`).
///
/// Idempotent + base-independent: SQLite has no `ADD COLUMN IF NOT EXISTS`, so the
/// add is guarded on `PRAGMA table_info`. Fresh installs already get the column
/// from `apply_people_schema` via `init_spectral_db`, so this guard lets the step
/// run harmlessly on fresh and existing DBs alike. SPECTRAL_SCHEMA_VERSION stays 14.
pub async fn migrate_v20_to_v21(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v20 -> v21 (people.graph_entity_id bridge column)");

    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('people') WHERE name = 'graph_entity_id'",
    )
    .fetch_one(pool)
    .await?;

    if has_column == 0 {
        sqlx::query("ALTER TABLE people ADD COLUMN graph_entity_id TEXT")
            .execute(pool)
            .await?;
    }

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_graph_entity ON people(graph_entity_id)")
        .execute(pool)
        .await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (21)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v21 (people.graph_entity_id)");
    Ok(())
}

/// Apply the recognition tool-event feed schema (v22, spectral-recognition
/// prep): `recognition_tool_events`, a timestamped, content-free stream of
/// tool calls (tool name, wing, coarse args-class). A sequential event stream
/// is the input Spectral's path-pursuit tracker needs for step-level routine
/// recognition; writes are feature-gated (`spectral-recognition`) and
/// fire-and-forget from the TaskLogger choke point.
///
/// Fully idempotent — every statement uses IF NOT EXISTS — safe on fresh
/// installs (via `init_spectral_db`) and as a migration step alike.
pub async fn apply_recognition_feed_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS recognition_tool_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            occurred_at TEXT NOT NULL,
            tool_name   TEXT NOT NULL,
            wing        TEXT,
            args_class  TEXT,
            session_id  TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_tool_events_time ON recognition_tool_events(occurred_at)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_tool_events_session ON recognition_tool_events(session_id, occurred_at)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate v21 -> v22 (spectral-recognition prep, two additive pieces):
///
/// 1. nullable `recognition_verdict TEXT` + `familiarity REAL` on
///    `recognition_events` — verdicts logged NEXT TO outcomes make the table
///    the validation ground truth for Spectral's recognize(); both stay NULL
///    until that path is wired. SQLite has no `ADD COLUMN IF NOT EXISTS`, so
///    the adds are guarded on `PRAGMA table_info` (fresh installs already get
///    the columns from `apply_recognition_schema`'s CREATE).
/// 2. the `recognition_tool_events` feed table (see
///    [`apply_recognition_feed_schema`]).
///
/// Idempotent + base-version independent. Records v22 (hardcoded per the
/// migration precedent in this file); SPECTRAL_SCHEMA_VERSION stays 14. The
/// session_manager gate for this step is `#[cfg(feature =
/// "spectral-recognition")]` — a feature-off build leaves the DB untouched at
/// v21 (constraint: zero behavior change when the flag is off), and the
/// guarded adds make it safe whichever build order a DB sees.
pub async fn migrate_v21_to_v22(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v21 -> v22 (recognition verdict columns + tool-event feed)");

    apply_recognition_v22_columns(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (22)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v22 (recognition_verdict + familiarity + recognition_tool_events)");
    Ok(())
}

/// Ensure the v22 recognition columns + feed table exist, **independent of the
/// global schema version**. Idempotent (PRAGMA-guarded `ADD COLUMN` +
/// `CREATE TABLE IF NOT EXISTS`) and safe to run on every boot.
///
/// This exists to close the cfg-gated-migration-skip hazard: the v22 step is
/// gated behind `#[cfg(feature = "spectral-recognition")]`, but a later
/// always-on migration (v23) can stamp `schema_version` past 22 while the
/// feature is OFF. A version-gated `if version < 22` would then never run when
/// the feature is later turned ON, so the columns would be silently missing and
/// the recognition path would break on activation. Applying by
/// column-existence — not by version — makes activation safe from any stamped
/// version. Observable: logs each column/table it actually adds (a steady-state
/// boot is silent).
pub async fn apply_recognition_v22_columns(pool: &Pool<Sqlite>) -> Result<()> {
    for (column, ddl) in [
        (
            "recognition_verdict",
            "ALTER TABLE recognition_events ADD COLUMN recognition_verdict TEXT",
        ),
        (
            "familiarity",
            "ALTER TABLE recognition_events ADD COLUMN familiarity REAL",
        ),
    ] {
        let has_column: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('recognition_events') WHERE name = ?",
        )
        .bind(column)
        .fetch_one(pool)
        .await?;
        if has_column == 0 {
            sqlx::query(ddl).execute(pool).await?;
            info!(
                "recognition schema repair: added missing column '{column}' to recognition_events \
                 (version-independent — cfg-gated v22 was skipped by a later version stamp)"
            );
        }
    }

    apply_recognition_feed_schema(pool).await?;
    Ok(())
}

/// Apply the decision-inbox schema: decisions, decision_audit (append-only,
/// hash-chained), risk_policy (trust dial), and defense-in-depth triggers.
///
/// Fully idempotent — every statement uses IF NOT EXISTS / INSERT OR IGNORE so
/// it is safe to run on every boot (sentinel mode) and on fresh installs.
pub async fn apply_decision_inbox_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    // ── DECISIONS ──
    // headline/detail are Jesse amendment A1: two separate REQUIRED text fields.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decisions (
            id            TEXT PRIMARY KEY,
            kind          TEXT NOT NULL CHECK (kind IN
                            ('approve_review','unblock','choice','risk_gate','automation_proposal','malformed')),
            goal_id       TEXT REFERENCES cards(id) ON DELETE SET NULL,
            project_id    TEXT REFERENCES projects(id) ON DELETE CASCADE,
            tier          INTEGER NOT NULL CHECK (tier IN (0,1,2)),
            headline      TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
            detail        TEXT NOT NULL CHECK (length(detail) > 0),
            payload_json  TEXT NOT NULL DEFAULT '{}',
            rank          REAL,
            status        TEXT NOT NULL DEFAULT 'open'
                          CHECK (status IN ('open','answered','expired','superseded')),
            answer        TEXT CHECK (answer IN ('approve','reject','choice','input')),
            answer_note   TEXT,
            answer_choice_id TEXT,
            answer_input  TEXT,
            acted_by      TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
            created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            resolved_at   TEXT,
            CHECK (status != 'answered'
                   OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
        )",
    )
    .execute(&mut *tx)
    .await?;

    // Defensive backfill: CREATE TABLE IF NOT EXISTS won't add columns to a
    // table created by an earlier iteration of this (unreleased) schema.
    let decision_cols: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('decisions')")
            .fetch_all(&mut *tx)
            .await?;
    for (col, ddl) in [
        (
            "answer_choice_id",
            "ALTER TABLE decisions ADD COLUMN answer_choice_id TEXT",
        ),
        (
            "answer_input",
            "ALTER TABLE decisions ADD COLUMN answer_input TEXT",
        ),
    ] {
        if !decision_cols.iter().any(|c| c == col) {
            sqlx::query(ddl).execute(&mut *tx).await?;
        }
    }

    // Widen the `kind` CHECK to admit 'automation_proposal' (Initiative → Decision
    // Inbox). SQLite cannot ALTER a CHECK, so an older table is rebuilt in place.
    // FK-safe: nothing references `decisions` via a foreign key (decision_audit
    // stores a plain TEXT id; the complete-guard trigger resolves by name after
    // the rename). Gated on the constraint text, so it runs at most once.
    let decisions_ddl: Option<String> =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'")
            .fetch_optional(&mut *tx)
            .await?;
    if decisions_ddl
        .map(|sql| !sql.contains("automation_proposal"))
        .unwrap_or(false)
    {
        info!("Widening decisions.kind CHECK for 'automation_proposal' (in-place rebuild)");
        // Indexes on the old table are dropped with it; recreated below.
        sqlx::query("DROP INDEX IF EXISTS idx_decisions_open")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DROP INDEX IF EXISTS idx_decisions_goal")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE TABLE decisions_new (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL CHECK (kind IN
                                ('approve_review','unblock','choice','risk_gate','automation_proposal','malformed')),
                goal_id       TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id    TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier          INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline      TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail        TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json  TEXT NOT NULL DEFAULT '{}',
                rank          REAL,
                status        TEXT NOT NULL DEFAULT 'open'
                              CHECK (status IN ('open','answered','expired','superseded')),
                answer        TEXT CHECK (answer IN ('approve','reject','choice','input')),
                answer_note   TEXT,
                answer_choice_id TEXT,
                answer_input  TEXT,
                acted_by      TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
                created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at   TEXT,
                CHECK (status != 'answered'
                       OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
            )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO decisions_new (id, kind, goal_id, project_id, tier, headline, detail,
                 payload_json, rank, status, answer, answer_note, answer_choice_id, answer_input,
                 acted_by, created_at, resolved_at)
             SELECT id, kind, goal_id, project_id, tier, headline, detail,
                 payload_json, rank, status, answer, answer_note, answer_choice_id, answer_input,
                 acted_by, created_at, resolved_at
             FROM decisions",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("DROP TABLE decisions")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE decisions_new RENAME TO decisions")
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_decisions_open
         ON decisions(status, rank DESC, created_at) WHERE status = 'open'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_decisions_goal ON decisions(goal_id)")
        .execute(&mut *tx)
        .await?;

    // ── DECISION AUDIT (append-only, hash chain) ──
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decision_audit (
            seq             INTEGER PRIMARY KEY AUTOINCREMENT,
            decision_id     TEXT NOT NULL,
            goal_id         TEXT,
            acted_by        TEXT NOT NULL,
            tier            INTEGER NOT NULL,
            outcome         TEXT NOT NULL,
            evidence_digest TEXT,
            prev_hash       TEXT,
            row_hash        TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_decision_audit_no_update BEFORE UPDATE ON decision_audit
         BEGIN SELECT RAISE(ABORT, 'decision_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_decision_audit_no_delete BEFORE DELETE ON decision_audit
         BEGIN SELECT RAISE(ABORT, 'decision_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;

    // ── RISK POLICY (trust dial) ──
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS risk_policy (
            action_class TEXT PRIMARY KEY,
            tier         INTEGER NOT NULL CHECK (tier IN (0,1,2)),
            rationale    TEXT,
            updated_by   TEXT NOT NULL DEFAULT 'system',
            updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    // Seeds. Unknown action_class resolves to Tier 2 (fail-closed) in code.
    sqlx::query(
        "INSERT OR IGNORE INTO risk_policy (action_class, tier, rationale) VALUES
            ('goal_ready', 0, 'Triage->Ready promotion is reversible'),
            ('goal_dispatch', 0, 'Dispatching a ready goal to a worker is reversible'),
            ('goal_review', 0, 'Worker reporting completion is informational'),
            ('goal_complete_confined', 0, 'Completion check passed, diff confined to declared paths, reversible class'),
            ('goal_approve_standard', 1, 'Review->Complete requires a recorded decision (Henry or Jesse)'),
            ('goal_retry_within_budget', 1, 'Reject/retry requires a recorded decision with rationale'),
            ('goal_cancel', 0, 'User-initiated cancellation of a goal is immediate; the worker is killed'),
            ('merge_to_main', 2, 'Irreversible publication'),
            ('push_main', 2, 'Irreversible publication'),
            ('schema_migration', 2, 'Data-shape change'),
            ('user_data_deletion', 2, 'Destructive, includes goal-card deletion'),
            ('network_external', 2, 'Side effects outside the machine'),
            ('spend', 2, 'Costs money'),
            ('secrets_access', 2, 'Credential exposure'),
            ('permission_change', 2, 'Expands capability surface'),
            ('orchestrator_edit', 2, 'Self-modification of the control loop'),
            ('policy_edit', 2, 'Changes to this table are themselves Tier 2')",
    )
    .execute(&mut *tx)
    .await?;

    // ── Defense-in-depth: block raw goal moves into complete-bound columns ──
    // Fires for ANY connection (including raw sqlite3) absent a matching
    // answered approve decision for the goal.
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_goal_complete_guard
         BEFORE UPDATE OF column_id ON cards
         FOR EACH ROW
         WHEN OLD.card_type = 'goal'
           AND NEW.column_id != OLD.column_id
           AND (SELECT state_binding FROM board_columns WHERE id = NEW.column_id) = 'complete'
           AND NOT EXISTS (
               SELECT 1 FROM decisions d
               WHERE d.goal_id = OLD.id
                 AND d.status = 'answered'
                 AND d.answer = 'approve'
           )
         BEGIN
             SELECT RAISE(ABORT, 'goal cannot enter complete without an answered approve decision');
         END",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the decision-inbox schema (schema v10).
///
/// Follows the idempotent migrate_v7_to_v8 template. The body is base-version
/// independent (purely additive CREATE TABLE IF NOT EXISTS / INSERT OR IGNORE),
/// so it correctly covers the v8 -> v10 step on this branch while v9
/// (session-list-perf, committed but unmerged) is absent, and would apply
/// equally over a v9 base once that lands. Records v10 in `schema_version`.
pub async fn migrate_v9_to_v10(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v9 -> v10 (decision inbox)");

    apply_decision_inbox_schema(pool).await?;

    // This migration lands the decision-inbox schema only (v10). The version is
    // hardcoded so it stays correct as SPECTRAL_SCHEMA_VERSION advances; the
    // v10 -> v11 step is applied separately by migrate_v10_to_v11.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (10)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v10 (decision inbox)");

    Ok(())
}

/// Verify the schema version of an existing Spectral database.
/// Returns Ok(version) if valid, logs a warning on mismatch.
pub async fn verify_schema_version(pool: &Pool<Sqlite>) -> Result<i32> {
    let version = sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM schema_version")
        .fetch_one(pool)
        .await?;

    if version != SPECTRAL_SCHEMA_VERSION {
        warn!(
            "Spectral schema version mismatch: found v{}, expected v{}",
            version, SPECTRAL_SCHEMA_VERSION
        );
    }

    Ok(version)
}

/// Migrate from schema v2 to v3: add tool_used column to skill_dismissals
/// and replace the index with one that includes dismissed_at.
pub async fn migrate_v2_to_v3(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v2 → v3");

    // Add tool_used column (SQLite ALTER TABLE ADD COLUMN)
    let has_col = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM pragma_table_info('skill_dismissals') WHERE name = 'tool_used'",
    )
    .fetch_one(pool)
    .await?;

    if has_col == 0 {
        sqlx::query("ALTER TABLE skill_dismissals ADD COLUMN tool_used TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await?;
    }

    // Replace index
    sqlx::query("DROP INDEX IF EXISTS idx_skill_dismissals_user_shape")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_skill_dismissals_lookup ON skill_dismissals(user_id, argument_shape_hash, dismissed_at)",
    )
    .execute(pool)
    .await?;

    // Record version
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (3)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v3");
    Ok(())
}

/// Migrate from schema v3 to v4: add workspaces table and active_workspace_id to users.
pub async fn migrate_v3_to_v4(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v3 -> v4");

    // Check if workspaces table already exists
    let has_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='workspaces')",
    )
    .fetch_one(pool)
    .await?;

    if !has_table {
        sqlx::query(
            "CREATE TABLE workspaces (
                id              TEXT PRIMARY KEY,
                user_id         TEXT NOT NULL REFERENCES users(id),
                name            TEXT NOT NULL,
                icon            TEXT NOT NULL DEFAULT 'layout-dashboard',
                sort_order      INTEGER NOT NULL DEFAULT 0,
                layout_json     TEXT NOT NULL,
                is_default      BOOLEAN NOT NULL DEFAULT FALSE,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_workspaces_user ON workspaces(user_id)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_workspaces_sort ON workspaces(user_id, sort_order)")
            .execute(pool)
            .await?;
    }

    // Add active_workspace_id to users
    let has_col = sqlx::query_scalar::<_, i32>(
        "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name = 'active_workspace_id'",
    )
    .fetch_one(pool)
    .await?;

    if has_col == 0 {
        sqlx::query(
            "ALTER TABLE users ADD COLUMN active_workspace_id TEXT REFERENCES workspaces(id)",
        )
        .execute(pool)
        .await?;
    }

    // Record version
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (4)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v4");
    Ok(())
}

/// Migrate from schema v4 to v5: add Brain as fourth workspace preset.
/// Only appends if the user already has the three default workspaces.
pub async fn migrate_v4_to_v5(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v4 -> v5");

    // Check if user already has a Brain workspace
    let has_brain = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM workspaces WHERE user_id = 'default' AND name = 'Brain')",
    )
    .fetch_one(pool)
    .await?;

    if !has_brain {
        let brain_id = uuid::Uuid::now_v7().to_string();
        let layout = serde_json::json!({"type": "panel", "tool": "memory", "config": {}});
        let layout_str = serde_json::to_string(&layout)?;

        sqlx::query(
            "INSERT INTO workspaces (id, user_id, name, icon, sort_order, layout_json, is_default)
             VALUES (?, 'default', 'Brain', 'brain', 3, ?, 0)",
        )
        .bind(&brain_id)
        .bind(&layout_str)
        .execute(pool)
        .await?;

        info!("Added Brain workspace preset");
    }

    // Record version
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (5)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v5");
    Ok(())
}

/// Migrate from schema v5 to v6: add attachments table for file uploads.
pub async fn migrate_v5_to_v6(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v5 -> v6");

    let has_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='attachments')",
    )
    .fetch_one(pool)
    .await?;

    if !has_table {
        sqlx::query(
            "CREATE TABLE attachments (
                id              TEXT PRIMARY KEY,
                session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                message_id      TEXT,
                filename        TEXT NOT NULL,
                mime_type       TEXT NOT NULL,
                size_bytes      INTEGER NOT NULL,
                path            TEXT NOT NULL,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_attachments_session ON attachments(session_id)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_attachments_message ON attachments(message_id)")
            .execute(pool)
            .await?;
    }

    // Record version
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (6)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v6");
    Ok(())
}

/// Migrate from schema v6 to v7: add projects and project_tags tables.
pub async fn migrate_v6_to_v7(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v6 -> v7");

    let has_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='projects')",
    )
    .fetch_one(pool)
    .await?;

    if !has_table {
        sqlx::query(
            "CREATE TABLE projects (
                id              TEXT PRIMARY KEY,
                user_id         TEXT NOT NULL DEFAULT 'default' REFERENCES users(id),
                slug            TEXT NOT NULL,
                name            TEXT NOT NULL,
                description     TEXT NOT NULL DEFAULT '',
                status          TEXT NOT NULL DEFAULT 'active'
                                CHECK (status IN ('active', 'paused', 'archived')),
                root_path       TEXT,
                site_url        TEXT,
                repo_url        TEXT,
                notes           TEXT NOT NULL DEFAULT '',
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                last_opened_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                UNIQUE(user_id, slug)
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX idx_projects_recency ON projects(user_id, status, last_opened_at DESC)",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX idx_projects_slug ON projects(user_id, slug)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_projects_name ON projects(user_id, name)")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE TRIGGER trg_projects_updated_at
                AFTER UPDATE ON projects
                FOR EACH ROW
                BEGIN
                    UPDATE projects SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                    WHERE id = NEW.id;
                END",
        )
        .execute(pool)
        .await?;
    }

    let has_tags_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='project_tags')",
    )
    .fetch_one(pool)
    .await?;

    if !has_tags_table {
        sqlx::query(
            "CREATE TABLE project_tags (
                project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                tag             TEXT NOT NULL,
                PRIMARY KEY (project_id, tag)
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_project_tags_tag ON project_tags(tag)")
            .execute(pool)
            .await?;
    }

    // Seed the implicit "personal" project if it doesn't exist
    let has_personal = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM projects WHERE id = '00000000-0000-0000-0000-000000000001')",
    )
    .fetch_one(pool)
    .await?;

    if !has_personal {
        sqlx::query(
            "INSERT INTO projects (id, user_id, slug, name, description, status)
             VALUES ('00000000-0000-0000-0000-000000000001', 'default', 'personal', 'Personal', 'Default project for unscoped activity.', 'active')",
        )
        .execute(pool)
        .await?;
    }

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (7)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v7");
    Ok(())
}

/// Migrate from schema v7 to v8: add board_columns and cards tables.
pub async fn migrate_v7_to_v8(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v7 -> v8");

    let has_columns_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='board_columns')",
    )
    .fetch_one(pool)
    .await?;

    if !has_columns_table {
        sqlx::query(
            "CREATE TABLE board_columns (
                id              TEXT PRIMARY KEY,
                project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                name            TEXT NOT NULL,
                position        INTEGER NOT NULL,
                column_kind     TEXT NOT NULL DEFAULT 'manual'
                                CHECK (column_kind IN ('manual', 'state')),
                state_binding   TEXT,
                wip_limit       INTEGER,
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_columns_project ON board_columns(project_id, position)")
            .execute(pool)
            .await?;
    }

    let has_cards_table = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='cards')",
    )
    .fetch_one(pool)
    .await?;

    if !has_cards_table {
        sqlx::query(
            "CREATE TABLE cards (
                id              TEXT PRIMARY KEY,
                project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                card_type       TEXT NOT NULL DEFAULT 'standard'
                                CHECK (card_type IN ('standard', 'goal', 'social_post')),
                title           TEXT NOT NULL,
                description     TEXT NOT NULL DEFAULT '',
                column_id       TEXT NOT NULL REFERENCES board_columns(id),
                position        INTEGER NOT NULL DEFAULT 0,
                created_by      TEXT NOT NULL DEFAULT 'user'
                                CHECK (created_by IN ('user', 'henry', 'hermes', 'codex', 'claude-code', 'librarian')),
                assigned_to     TEXT,
                metadata_json   TEXT NOT NULL DEFAULT '{}',
                created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                archived_at     TEXT
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX idx_cards_project ON cards(project_id, column_id, position)")
            .execute(pool)
            .await?;
        sqlx::query("CREATE INDEX idx_cards_type ON cards(project_id, card_type)")
            .execute(pool)
            .await?;
        sqlx::query(
            "CREATE INDEX idx_cards_archived ON cards(archived_at) WHERE archived_at IS NULL",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER trg_cards_updated_at
                AFTER UPDATE ON cards
                FOR EACH ROW
                BEGIN
                    UPDATE cards SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                    WHERE id = NEW.id;
                END",
        )
        .execute(pool)
        .await?;
    }

    // Seed default columns for Personal project if not present
    let has_personal_cols: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM board_columns WHERE project_id = '00000000-0000-0000-0000-000000000001')",
    )
    .fetch_one(pool)
    .await?;

    if !has_personal_cols {
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind) VALUES
                ('col-personal-backlog', '00000000-0000-0000-0000-000000000001', 'Backlog', 0, 'manual'),
                ('col-personal-doing',   '00000000-0000-0000-0000-000000000001', 'Doing',   1, 'manual'),
                ('col-personal-done',    '00000000-0000-0000-0000-000000000001', 'Done',    2, 'manual')",
        )
        .execute(pool)
        .await?;
    }

    // Seed default columns for any existing projects that don't have columns yet
    let projects_without_cols: Vec<String> = sqlx::query_scalar(
        "SELECT p.id FROM projects p
         LEFT JOIN board_columns bc ON bc.project_id = p.id
         WHERE bc.id IS NULL",
    )
    .fetch_all(pool)
    .await?;

    for project_id in &projects_without_cols {
        crate::cards::seed_default_columns(pool, project_id)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
    }

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (8)")
        .execute(pool)
        .await?;

    info!("Spectral schema migrated to v8");
    Ok(())
}

/// Check whether the Spectral schema has already been initialized.
pub async fn is_schema_initialized(pool: &Pool<Sqlite>) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='users')",
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

#[cfg(test)]
mod recognition_schema_tests {
    use super::*;

    async fn mem_pool() -> Pool<Sqlite> {
        sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    async fn table_exists(pool: &Pool<Sqlite>, name: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn fresh_init_lands_v11_with_recognition_tables() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Fresh init lands at the current schema version (now v12, post CRM
        // merge). The recognition tables must still be present regardless of the
        // version bump.
        assert_eq!(
            verify_schema_version(&pool).await.unwrap(),
            SPECTRAL_SCHEMA_VERSION
        );
        assert!(table_exists(&pool, "recognition_events").await);
        assert!(table_exists(&pool, "recognition_set_members").await);
    }

    /// The cfg-gated-migration-skip guarantee: the v22 recognition columns are
    /// applied by column-existence, independent of the global version stamp — so a
    /// DB stamped past 22 (by the always-on v23) with the columns never applied
    /// still gets them repaired. This is the exact case that would silently break
    /// on `spectral-recognition` activation without the fix.
    #[tokio::test]
    async fn recognition_v22_columns_applied_independent_of_version() {
        let pool = mem_pool().await;
        // Simulate a DB that never ran v22: a bare recognition_events (no verdict
        // columns) and schema_version already stamped at 23 (as the always-on v23
        // migration does on a feature-off DB).
        sqlx::query("CREATE TABLE schema_version (version INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (23)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE recognition_events (retrieval_id TEXT PRIMARY KEY, query TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        let cols = |pool: Pool<Sqlite>| async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pragma_table_info('recognition_events') \
                 WHERE name IN ('recognition_verdict','familiarity')",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };

        assert_eq!(cols(pool.clone()).await, 0, "precondition: columns absent");

        // Version-independent repair: applies despite the version being past 22.
        apply_recognition_v22_columns(&pool).await.unwrap();

        assert_eq!(
            cols(pool.clone()).await,
            2,
            "v22 columns applied even though schema_version is stamped at 23"
        );
        assert!(
            table_exists(&pool, "recognition_tool_events").await,
            "feed table also ensured"
        );

        // Idempotent: a second boot adds nothing.
        apply_recognition_v22_columns(&pool).await.unwrap();
        assert_eq!(cols(pool.clone()).await, 2, "idempotent on re-run");
    }

    #[tokio::test]
    async fn migrate_v10_to_v11_creates_tables_and_stamps_version() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Simulate a pre-v11 database: drop the recognition tables and roll the
        // recorded version back to 10.
        sqlx::query("DROP TABLE recognition_set_members")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE recognition_events")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM schema_version")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (10)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v10_to_v11(&pool).await.unwrap();

        assert_eq!(verify_schema_version(&pool).await.unwrap(), 11);
        assert!(table_exists(&pool, "recognition_events").await);
        assert!(table_exists(&pool, "recognition_set_members").await);

        // Idempotent: a second run is a clean no-op.
        migrate_v10_to_v11(&pool).await.unwrap();
        assert_eq!(verify_schema_version(&pool).await.unwrap(), 11);
    }
}

#[cfg(test)]
mod people_schema_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> Pool<Sqlite> {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    async fn people_table_exists(pool: &Pool<Sqlite>) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='people')",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn current_version(pool: &Pool<Sqlite>) -> i32 {
        sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM schema_version")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn fresh_install_has_people_at_v12() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        assert!(people_table_exists(&pool).await);
        assert_eq!(current_version(&pool).await, SPECTRAL_SCHEMA_VERSION);
    }

    /// migrate_v11_to_v12 is base-independent: it must add the people table and
    /// stamp v12 whether the existing DB reports v10 (a recognition-less base) or
    /// v11.
    #[tokio::test]
    async fn migration_is_base_independent() {
        for base in [10, 11] {
            let pool = mem_pool().await;
            // Minimal pre-v12 DB: only the version ledger seeded at `base`.
            sqlx::query(
                "CREATE TABLE schema_version (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                .bind(base)
                .execute(&pool)
                .await
                .unwrap();

            assert!(!people_table_exists(&pool).await);

            migrate_v11_to_v12(&pool).await.unwrap();

            assert!(people_table_exists(&pool).await, "base v{base}: table");
            assert_eq!(current_version(&pool).await, 12, "base v{base}: version");

            // Idempotent: a second run is a no-op, not an error.
            migrate_v11_to_v12(&pool).await.unwrap();
        }
    }

    /// migrate_v20_to_v21 adds graph_entity_id to an old (column-less) people
    /// table and is idempotent — safe to re-run over a DB that already has the
    /// column (the fresh-install case, where apply_people_schema added it).
    #[tokio::test]
    async fn v21_adds_graph_entity_id_and_is_idempotent() {
        async fn has_graph_col(pool: &Pool<Sqlite>) -> i64 {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('people') WHERE name = 'graph_entity_id'",
            )
            .fetch_one(pool)
            .await
            .unwrap()
        }

        // Old v20 DB: a people table WITHOUT graph_entity_id.
        let pool = mem_pool().await;
        sqlx::query(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY,
                 applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO schema_version (version) VALUES (20)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE people (entity_uuid TEXT PRIMARY KEY,
                 canonical_id TEXT NOT NULL UNIQUE, display_name TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(has_graph_col(&pool).await, 0);

        migrate_v20_to_v21(&pool).await.unwrap();
        assert_eq!(has_graph_col(&pool).await, 1);
        assert_eq!(current_version(&pool).await, 21);

        // Idempotent: re-run is a clean no-op (guard prevents a duplicate column).
        migrate_v20_to_v21(&pool).await.unwrap();
        assert_eq!(has_graph_col(&pool).await, 1);

        // Over a fresh-init DB (column already present via apply_people_schema):
        // also a clean no-op.
        let fresh = mem_pool().await;
        init_spectral_db(&fresh).await.unwrap();
        assert_eq!(has_graph_col(&fresh).await, 1);
        migrate_v20_to_v21(&fresh).await.unwrap();
        assert_eq!(has_graph_col(&fresh).await, 1);
    }
}

#[cfg(test)]
mod inbox_schema_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> Pool<Sqlite> {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    async fn current_version(pool: &Pool<Sqlite>) -> i32 {
        sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM schema_version")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn inbox_columns(pool: &Pool<Sqlite>) -> Vec<String> {
        sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info('inbox_files')")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn fresh_install_has_inbox_at_v13() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, SPECTRAL_SCHEMA_VERSION);
        assert_eq!(SPECTRAL_SCHEMA_VERSION, 14);

        let cols = inbox_columns(&pool).await;
        for expected in [
            "id",
            "filename",
            "original_url",
            "content_type",
            "size_bytes",
            "disk_path",
            "status",
            "project_id",
            "created_at",
        ] {
            assert!(
                cols.iter().any(|c| c == expected),
                "inbox_files missing column {expected}; got {cols:?}"
            );
        }
    }

    /// migrate_v12_to_v13 is base-independent: it must add inbox_files and stamp
    /// v13 over any earlier recorded base.
    #[tokio::test]
    async fn migration_is_base_independent() {
        for base in [10, 11, 12] {
            let pool = mem_pool().await;
            sqlx::query(
                "CREATE TABLE schema_version (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                .bind(base)
                .execute(&pool)
                .await
                .unwrap();

            assert!(inbox_columns(&pool).await.is_empty(), "base v{base}: pre");

            migrate_v12_to_v13(&pool).await.unwrap();

            assert!(
                !inbox_columns(&pool).await.is_empty(),
                "base v{base}: table"
            );
            assert_eq!(current_version(&pool).await, 13, "base v{base}: version");

            // Idempotent: a second run is a no-op, not an error.
            migrate_v12_to_v13(&pool).await.unwrap();
        }
    }

    /// migrate_v13_to_v14 (#453) removes empty duplicate manual columns from
    /// goal boards, stamps v14, and is idempotent.
    #[tokio::test]
    async fn migrate_v13_to_v14_dedupes_columns_and_stamps() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Give the personal project both the manual seed columns (from init) and
        // the goal lifecycle columns, then force the recorded version back to 13.
        crate::cards::seed_goal_columns(&pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        // Re-add a stray empty manual column to prove the migration removes it.
        sqlx::query(
            "INSERT INTO board_columns (id, project_id, name, position, column_kind)
             VALUES ('stray-manual', ?, 'Doing', 1, 'manual')",
        )
        .bind(crate::projects::PERSONAL_PROJECT_ID)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (13)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v13_to_v14(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 14);
        let stray: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM board_columns WHERE id = 'stray-manual'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stray, 0, "empty duplicate manual column must be removed");

        // Idempotent: a second run is a no-op, not an error.
        migrate_v13_to_v14(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 14);
    }

    /// migrate_v15_to_v16 (#490) backfills the Cancelled lifecycle column on a
    /// pre-existing goal board, stamps v16, and is idempotent.
    #[tokio::test]
    async fn migrate_v15_to_v16_backfills_cancelled_and_stamps() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Seed the lifecycle columns, then delete the cancelled one to simulate a
        // board seeded before #490, and rewind the recorded version to 15.
        crate::cards::seed_goal_columns(&pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        sqlx::query("DELETE FROM board_columns WHERE state_binding = 'cancelled'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (15)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v15_to_v16(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 16);
        let cancelled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_columns WHERE state_binding = 'cancelled'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cancelled, 1, "cancelled column backfilled exactly once");

        // Idempotent: a second run adds nothing and does not error.
        migrate_v15_to_v16(&pool).await.unwrap();
        let cancelled_again: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_columns WHERE state_binding = 'cancelled'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cancelled_again, 1, "no duplicate cancelled column");
    }

    /// migrate_v16_to_v17 (#502): a board that NEVER held a goal card has only
    /// the legacy manual Backlog/Doing/Done columns and was skipped by every
    /// prior fixup. After v17 it must carry the full canonical lifecycle, with
    /// Doing→In Progress and Done→Complete cards preserved/moved and the emptied
    /// legacy columns dropped. Backlog (and its card) is kept.
    #[tokio::test]
    async fn migrate_v16_to_v17_applies_canonical_to_legacy_board() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // A legacy board: only manual Backlog/Doing/Done, no lifecycle columns.
        let project = "legacy-board-502";
        sqlx::query(
            "INSERT INTO projects (id, user_id, slug, name, status)
             VALUES (?, 'default', 'legacy', 'Legacy', 'active')",
        )
        .bind(project)
        .execute(&pool)
        .await
        .unwrap();
        for (name, pos) in [("Backlog", 0), ("Doing", 1), ("Done", 2)] {
            sqlx::query(
                "INSERT INTO board_columns (id, project_id, name, position, column_kind)
                 VALUES (?, ?, ?, ?, 'manual')",
            )
            .bind(format!("{project}-col-{name}"))
            .bind(project)
            .bind(name)
            .bind(pos)
            .execute(&pool)
            .await
            .unwrap();
        }
        // One card in each legacy column.
        for (name, card) in [
            ("Backlog", "card-backlog"),
            ("Doing", "card-doing"),
            ("Done", "card-done"),
        ] {
            // 'standard' cards: the migration moves cards by column regardless of
            // type, and standard cards sidestep the goal-lifecycle approval
            // trigger that guards entry into `complete` (not under test here).
            sqlx::query(
                "INSERT INTO cards (id, project_id, card_type, title, column_id, position)
                 VALUES (?, ?, 'standard', ?, ?, 0)",
            )
            .bind(card)
            .bind(project)
            .bind(name)
            .bind(format!("{project}-col-{name}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (16)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v16_to_v17(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 17);

        // Full canonical lifecycle now present on the legacy board.
        let states: Vec<String> = sqlx::query_scalar(
            "SELECT state_binding FROM board_columns
             WHERE project_id = ? AND column_kind = 'state' AND state_binding IS NOT NULL
             ORDER BY position",
        )
        .bind(project)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            states,
            vec![
                "triage",
                "ready",
                "in_progress",
                "review",
                "complete",
                "cancelled"
            ],
            "canonical lifecycle seeded on the never-goal'd board"
        );

        // Doing/Done legacy columns dropped; Backlog kept.
        let manual: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM board_columns WHERE project_id = ? AND column_kind = 'manual'",
        )
        .bind(project)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(manual, vec!["Backlog"], "Doing/Done dropped, Backlog kept");

        // Cards moved: Doing→In Progress, Done→Complete; Backlog card untouched.
        let col_of = |card: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT state_binding FROM board_columns
                     WHERE id = (SELECT column_id FROM cards WHERE id = ?)",
                )
                .bind(card)
                .fetch_optional(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(col_of("card-doing").await.as_deref(), Some("in_progress"));
        assert_eq!(col_of("card-done").await.as_deref(), Some("complete"));
        // Backlog card still in the (manual) Backlog column — no state_binding.
        let backlog_col: String = sqlx::query_scalar(
            "SELECT name FROM board_columns WHERE id = (SELECT column_id FROM cards WHERE id = 'card-backlog')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(backlog_col, "Backlog", "Backlog card preserved in place");

        // Idempotent: a second run changes nothing and does not error.
        migrate_v16_to_v17(&pool).await.unwrap();
        let state_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_columns WHERE project_id = ? AND column_kind = 'state'",
        )
        .bind(project)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state_count, 6, "no duplicate lifecycle columns on re-run");
    }

    /// migrate_v17_to_v18 (#514): a v10-era DB seeded `risk_policy` before the
    /// `goal_cancel` row was added (#500), so it is missing — an unknown
    /// action_class fails closed to Tier 2 and Cancel always 409s. After v18 the
    /// row must exist at Tier 0, a row that diverged to a wrong tier must be
    /// corrected to 0, other rows' customizations must be preserved, and a re-run
    /// must change nothing.
    #[tokio::test]
    async fn migrate_v17_to_v18_reconciles_goal_cancel_risk_policy() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Simulate a v10-era risk_policy: the pre-#500 seed had NO goal_cancel row.
        // Clear the fresh-install seed and re-create the legacy subset, including a
        // user-customized tier on one unrelated row to prove it is preserved.
        sqlx::query("DELETE FROM risk_policy")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO risk_policy (action_class, tier, rationale) VALUES
                ('goal_ready', 0, 'legacy'),
                ('goal_dispatch', 0, 'legacy'),
                ('goal_review', 0, 'legacy'),
                ('goal_complete_confined', 0, 'legacy'),
                ('goal_approve_standard', 1, 'legacy'),
                ('goal_retry_within_budget', 1, 'legacy'),
                ('merge_to_main', 2, 'legacy'),
                ('push_main', 1, 'user-customized down from 2')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (17)")
            .execute(&pool)
            .await
            .unwrap();

        // Precondition: goal_cancel genuinely absent (would fail closed to Tier 2).
        let before: Option<i64> =
            sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = 'goal_cancel'")
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(before, None, "goal_cancel absent on a v10-era DB");

        migrate_v17_to_v18(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 18);

        // goal_cancel now present at Tier 0 — Cancel resolves as reversible, no 409.
        let cancel_tier: i64 =
            sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = 'goal_cancel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cancel_tier, 0, "goal_cancel reconciled to Tier 0");

        // Defensive restore filled in seed rows absent on this legacy DB.
        let secrets_tier: i64 = sqlx::query_scalar(
            "SELECT tier FROM risk_policy WHERE action_class = 'secrets_access'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(secrets_tier, 2, "absent seed row restored");

        // User customization on an existing row is preserved (never clobbered).
        let push_tier: i64 =
            sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = 'push_main'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            push_tier, 1,
            "user-customized tier preserved, not reset to seed"
        );

        // A goal_cancel that exists at a WRONG tier is force-corrected to 0.
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'goal_cancel'")
            .execute(&pool)
            .await
            .unwrap();
        migrate_v17_to_v18(&pool).await.unwrap();
        let recancel_tier: i64 =
            sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = 'goal_cancel'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            recancel_tier, 0,
            "wrong goal_cancel tier force-corrected to 0"
        );

        // Idempotent: a second clean run changes nothing and does not error.
        migrate_v17_to_v18(&pool).await.unwrap();
        let cancel_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM risk_policy WHERE action_class = 'goal_cancel'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cancel_count, 1, "no duplicate goal_cancel row on re-run");
        assert_eq!(push_tier, 1, "customization still preserved after re-run");
    }

    /// Count schema objects (table/view/trigger) by exact name.
    async fn object_exists(pool: &Pool<Sqlite>, name: &str) -> bool {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = ?1")
            .bind(name)
            .fetch_one(pool)
            .await
            .unwrap();
        n > 0
    }

    /// Re-create the dormant pre-v19 `memories` + `knowledge_graph` subset that
    /// `init_spectral_db` used to build (now removed). Lets the test simulate an
    /// existing DB so it can prove `migrate_v18_to_v19` drops the whole unit.
    async fn create_legacy_memories_subset(pool: &Pool<Sqlite>) {
        for stmt in [
            "CREATE TABLE memories (id TEXT PRIMARY KEY, key TEXT NOT NULL, content TEXT NOT NULL)",
            "CREATE VIRTUAL TABLE memories_fts USING fts5(key, content, content=memories, content_rowid=rowid)",
            "CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN \
                INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content); END",
            "CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN \
                INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content); END",
            "CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN \
                INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content); \
                INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content); END",
            "CREATE TABLE knowledge_graph (id TEXT PRIMARY KEY, subject TEXT NOT NULL, predicate TEXT NOT NULL, \
                object TEXT NOT NULL, valid_until TEXT, source_memory_id TEXT REFERENCES memories(id))",
            "CREATE VIRTUAL TABLE knowledge_graph_fts USING fts5(subject, predicate, object, content=knowledge_graph)",
            "CREATE VIEW current_memories AS SELECT * FROM memories",
            "CREATE VIEW current_knowledge AS SELECT * FROM knowledge_graph WHERE valid_until IS NULL",
        ] {
            sqlx::query(stmt).execute(pool).await.unwrap();
        }
    }

    /// migrate_v18_to_v19: the dead `memories` + `knowledge_graph` subset (tables,
    /// FTS, triggers, views) must be gone after the migration, the version stamped
    /// to 19, a re-run must be a clean no-op, and the migration must not error on a
    /// fresh DB that never had the tables (idempotent `DROP ... IF EXISTS`).
    #[tokio::test]
    async fn migrate_v18_to_v19_drops_dead_memories_and_knowledge_graph() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Fresh init no longer creates these — prove that, then simulate a pre-v19
        // DB by re-creating the dormant subset.
        assert!(
            !object_exists(&pool, "memories").await,
            "init_spectral_db must no longer create the dead memories table"
        );
        create_legacy_memories_subset(&pool).await;
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (18)")
            .execute(&pool)
            .await
            .unwrap();

        let dead_objects = [
            "memories",
            "memories_fts",
            "memories_ai",
            "memories_ad",
            "memories_au",
            "knowledge_graph",
            "knowledge_graph_fts",
            "current_memories",
            "current_knowledge",
        ];
        for obj in dead_objects {
            assert!(
                object_exists(&pool, obj).await,
                "precondition: {obj} present before migration"
            );
        }

        migrate_v18_to_v19(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 19);

        for obj in dead_objects {
            assert!(
                !object_exists(&pool, obj).await,
                "{obj} must be dropped after v19"
            );
        }

        // Idempotent: a second run over the now-clean DB changes nothing, no error.
        migrate_v18_to_v19(&pool).await.unwrap();
        for obj in dead_objects {
            assert!(
                !object_exists(&pool, obj).await,
                "{obj} still gone on re-run"
            );
        }

        // Safe on a fresh DB that never had the tables (the fresh-install path).
        let fresh = mem_pool().await;
        init_spectral_db(&fresh).await.unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (18)")
            .execute(&fresh)
            .await
            .unwrap();
        migrate_v18_to_v19(&fresh).await.unwrap();
        assert_eq!(current_version(&fresh).await, 19);
    }
}
