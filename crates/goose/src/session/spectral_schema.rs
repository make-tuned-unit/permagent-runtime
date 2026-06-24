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
            cited_memory_ids    TEXT NOT NULL DEFAULT '[]'
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
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_people_company ON people(company)")
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
                            ('approve_review','unblock','choice','risk_gate','malformed')),
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
}
