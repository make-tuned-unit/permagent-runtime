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
pub const SPECTRAL_SCHEMA_VERSION: i32 = 6;

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

    // ── MEMORIES ──
    sqlx::query(
        "CREATE TABLE memories (
            id              TEXT PRIMARY KEY,
            user_id         TEXT NOT NULL REFERENCES users(id),
            key             TEXT NOT NULL,
            content         TEXT NOT NULL,
            category        TEXT NOT NULL DEFAULT 'core',
            wing            TEXT,
            hall            TEXT,
            room            TEXT,
            embedding       BLOB,
            valid_from      TEXT,
            valid_until     TEXT,
            superseded_by   TEXT,
            confidence      REAL DEFAULT 1.0,
            signal_score    REAL DEFAULT 0.5,
            source_session  TEXT REFERENCES sessions(id),
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_memories_user ON memories(user_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_memories_wing ON memories(wing)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_memories_hall ON memories(hall)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE INDEX idx_memories_current ON memories(valid_until) WHERE valid_until IS NULL",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX idx_memories_signal ON memories(signal_score DESC)")
        .execute(&mut *tx)
        .await?;

    // ── MEMORIES FTS ──
    // FTS virtual tables and triggers must be created outside the transaction
    // on some SQLite builds, so we commit first and create them after.
    // Actually sqlx + SQLite handles this fine within the same connection,
    // but FTS triggers reference the base table so we create them in order.

    sqlx::query(
        "CREATE VIRTUAL TABLE memories_fts USING fts5(
            key, content, content=memories, content_rowid=rowid
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, key, content)
            VALUES (new.rowid, new.key, new.content);
        END",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, key, content)
            VALUES ('delete', old.rowid, old.key, old.content);
        END",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, key, content)
            VALUES ('delete', old.rowid, old.key, old.content);
            INSERT INTO memories_fts(rowid, key, content)
            VALUES (new.rowid, new.key, new.content);
        END",
    )
    .execute(&mut *tx)
    .await?;

    // ── KNOWLEDGE GRAPH ──
    sqlx::query(
        "CREATE TABLE knowledge_graph (
            id                TEXT PRIMARY KEY,
            subject           TEXT NOT NULL,
            predicate         TEXT NOT NULL,
            object            TEXT NOT NULL,
            valid_from        TEXT NOT NULL,
            valid_until       TEXT,
            source_memory_id  TEXT REFERENCES memories(id),
            confidence        REAL DEFAULT 1.0,
            created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX idx_kg_subject ON knowledge_graph(subject)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_kg_predicate ON knowledge_graph(predicate)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_kg_object ON knowledge_graph(object)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX idx_kg_subject_predicate ON knowledge_graph(subject, predicate)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE INDEX idx_kg_current ON knowledge_graph(valid_until) WHERE valid_until IS NULL",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE VIRTUAL TABLE knowledge_graph_fts USING fts5(
            subject, predicate, object, content=knowledge_graph
        )",
    )
    .execute(&mut *tx)
    .await?;

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
    sqlx::query(
        "ALTER TABLE users ADD COLUMN active_workspace_id TEXT REFERENCES workspaces(id)",
    )
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

    // ── VIEWS ──
    sqlx::query(
        "CREATE VIEW current_memories AS
        SELECT * FROM memories WHERE valid_until IS NULL ORDER BY created_at DESC",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE VIEW current_knowledge AS
        SELECT * FROM knowledge_graph WHERE valid_until IS NULL ORDER BY valid_from DESC",
    )
    .execute(&mut *tx)
    .await?;

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

    info!(
        "Spectral schema v{} initialized successfully",
        SPECTRAL_SCHEMA_VERSION
    );
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
        sqlx::query("ALTER TABLE users ADD COLUMN active_workspace_id TEXT REFERENCES workspaces(id)")
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

/// Check whether the Spectral schema has already been initialized.
pub async fn is_schema_initialized(pool: &Pool<Sqlite>) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='users')",
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}
