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
/// Ruling 2026-06-15. v9 is reserved by the session-list-perf branch (committed
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
/// v28 = per-call cost ledger (`cost_ledger`) + O(1) cost-rollup columns on
/// `sessions` (cost-transparency workstream). New table + PRAGMA-guarded ADD
/// COLUMNs, additive and base-independent. `migrate_v27_to_v28` applies it.
///
/// v33 = per-user notification channel thresholds (`notification_preferences`)
/// and the durable daily digest queue (`notification_digest_entries`). New
/// tables only, additive and base-independent. `migrate_v32_to_v33` applies it.
///
/// v34 = local-date tracking for daily digest catch-up. Additive and
/// base-independent. `migrate_v33_to_v34` applies it.
///
/// v35 = cited per-project ecosystem and competitive intelligence
/// (`project_intel`). New table + index, additive and idempotent.
/// `migrate_v34_to_v35` applies it.
///
/// v36 = authenticated principal attribution on Decision-Inbox audit rows.
/// Additive, nullable for legacy/non-HTTP audit events, and idempotent.
/// `migrate_v35_to_v36` applies it.
///
/// v37 = durable effect outbox for Decision-Inbox effects. New table + index,
/// additive and idempotent. `migrate_v36_to_v37` applies it.
///
/// v38 = first-party analytics events (#23 — daemon as collector, no
/// third-party dependency). New table + index, additive and idempotent.
/// `migrate_v37_to_v38` applies it.
///
/// v51 = RLM control-plane context store (`rlm_context`) — the durable
/// replacement for the in-process RLM DashMap, so evaluation context survives a
/// daemon restart. New table + partial index, additive and base-independent.
/// `migrate_v50_to_v51` applies it; `apply_rlm_context_schema` also runs on
/// every boot, version-independent.
///
/// v42 = durable growth actions + pre-registered outcomes (`growth_actions`,
/// `growth_action_outcomes`; docs/proposals/grow-action-outcome-loop.md). Until
/// this existed a growth action had no identity — it was recomputed on every
/// load and cached as JSON under `projects.metadata_json`'s "growth_actions"
/// key (crates/goose-server/src/routes/growth_actions.rs:44), so nothing could
/// be attached to it. New tables + index only, additive and base-independent.
/// `migrate_v41_to_v42` applies it.
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
            cost_usd                       REAL,
            accumulated_cost_usd           REAL,
            accumulated_cache_read_tokens  INTEGER,
            accumulated_cache_write_tokens INTEGER,
            accumulated_cache_savings_usd  REAL,
            schedule_id       TEXT,
            recipe_json       TEXT,
            user_recipe_values_json TEXT,
            provider_name     TEXT,
            model_config_json TEXT,
            goose_mode        TEXT NOT NULL DEFAULT 'auto',
            thread_id         TEXT,
            -- The project the UI had open when this session was created, as a
            -- HYPOTHESIS about scope (see `permagent::session_wing`). NOT the
            -- session's wing: a turn only inherits it when the turn's own
            -- content or tool calls corroborate it. Nullable because a global
            -- chat honestly has no project.
            project_hint_id   TEXT,
            project_hint_wing TEXT,
            -- When this session last wrote a chat turn. A hint does not survive
            -- a long silence: see `permagent::session_wing::HINT_GAP_SECONDS`.
            project_hint_last_turn_at TEXT
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
    // Backs `list_sessions_by_schedule_id`'s `WHERE schedule_id = ?` — the
    // Automate tab's per-schedule session lookup. See
    // `apply_sessions_schedule_id_index` / migrate_v48_to_v49 for the upgrade
    // path (this fresh-init copy keeps a brand-new DB from ever missing it).
    sqlx::query("CREATE INDEX idx_sessions_schedule_id ON sessions(schedule_id)")
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
            skill_path        TEXT,
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
            metadata_json   TEXT NOT NULL DEFAULT '{}',
            graph_entity_id TEXT,
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
    apply_decision_audit_principal_schema(pool).await?;

    // Recognition instrumentation tables (v11). Idempotent; shared with
    // migrate_v10_to_v11 for existing installs.
    apply_recognition_schema(pool).await?;

    // CRM people table (schema v12). Idempotent; shared with migrate_v11_to_v12.
    apply_people_schema(pool).await?;

    // Person-keyed meetings (schema v44). Idempotent; shared with
    // migrate_v43_to_v44 so a fresh install can log a meeting on first boot.
    apply_person_meetings_schema(pool).await?;

    // Person merge/delete bookkeeping (schema v50): absorbed-identifier aliases
    // and the merge/delete snapshot log. Idempotent; shared with
    // migrate_v49_to_v50.
    apply_person_merge_schema(pool).await?;

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

    // Project document hub table (schema v24, #471 Layer 2). Idempotent;
    // shared with migrate_v23_to_v24. Purely additive, base-independent.
    apply_project_documents_schema(pool).await?;

    // Project notes table (schema v25). Idempotent; shared with
    // migrate_v24_to_v25. Purely additive, base-independent.
    apply_project_notes_schema(pool).await?;

    // Durable activity journal (schema v27, #619). Idempotent; shared with
    // migrate_v26_to_v27. Purely additive, base-independent.
    apply_activity_journal_schema(pool).await?;

    // Per-call cost ledger + session cost-rollup columns (schema v28). Idempotent;
    // shared with migrate_v27_to_v28. The ADD COLUMNs below are PRAGMA-guarded, so
    // they no-op here (fresh installs already got the columns from the sessions
    // CREATE above) and only fire on existing DBs.
    apply_cost_ledger_schema(pool).await?;

    // Egress audit log (schema v29, sovereignty). Idempotent; shared with
    // migrate_v28_to_v29. Append-only, purely additive, base-independent.
    apply_egress_audit_schema(pool).await?;

    // Project stack organizer table (schema v31, #512): which services a
    // project runs on + which login identity is used per service —
    // reference-only, no secrets by design. Idempotent; shared with
    // migrate_v30_to_v31. Purely additive, base-independent.
    apply_project_stack_schema(pool).await?;

    // Per-user notification policy + durable daily-digest queue (schema v33,
    // #66). Idempotent; shared with migrate_v32_to_v33.
    apply_notification_routing_schema(pool).await?;

    // Cited project ecosystem + competitive-intelligence findings (schema v35,
    // #889). Idempotent; shared with migrate_v34_to_v35 so fresh installs get
    // the table on first boot, not only after a later upgrade pass.
    apply_project_intel_schema(pool).await?;

    // Durable Decision-Inbox effect outbox (schema v37). Idempotent; shared
    // with migrate_v36_to_v37 so fresh installs get it on first boot.
    apply_effect_outbox_schema(pool).await?;

    // First-party analytics events (schema v38). Idempotent; shared with
    // migrate_v37_to_v38 so fresh installs get it on first boot.
    apply_analytics_events_schema(pool).await?;

    // Durable growth actions + pre-registered outcomes (schema v42).
    // Idempotent; shared with migrate_v41_to_v42. Required here because the
    // `version < N` ladder in `SessionStorage::pool` sits inside the
    // is_schema_initialized branch (session_manager.rs:799) and never runs on
    // a fresh DB — migration-only wiring would leave a first-boot install
    // failing every Grow write with `no such table: growth_actions` until the
    // second daemon boot.
    apply_growth_actions_schema(pool).await?;

    // Daemon control-plane auth audit (schema v43). Idempotent; shared with
    // migrate_v42_to_v43 so a fresh install audits from its first boot rather
    // than only after a later upgrade pass — an audit that starts late is an
    // audit with a hole in it exactly where a new machine is least observed.
    apply_daemon_auth_audit_schema(pool).await?;

    // The Financier's ledger (schema v46): watchlist, notes, positions for
    // the Finance tab. Idempotent; shared with migrate_v45_to_v46. Fresh
    // installs never run the version ladder, so this must live here.
    apply_finance_ledger_schema(pool).await?;
    // Household spend + RSI-alert dedup (schema v47). Same reason: fresh
    // installs never run the version ladder.
    apply_finance_spend_schema(pool).await?;

    // The Forecaster's market-series registry, points, forecasts and briefs
    // (schema v48). Same reason again: fresh installs never run the version
    // ladder, and a first-boot install would otherwise fail every bind with
    // `no such table: forecaster_series` until the second daemon boot.
    apply_forecaster_schema(pool).await?;

    // Failure-learning incident capture. Version-independent, additive, and
    // idempotent so the pinned fresh-init base stamp remains unchanged.
    apply_incidents_schema(pool).await?;
    apply_lessons_schema(pool).await?;

    // The session project hint + per-turn wing provenance. The two `sessions`
    // columns are already in the CREATE TABLE above, so the guarded ADD COLUMNs
    // no-op here; the provenance table is not, and a fresh install that skipped
    // it would silently stop recording which signal winged each turn — the one
    // number this whole change is measured by.
    apply_session_project_hint_schema(pool).await?;

    info!(
        "Spectral schema v{} initialized successfully",
        SPECTRAL_SCHEMA_VERSION
    );
    Ok(())
}

/// Apply the Phase-1 failure-learning incident-capture schema.
///
/// This is deliberately version-independent: incident capture is an additive
/// table and must become available on every database regardless of its recorded
/// base version. It records grounded failures only; later learning-loop stages
/// do not belong in this schema.
/// Governed lesson pool (Phase 3). Two tables by design:
///
/// - `lesson_events` is an APPEND-ONLY ledger. It is the truth.
/// - `lessons` is a derived, mutable projection carrying the current
///   importance, kept for cheap reads.
///
/// The split is what makes a bad lesson revocable: the projection can always be
/// recomputed from the ledger (`lessons::replay_importance`), so drift is
/// corrected by replay rather than by trusting that the mutable copy stayed
/// right. An append-only lesson list with no ledger has no such recovery.
///
/// New-table-only and idempotent, so it runs on every boot and does not disturb
/// the pinned fresh-init stamp.
pub async fn apply_lessons_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS lessons (
            id            TEXT PRIMARY KEY,
            fingerprint   TEXT NOT NULL UNIQUE,
            text          TEXT NOT NULL,
            incident_id   TEXT NOT NULL REFERENCES incidents(id),
            importance    INTEGER NOT NULL DEFAULT 2,
            retired       INTEGER NOT NULL DEFAULT 0,
            created_at    TEXT NOT NULL,
            last_used_at  TEXT
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS lesson_events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            lesson_id    TEXT NOT NULL REFERENCES lessons(id) ON DELETE CASCADE,
            event        TEXT NOT NULL CHECK (event IN
                           ('admitted','corroborated','contradicted','retired')),
            occurred_at  TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    // The ledger is the truth, so it must not be rewritten. Same discipline as
    // `decision_audit` and `egress_audit`.
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS lesson_events_no_update
         BEFORE UPDATE ON lesson_events
         BEGIN SELECT RAISE(ABORT, 'lesson_events is append-only'); END",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS lesson_events_no_delete
         BEFORE DELETE ON lesson_events
         BEGIN SELECT RAISE(ABORT, 'lesson_events is append-only'); END",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_lessons_active ON lessons (retired, importance DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_lesson_events_lesson ON lesson_events (lesson_id)")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn apply_incidents_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS incidents (
            id            TEXT PRIMARY KEY,
            created_at    TEXT NOT NULL,
            session_id    TEXT REFERENCES sessions(id),
            surface       TEXT NOT NULL,
            user_goal     TEXT NOT NULL,
            observation   TEXT NOT NULL,
            mechanism     TEXT NOT NULL CHECK (mechanism IN (
                'A_environment', 'B_design_assumption', 'C_error_swallowing',
                'D_fail_plausible', 'E_operational_omission', 'unclassified'
            )),
            artifact_kind TEXT NOT NULL CHECK (artifact_kind IN (
                'user_report', 'tool_error', 'exit_code', 'http_status',
                'run_diff', 'recognition_record'
            )),
            artifact_ref  TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'open'
                          CHECK (status IN ('open','triaged','regressed','dismissed','resolved')),
            resolved_at   TEXT
        )",
    )
    .execute(pool)
    .await?;

    // Reconcile pre-wave-1 DBs whose CHECK lacks 'resolved' (the table was
    // insert-only; incidents could never close). SQLite cannot alter a CHECK,
    // so this is the documented in-place rebuild — run every boot,
    // version-independent, idempotent via the sqlite_master DDL probe (the
    // same posture the cfg-gated-migration trap demands).
    let ddl: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'incidents'",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(ddl) = ddl {
        if !ddl.contains("'resolved'") {
            let mut conn = pool.acquire().await?;
            sqlx::query("PRAGMA foreign_keys = OFF")
                .execute(&mut *conn)
                .await?;
            let rebuild: Result<()> = async {
                sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
                sqlx::query(
                    "CREATE TABLE incidents_new (
                        id            TEXT PRIMARY KEY,
                        created_at    TEXT NOT NULL,
                        session_id    TEXT REFERENCES sessions(id),
                        surface       TEXT NOT NULL,
                        user_goal     TEXT NOT NULL,
                        observation   TEXT NOT NULL,
                        mechanism     TEXT NOT NULL CHECK (mechanism IN (
                            'A_environment', 'B_design_assumption', 'C_error_swallowing',
                            'D_fail_plausible', 'E_operational_omission', 'unclassified'
                        )),
                        artifact_kind TEXT NOT NULL CHECK (artifact_kind IN (
                            'user_report', 'tool_error', 'exit_code', 'http_status',
                            'run_diff', 'recognition_record'
                        )),
                        artifact_ref  TEXT NOT NULL,
                        status        TEXT NOT NULL DEFAULT 'open'
                                      CHECK (status IN ('open','triaged','regressed','dismissed','resolved')),
                        resolved_at   TEXT
                    )",
                )
                .execute(&mut *conn)
                .await?;
                sqlx::query("INSERT INTO incidents_new SELECT * FROM incidents")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("DROP TABLE incidents").execute(&mut *conn).await?;
                sqlx::query("ALTER TABLE incidents_new RENAME TO incidents")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            .await;
            if rebuild.is_err() {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            }
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *conn)
                .await?;
            rebuild?;
        }
    }

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_incidents_status_created
         ON incidents(status, created_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_incidents_session ON incidents(session_id)")
        .execute(pool)
        .await?;
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
/// The session project hint and the per-turn wing provenance record.
///
/// # What it adds
///
/// * `sessions.project_hint_id` / `sessions.project_hint_wing` — the project
///   the UI had open when the session was created, in canonical
///   `project:<slug>` form plus the slug that would be its wing. It is a
///   HYPOTHESIS about scope, never the session's wing: Spectral measured a
///   session-start pin at 21% verified precision (of 334 turns it would assign,
///   the turn's own content names the same project in 31, a different one in
///   114, none in 189). Writing it as a wing would be invisibly wrong in the
///   recognition ground truth and the TACT gate, so the hint is stored and the
///   wing is earned per turn — see [`crate::session_wing`].
///
/// * `chat_turn_wing_provenance` — one row per chat turn recording what was
///   decided and on what evidence: the hint that was available, the bucket the
///   turn fell into (`corroborated` / `conflicting` / `unverifiable`), which
///   source corroborated it (`content-name` / `alias` / `tool-path`), and the
///   wing actually written. Without this the yield of each signal would be an
///   assumption rather than a measurement, and a turn left honestly unwinged
///   would be indistinguishable from a turn nothing ever looked at.
///
/// Keyed on `memory_key` (the Brain key, `chat-<session>-<idx>`) so a re-write
/// of the same turn replaces its provenance rather than accumulating rows.
///
/// # Idempotence
///
/// PRAGMA-guarded `ADD COLUMN` plus `CREATE TABLE IF NOT EXISTS`: additive,
/// base-version independent, and safe to run on every boot. It is deliberately
/// NOT gated on a version stamp — this codebase has been bitten three times by
/// a schema repair sitting behind a `version < N` that a later stamp skipped
/// past (see the recognition-columns and briefings safety nets), and a missing
/// column here would fail every chat-turn write, not a niche feature.
pub async fn apply_session_project_hint_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    for (col, ddl) in [
        (
            "project_hint_id",
            "ALTER TABLE sessions ADD COLUMN project_hint_id TEXT",
        ),
        (
            "project_hint_wing",
            "ALTER TABLE sessions ADD COLUMN project_hint_wing TEXT",
        ),
        (
            "project_hint_last_turn_at",
            "ALTER TABLE sessions ADD COLUMN project_hint_last_turn_at TEXT",
        ),
    ] {
        let has_column: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?")
                .bind(col)
                .fetch_one(&mut *tx)
                .await?;
        if has_column == 0 {
            sqlx::query(ddl).execute(&mut *tx).await?;
        }
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS chat_turn_wing_provenance (
            memory_key        TEXT PRIMARY KEY,
            session_id        TEXT NOT NULL,
            project_hint_id   TEXT,
            project_hint_wing TEXT,
            verdict           TEXT NOT NULL
                              CHECK (verdict IN ('corroborated','conflicting','unverifiable')),
            corroborated_by   TEXT,
            named_wing        TEXT,
            wing_written      TEXT,
            created_at        TEXT NOT NULL
                              DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chat_turn_wing_provenance_session
         ON chat_turn_wing_provenance(session_id)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_chat_turn_wing_provenance_verdict
         ON chat_turn_wing_provenance(verdict)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

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
            injected_memory_ids TEXT,
            injected_memory_ids_source TEXT,
            citation_checked_at TEXT,
            outcome_label       TEXT,
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

/// Person-keyed meetings (schema v44). One row per logged meeting with a
/// directory person — the profile timeline and the Home Calendar card both
/// read this table. Calendar.app is a best-effort write on create, not the
/// source of truth. Fully idempotent (`CREATE TABLE / INDEX IF NOT EXISTS`).
pub async fn apply_person_meetings_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS person_meetings (
            id               TEXT PRIMARY KEY,
            entity_uuid      TEXT NOT NULL REFERENCES people(entity_uuid) ON DELETE CASCADE,
            title            TEXT NOT NULL,
            starts_at        TEXT NOT NULL,
            ends_at          TEXT,
            notes            TEXT NOT NULL DEFAULT '',
            calendar_synced  INTEGER NOT NULL DEFAULT 0,
            project_id       TEXT,
            follow_up_at     TEXT,
            follow_up_note   TEXT NOT NULL DEFAULT '',
            follow_up_done   INTEGER NOT NULL DEFAULT 0,
            calendar_uid     TEXT,
            created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_meetings_person \
         ON person_meetings(entity_uuid, starts_at DESC)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_meetings_starts ON person_meetings(starts_at)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_person_meetings_updated_at
            AFTER UPDATE ON person_meetings
            FOR EACH ROW
            BEGIN
                UPDATE person_meetings SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                WHERE id = NEW.id;
            END",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    apply_person_meetings_v45_columns(pool).await?;
    Ok(())
}

/// v45 columns on an existing v44 `person_meetings` table. Fresh installs get
/// these from CREATE TABLE; existing DBs get a PRAGMA-guarded ADD COLUMN.
pub async fn apply_person_meetings_v45_columns(pool: &Pool<Sqlite>) -> Result<()> {
    for (column, ddl) in [
        (
            "project_id",
            "ALTER TABLE person_meetings ADD COLUMN project_id TEXT",
        ),
        (
            "follow_up_at",
            "ALTER TABLE person_meetings ADD COLUMN follow_up_at TEXT",
        ),
        (
            "follow_up_note",
            "ALTER TABLE person_meetings ADD COLUMN follow_up_note TEXT NOT NULL DEFAULT ''",
        ),
        (
            "follow_up_done",
            "ALTER TABLE person_meetings ADD COLUMN follow_up_done INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "calendar_uid",
            "ALTER TABLE person_meetings ADD COLUMN calendar_uid TEXT",
        ),
    ] {
        let has_column: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('person_meetings') WHERE name = ?",
        )
        .bind(column)
        .fetch_one(pool)
        .await?;
        if has_column == 0 {
            sqlx::query(ddl).execute(pool).await?;
        }
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_meetings_project \
         ON person_meetings(project_id, starts_at DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_meetings_follow_up \
         ON person_meetings(follow_up_at) WHERE follow_up_done = 0",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_person_meetings_calendar_uid \
         ON person_meetings(calendar_uid) WHERE calendar_uid IS NOT NULL",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v44: person-keyed meetings. New table + indexes + updated_at trigger;
/// additive and base-independent. Fresh installs get the same table from
/// `init_spectral_db`, which never reaches the migration ladder.
pub async fn migrate_v43_to_v44(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v43 -> v44 (person meetings)");
    apply_people_schema(pool).await?;
    apply_person_meetings_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (44)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v44 (person meetings)");
    Ok(())
}

/// v45: follow-up, optional project, and Calendar.app uid on person_meetings.
/// Additive ALTER + indexes; base-independent. Fresh installs get the same
/// columns from `apply_person_meetings_schema`.
pub async fn migrate_v44_to_v45(pool: &Pool<Sqlite>) -> Result<()> {
    info!(
        "Migrating Spectral schema v44 -> v45 (person meeting follow-up / project / calendar uid)"
    );
    apply_people_schema(pool).await?;
    apply_person_meetings_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (45)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v45 (person meeting follow-up / project / calendar uid)");
    Ok(())
}

/// The Financier's ledger (v46): watchlist, research notes, and positions
/// the Finance tab and the Financier's tools both write. Quotes are never
/// stored — they are fetched at read time. Idempotent.
pub async fn apply_finance_ledger_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_watchlist (
            id          TEXT PRIMARY KEY,
            symbol      TEXT NOT NULL UNIQUE,
            label       TEXT,
            notes       TEXT,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_notes (
            id          TEXT PRIMARY KEY,
            title       TEXT NOT NULL,
            body        TEXT NOT NULL,
            symbol      TEXT,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_positions (
            id            TEXT PRIMARY KEY,
            symbol        TEXT NOT NULL,
            company_name  TEXT NOT NULL,
            entry_date    TEXT NOT NULL,
            entry_price   REAL NOT NULL,
            shares        INTEGER NOT NULL,
            exit_date     TEXT,
            exit_price    REAL,
            notes         TEXT,
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_finance_notes_created
         ON finance_notes (created_at DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_finance_positions_entry
         ON finance_positions (entry_date DESC)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v46: Financier ledger tables. Additive and base-independent.
pub async fn migrate_v45_to_v46(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v45 -> v46 (financier ledger)");
    apply_finance_ledger_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (46)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v46 (financier ledger)");
    Ok(())
}

/// Household spend ledger + RSI-alert dedup (v47). Quotes still never stored.
pub async fn apply_finance_spend_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_transactions (
            id           TEXT PRIMARY KEY,
            date         TEXT NOT NULL,
            amount       REAL NOT NULL,
            payee        TEXT NOT NULL,
            category     TEXT NOT NULL,
            account      TEXT,
            source_file  TEXT,
            created_at   TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_finance_txn_date
         ON finance_transactions (date DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_rsi_alerts (
            symbol      TEXT NOT NULL,
            day         TEXT NOT NULL,
            rsi         REAL NOT NULL,
            threshold   REAL NOT NULL,
            created_at  TEXT NOT NULL,
            PRIMARY KEY (symbol, day)
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finance_daily_picks (
            day              TEXT PRIMARY KEY,
            as_of            TEXT NOT NULL,
            ticker           TEXT,
            company_name     TEXT,
            why              TEXT NOT NULL,
            model            TEXT,
            candidate_count  INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v47: spend ledger + RSI alert dedup. Additive and base-independent.
pub async fn migrate_v46_to_v47(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v46 -> v47 (finance spend + RSI alerts)");
    // Re-apply the v46 ledger first. A DB can be stamped 46/47 with the
    // spend tables present and watchlist/notes/positions gone (2026-08-21).
    apply_finance_ledger_schema(pool).await?;
    apply_finance_spend_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (47)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v47 (finance spend + RSI alerts)");
    Ok(())
}

/// Apply The Forecaster's schema (v48): the market series registry, its
/// append-only points, its forecasts, and the weekly per-project brief.
///
/// A deliberate departure from the Financier, which never stores a quote
/// (`apply_finance_ledger_schema`, "a price is only a price at read time").
/// The Forecaster *must* store, because history is the product: the Financier
/// answers *what is it now*, the Forecaster *where is it going*, and only the
/// second needs a past. The rule is scoped, not broken — nothing here caches a
/// live quote for a read-time answer.
///
/// `forecaster_series` carries no subject list of its own. A row hangs off an
/// existing `project_intel` row (`intel_id`) or off the project itself, so the
/// Market card and the Ecosystem panel are one concept rather than two lists
/// that can disagree. `intel_id` is nullable and deliberately un-foreign-keyed:
/// dismissing an intel row must not silently delete collected history.
///
/// Fully idempotent (IF NOT EXISTS), so it is safe on every boot and on fresh
/// installs.
pub async fn apply_forecaster_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS forecaster_series (
            id                TEXT PRIMARY KEY,
            project_id        TEXT NOT NULL,
            intel_id          TEXT,
            source_kind       TEXT NOT NULL,
            subject           TEXT NOT NULL,
            cadence           TEXT NOT NULL DEFAULT 'daily'
                              CHECK (cadence IN ('daily','weekly')),
            label             TEXT NOT NULL DEFAULT '',
            status            TEXT NOT NULL DEFAULT 'proposed'
                              CHECK (status IN ('proposed','active','dismissed')),
            last_collected_at TEXT,
            last_error        TEXT,
            created_at        TEXT NOT NULL
                              DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await?;
    // The registry's identity. Re-proposing the same subject for the same
    // project must reach the existing row rather than fork its history.
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_forecaster_series_subject
         ON forecaster_series (project_id, source_kind, subject)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_forecaster_series_project
         ON forecaster_series (project_id, status)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_forecaster_series_intel
         ON forecaster_series (intel_id) WHERE intel_id IS NOT NULL",
    )
    .execute(pool)
    .await?;

    // Append-only observations. The composite primary key is what makes
    // re-collecting an overlapping window idempotent: the collector always
    // writes INSERT OR IGNORE and a second pass over the same days inserts
    // zero rows rather than doubling the series.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS forecaster_points (
            series_id  TEXT NOT NULL,
            ts         TEXT NOT NULL,
            value      REAL NOT NULL,
            PRIMARY KEY (series_id, ts)
        )",
    )
    .execute(pool)
    .await?;

    // `method` is NOT NULL by design. A forecast whose label does not match
    // what produced it is the failure mode this whole feature is built to
    // avoid, so the column cannot be skipped on the way in.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS forecaster_forecasts (
            id               TEXT PRIMARY KEY,
            series_id        TEXT NOT NULL,
            made_at          TEXT NOT NULL,
            horizon          INTEGER NOT NULL,
            method           TEXT NOT NULL,
            point_json       TEXT NOT NULL DEFAULT '[]',
            quantiles_json   TEXT NOT NULL DEFAULT '{}',
            mase_vs_baseline REAL,
            folds            INTEGER,
            fold_wins        INTEGER
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_forecaster_forecasts_series
         ON forecaster_forecasts (series_id, made_at DESC)",
    )
    .execute(pool)
    .await?;

    // One synthesised market brief per project per sweep. `method_mix` records
    // which methods actually produced the numbers the prose restates, so the
    // brief can never outrun its own evidence.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS forecaster_briefs (
            id             TEXT PRIMARY KEY,
            project_id     TEXT NOT NULL,
            generated_at   TEXT NOT NULL,
            summary        TEXT NOT NULL,
            method_mix     TEXT NOT NULL DEFAULT '{}',
            input_json     TEXT NOT NULL DEFAULT '{}',
            model          TEXT
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_forecaster_briefs_project
         ON forecaster_briefs (project_id, generated_at DESC)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v48: The Forecaster's series registry, points, forecasts and briefs.
/// New tables and indexes only; additive and base-version independent.
pub async fn migrate_v47_to_v48(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v47 -> v48 (forecaster market series)");
    apply_forecaster_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (48)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v48 (forecaster market series)");
    Ok(())
}

/// Apply the `sessions.schedule_id` index (nightly health review, 2026-08-25 —
/// "schedule polling storm"). `sessions` carries `user_id`, `updated_at`,
/// `session_type`, `thread_id` indexes but never one on `schedule_id`, even
/// though `SessionStorage::list_sessions_by_schedule_id`
/// (`WHERE s.schedule_id = ?`) is exactly the query the Automate tab polls
/// every 5-15s per scheduled job — an unindexed full scan of `sessions`
/// (~970 rows), 1-3.7s per poll under load. Additive and idempotent
/// (`CREATE INDEX IF NOT EXISTS`): safe to run on a DB that already has it.
pub async fn apply_sessions_schedule_id_index(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_schedule_id ON sessions(schedule_id)")
        .execute(pool)
        .await?;
    Ok(())
}

/// Migrate an existing database to add the `sessions.schedule_id` index
/// (schema v49). Base-independent (`CREATE INDEX IF NOT EXISTS`), so it
/// applies cleanly regardless of which prior version the DB sits at.
pub async fn migrate_v48_to_v49(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v48 -> v49 (sessions.schedule_id index)");
    apply_sessions_schedule_id_index(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (49)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v49 (sessions.schedule_id index)");
    Ok(())
}

/// People merge/delete bookkeeping (schema v50).
///
/// Two tables, both additive and idempotent:
///
/// * `person_aliases` — every identifier a *surviving* person has absorbed.
///   Spectral has no entity re-key API (see `people_merge`), so when a
///   duplicate is merged away its `entity_uuid`, `canonical_id`,
///   `graph_entity_id` and `display_name` are recorded here against the
///   survivor. That is what keeps the duplicate's Brain memories reachable:
///   `/api/people/{id}/activity` matches memories by the person's NAME, so an
///   absorbed name keeps finding them on the survivor's profile.
/// * `person_merge_log` — one row per merge or delete, carrying a JSON
///   snapshot of everything that moved. It is both the audit record and the
///   undo source (`people_merge::undo_merge`).
///
/// `person_aliases.entity_uuid` cascades from `people`, so deleting a survivor
/// later takes their absorbed aliases with them. `person_merge_log` does NOT
/// reference `people`: it has to outlive the row it describes, which is the
/// whole point of a snapshot.
pub async fn apply_person_merge_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS person_aliases (
            id           TEXT PRIMARY KEY,
            entity_uuid  TEXT NOT NULL REFERENCES people(entity_uuid) ON DELETE CASCADE,
            alias_kind   TEXT NOT NULL CHECK (alias_kind IN
                            ('entity_uuid','canonical_id','graph_entity_id','display_name')),
            alias_value  TEXT NOT NULL,
            merge_id     TEXT,
            created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            UNIQUE (alias_kind, alias_value)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_aliases_entity ON person_aliases(entity_uuid)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_person_aliases_merge ON person_aliases(merge_id)")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS person_merge_log (
            id              TEXT PRIMARY KEY,
            kind            TEXT NOT NULL CHECK (kind IN ('merge','delete')),
            survivor_uuid   TEXT,
            duplicate_uuid  TEXT NOT NULL,
            summary         TEXT NOT NULL DEFAULT '',
            snapshot        TEXT NOT NULL,
            undone_at       TEXT,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_merge_log_created \
         ON person_merge_log(created_at DESC)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_person_merge_log_survivor \
         ON person_merge_log(survivor_uuid)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// v50: people merge/delete bookkeeping (`person_aliases` + `person_merge_log`).
/// New tables + indexes only — additive and base-independent, so it applies
/// cleanly over any earlier base. Fresh installs get the same tables from
/// `init_spectral_db`, which never reaches the migration ladder.
pub async fn migrate_v49_to_v50(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v49 -> v50 (person aliases + merge log)");
    apply_people_schema(pool).await?;
    apply_person_merge_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (50)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v50 (person aliases + merge log)");
    Ok(())
}

/// Apply the RLM control-plane context store (`rlm_context`).
///
/// The durable replacement for the process-local `DashMap` that used to be the
/// whole of [`crate::rlm`]: a transactional, versioned, exactly-read key/value
/// store scoped by session or goal, so evaluation context outlives an LLM turn
/// AND a daemon restart. It lives here rather than in the Brain because recall
/// is ranked and probabilistic — a control plane must read back exactly what it
/// wrote — and rather than in `cards.metadata_json` because that is an
/// unversioned blob whose read-modify-write loses concurrent updates.
///
/// `permagent.db` already runs in WAL with a checkpoint timer and is already in
/// the hourly backup snapshot set as `DbTarget::Spectral`, so this table
/// inherits its durability rather than inventing any.
///
/// Fully idempotent (`CREATE TABLE / INDEX IF NOT EXISTS`) and applied on every
/// boot, not only behind the version gate: a version-gated schema step is
/// exactly how the recognition columns and the finance tables went missing in
/// production (see the notes in `SessionManager`).
pub async fn apply_rlm_context_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS rlm_context (
            scope      TEXT    NOT NULL CHECK (scope IN ('session','goal')),
            scope_id   TEXT    NOT NULL,
            key        TEXT    NOT NULL,
            value_json TEXT    NOT NULL,
            version    INTEGER NOT NULL DEFAULT 1,
            created_at TEXT    NOT NULL,
            updated_at TEXT    NOT NULL,
            expires_at TEXT,
            PRIMARY KEY (scope, scope_id, key)
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_rlm_context_expiry \
         ON rlm_context(expires_at) WHERE expires_at IS NOT NULL",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v51: the RLM control-plane context store. New table + partial index only —
/// additive and base-independent, so it applies cleanly over any earlier base.
pub async fn migrate_v50_to_v51(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v50 -> v51 (RLM control-plane context store)");
    apply_rlm_context_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (51)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v51 (RLM control-plane context store)");
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

/// Apply the project-document-hub schema (v24, #471 Layer 2): `project_documents`,
/// a per-project attachment relation. Each row references a file on disk under
/// `~/.permagent/project-docs/<project_id>/<id>/<filename>`; the row carries the
/// mime type so the in-app viewer can dispatch a renderer. The `project_id` FK
/// cascades on project delete (the route layer removes the files). Fully
/// idempotent (`CREATE TABLE / INDEX IF NOT EXISTS`).
pub async fn apply_project_documents_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_documents (
            id           TEXT PRIMARY KEY,
            project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            filename     TEXT NOT NULL,
            mime_type    TEXT NOT NULL,
            size_bytes   INTEGER NOT NULL,
            path         TEXT NOT NULL,
            uploaded_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_documents_project \
         ON project_documents(project_id, uploaded_at DESC)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the project-document-hub schema (schema v24).
///
/// Purely additive and base-version independent (`CREATE TABLE IF NOT EXISTS`),
/// so it applies cleanly over any earlier base. Records v24 in `schema_version`.
pub async fn migrate_v23_to_v24(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v23 -> v24 (project documents)");

    apply_project_documents_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (24)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v24 (project documents)");

    Ok(())
}

/// Apply the project-notes schema (v25): `project_notes`, a per-project
/// freeform note relation. Each row is a title + body the user (or agent) wrote
/// on a project; `memory_key` records the Brain key its text was indexed under
/// so the note is recallable + Librarian-enriched. The `project_id` FK cascades
/// on project delete. Fully idempotent (`CREATE TABLE / INDEX IF NOT EXISTS`).
pub async fn apply_project_notes_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_notes (
            id           TEXT PRIMARY KEY,
            project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            title        TEXT,
            body         TEXT NOT NULL,
            memory_key   TEXT,
            created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_notes_project \
         ON project_notes(project_id, created_at DESC)",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the project-notes schema (schema v25).
///
/// Purely additive and base-version independent (`CREATE TABLE IF NOT EXISTS`),
/// so it applies cleanly over any earlier base. Records v25 in `schema_version`.
pub async fn migrate_v24_to_v25(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v24 -> v25 (project notes)");

    apply_project_notes_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (25)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v25 (project notes)");

    Ok(())
}

/// Ensure `projects.metadata_json` exists (schema v26, #456 / ruling 3 in
/// GOAL_COMPLETION_AND_VERIFICATION.md §3d): a general project metadata bag
/// mirroring `cards.metadata_json`. First tenant: `build_command` — the
/// project-level build check the orchestrator seeds onto code-flavored goals
/// as a `command_exit_zero` completion check. The publish sequence (#457)
/// lands in the same bag later. PRAGMA-guarded ADD COLUMN, idempotent.
pub async fn apply_project_metadata_column(pool: &Pool<Sqlite>) -> Result<()> {
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'metadata_json'",
    )
    .fetch_one(pool)
    .await?;
    if has_column == 0 {
        sqlx::query("ALTER TABLE projects ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'")
            .execute(pool)
            .await?;
        info!("Added projects.metadata_json column (schema v26)");
    }
    Ok(())
}

/// Add the `projects.graph_entity_id` bridge column (#595 — graph identity for
/// non-ontology projects). Mirrors `people.graph_entity_id` (#583): the bare
/// 64-hex content-addressed `EntityId` of the project's graph node, filled when
/// the project first needs a graph identity (ontology-resolved or
/// runtime-minted on person→project associate). PRAGMA-guarded and applied by
/// column-existence — NOT gated on a version stamp (the `apply_skill_path_column`
/// precedent), so it is present on any DB regardless of the recorded schema
/// version. Idempotent.
pub async fn apply_project_graph_entity_column(pool: &Pool<Sqlite>) -> Result<()> {
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'graph_entity_id'",
    )
    .fetch_one(pool)
    .await?;
    if has_column == 0 {
        sqlx::query("ALTER TABLE projects ADD COLUMN graph_entity_id TEXT")
            .execute(pool)
            .await?;
        info!("Added projects.graph_entity_id column (graph identity bridge, #595)");
    }
    Ok(())
}

/// Add the `skills.skill_path` index column: it points each indexed skill at its
/// on-disk `SKILL.md` folder (the portable agentskills.io source-of-truth). The
/// Apply the `agent_briefings` schema — the worker-agents-report-to-Henry
/// line (see [`crate::briefings`]).
///
/// Deliberately NOT a numbered migration. Applied by table existence on every
/// boot, so it cannot be skipped by a version stamp advancing past it — the
/// same hazard that left `recognition_verdict` missing from a v40 production DB
/// for weeks. Additive and idempotent; SPECTRAL_SCHEMA_VERSION stays 14.
pub async fn apply_briefings_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_briefings (
            id              TEXT PRIMARY KEY,
            from_agent      TEXT NOT NULL,
            kind            TEXT NOT NULL,
            severity        TEXT NOT NULL,
            summary         TEXT NOT NULL,
            detail          TEXT,
            ref_kind        TEXT,
            ref_id          TEXT,
            created_at      TEXT NOT NULL,
            acknowledged_at TEXT
        )",
    )
    .execute(pool)
    .await?;

    // The read path is always "what has Henry not seen yet", so the index is
    // on the unacknowledged set rather than the whole table.
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_briefings_unacked
            ON agent_briefings(acknowledged_at, created_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// DB row is the fast lookup + repetition-detection index; the folder is
/// authoritative. PRAGMA-guarded and applied by column-existence — NOT gated on
/// a version stamp — so it is present on any DB regardless of the recorded schema
/// version (mirrors the recognition-columns safety-net precedent, since the
/// SPECTRAL_SCHEMA_VERSION const is not bumped for this additive column).
/// Idempotent.
pub async fn apply_skill_path_column(pool: &Pool<Sqlite>) -> Result<()> {
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('skills') WHERE name = 'skill_path'",
    )
    .fetch_one(pool)
    .await?;
    if has_column == 0 {
        sqlx::query("ALTER TABLE skills ADD COLUMN skill_path TEXT")
            .execute(pool)
            .await?;
        info!("Added skills.skill_path column (SKILL.md source-of-truth index)");
    }
    Ok(())
}

/// Migrate an existing database to the project-metadata schema (schema v26).
///
/// A single guarded `ALTER TABLE ... ADD COLUMN`, base-version independent and
/// idempotent, so it applies cleanly over any earlier base. Records v26 in
/// `schema_version`.
pub async fn migrate_v25_to_v26(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v25 -> v26 (projects.metadata_json)");

    apply_project_metadata_column(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (26)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v26 (projects.metadata_json)");

    Ok(())
}

/// Apply the durable activity-journal schema (v27, #619): `activity_journal`,
/// an append-only log of selected event-bus kinds (goal transitions, decisions,
/// librarian runs, task failures) with actor + evidence pointer. The journal
/// INDEXES the existing durable stores — `ref_kind`/`ref_id` point at the card,
/// decision, memory, or task — it never duplicates their bodies. `ts` is an
/// RFC3339 UTC millisecond timestamp (same shape as the strftime defaults
/// elsewhere in this schema), so lexicographic order is chronological order and
/// the DESC index serves the newest-first timeline page directly. Fully
/// idempotent (`CREATE TABLE / INDEX IF NOT EXISTS`; the guard triggers are
/// dropped and recreated so their retention window follows the Rust constant).
pub async fn apply_activity_journal_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS activity_journal (
            id       TEXT PRIMARY KEY,
            ts       TEXT NOT NULL,
            kind     TEXT NOT NULL,
            actor    TEXT NOT NULL,
            title    TEXT NOT NULL,
            detail   TEXT,
            ref_kind TEXT,
            ref_id   TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_activity_journal_ts ON activity_journal(ts DESC)")
        .execute(&mut *tx)
        .await?;

    // This user-facing record of what agents did must not be rewritable; deletion
    // is legitimate only for the retention pass after the window.
    sqlx::query("DROP TRIGGER IF EXISTS trg_activity_journal_no_update")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE TRIGGER trg_activity_journal_no_update
         BEFORE UPDATE ON activity_journal
         BEGIN SELECT RAISE(ABORT, 'activity_journal is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DROP TRIGGER IF EXISTS trg_activity_journal_no_delete")
        .execute(&mut *tx)
        .await?;
    // `RETENTION_DAYS` is a compile-time i64 const in activity_journal.rs, not
    // external data.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER trg_activity_journal_no_delete
         BEFORE DELETE ON activity_journal
         WHEN OLD.ts >= strftime('%Y-%m-%dT%H:%M:%fZ','now','-{} days')
         BEGIN SELECT RAISE(ABORT, 'activity_journal is append-only'); END",
        crate::activity_journal::RETENTION_DAYS
    )))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the activity-journal schema (schema v27).
///
/// Purely additive and base-version independent (`CREATE TABLE IF NOT EXISTS`),
/// so it applies cleanly over any earlier base. Records v27 in `schema_version`.
pub async fn migrate_v26_to_v27(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v26 -> v27 (activity journal)");

    apply_activity_journal_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (27)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v27 (activity journal)");

    Ok(())
}

/// Apply the egress-audit schema (schema v29, sovereignty): `egress_audit` —
/// the always-on, append-only record of every **cloud** inference call the
/// sovereignty guard sees (blocked or allowed), with a SHA-256 content hash by
/// default (full prompt only when `sovereign_capture_prompts` is enabled).
///
/// Append-only is enforced *at the DB*: `BEFORE UPDATE` / `BEFORE DELETE`
/// triggers `RAISE(ABORT)` so evidence of a cloud egress can never be quietly
/// rewritten or deleted — a mutable audit is a lying audit. Purely additive and
/// base-version independent (`CREATE ... IF NOT EXISTS`), so it applies cleanly
/// over any earlier base and is safe on every boot. See
/// [`crate::sovereignty`] for the writer/reader and enforcement policy.
pub async fn apply_egress_audit_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS egress_audit (
            id           TEXT PRIMARY KEY,
            ts           TEXT NOT NULL,
            provider     TEXT NOT NULL,
            model        TEXT NOT NULL,
            session_id   TEXT,
            project_id   TEXT,
            kind         TEXT NOT NULL DEFAULT 'inference',
            blocked      INTEGER NOT NULL DEFAULT 0,
            content_hash TEXT NOT NULL,
            prompt       TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_egress_audit_ts ON egress_audit(ts DESC)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_egress_audit_no_update
            BEFORE UPDATE ON egress_audit
            BEGIN SELECT RAISE(ABORT, 'egress_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_egress_audit_no_delete
            BEFORE DELETE ON egress_audit
            BEGIN SELECT RAISE(ABORT, 'egress_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the egress-audit schema (schema v29).
///
/// Purely additive and base-version independent; records v29 in
/// `schema_version`.
pub async fn migrate_v28_to_v29(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v28 -> v29 (egress audit log)");
    apply_egress_audit_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (29)")
        .execute(pool)
        .await?;
    Ok(())
}

/// Backfill the Failed goal-lifecycle column (schema v30, #250). Boards seeded
/// before the Failed column existed lack the target column `park_goal` writes
/// into once exhausted goals stop re-pooling into Triage. A data fixup — no
/// DDL — base-version independent and idempotent (insert-where-absent, mirrors
/// the v16 Cancelled backfill). Records v30.
pub async fn migrate_v29_to_v30(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v29 -> v30 (backfill Failed column, #250)");

    let added = crate::cards::backfill_failed_column(pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Hardcoded (v30) per the migration precedent in this file.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (30)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v30 ({added} Failed columns added)");

    Ok(())
}

/// Apply the project stack-organizer schema (#512): `project_stack_entries`
/// — one row per service a project is built on, recording WHICH login identity
/// (email/handle) is used for that service on that project.
///
/// REFERENCE-ONLY BY DESIGN: there is deliberately no column for a password,
/// token, or secret of any kind, and none may ever be added here — the actual
/// credential stays in the user's password manager. `identity` is the
/// low-sensitivity account label ("jesse+kinrows@…"), nothing more (#512 ruled
/// out autofill/secret storage after the WKWebView/Associated-Domains research).
///
/// The `category` CHECK mirrors the `projects.status` inline-CHECK precedent;
/// the Rust-side [`crate::project_stack::VALID_CATEGORIES`] list must stay in
/// sync with it (widen BOTH if a category is ever added). The `project_id` FK
/// cascades on project delete. Fully idempotent
/// (`CREATE TABLE / INDEX / TRIGGER IF NOT EXISTS`), base-independent.
pub async fn apply_project_stack_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_stack_entries (
            id            TEXT PRIMARY KEY,
            project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            service_name  TEXT NOT NULL,
            category      TEXT NOT NULL DEFAULT 'other'
                          CHECK (category IN ('hosting', 'database', 'backend', 'auth',
                                              'analytics', 'social', 'domain', 'other')),
            identity      TEXT,
            notes         TEXT NOT NULL DEFAULT '',
            dashboard_url TEXT,
            created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_stack_entries_project \
         ON project_stack_entries(project_id, category, service_name)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_project_stack_entries_updated_at
            AFTER UPDATE ON project_stack_entries
            FOR EACH ROW
            BEGIN
                UPDATE project_stack_entries SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                WHERE id = NEW.id;
            END",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the project stack-organizer schema (v31,
/// #512). Purely additive and base-version independent
/// (`CREATE ... IF NOT EXISTS`), so it applies cleanly over any earlier base.
/// Records v31 in `schema_version`. (Reconciled onto v31: main already used v30
/// for the #250 Failed-column backfill.)
pub async fn migrate_v30_to_v31(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v30 -> v31 (project stack organizer)");
    apply_project_stack_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (31)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v31 (project stack organizer)");
    Ok(())
}

/// v32 (#430, S4): reconcile the supervised-CC-gate `risk_policy` seed onto
/// existing DBs. Adds the three `cc_*` action classes the S4 classifier
/// resolves gates to (`platform_extensions::gate_classifier::SEEDED_CLASSES`);
/// `network_external` and the fail-closed `cc_unclassified` sentinel are NOT
/// added (the former predates S4, the latter is deliberately unseeded).
///
/// `INSERT OR IGNORE` so any user/Henry tier customization on an already-present
/// row survives (same posture as the v18 reconcile). Purely additive to a
/// free-text-PK table — no CHECK to widen — and base-independent, so it applies
/// cleanly over any earlier base. Records v32 in `schema_version`.
pub async fn migrate_v31_to_v32(pool: &Pool<Sqlite>) -> Result<()> {
    info!(
        "Migrating Spectral schema v31 -> v32 (seed supervised-CC-gate risk_policy classes, #430)"
    );
    sqlx::query(
        "INSERT OR IGNORE INTO risk_policy (action_class, tier, rationale) VALUES
            ('cc_read_only', 0, 'Supervised CC read-only tool (Read/Glob/Grep/LS/NotebookRead/BashOutput/TodoWrite) — no effect outside the session'),
            ('cc_workspace_edit', 1, 'Supervised CC file edit (Write/Edit/MultiEdit/NotebookEdit) — confined, git-reversible; recorded decision'),
            ('cc_shell', 2, 'Supervised CC shell (Bash/KillBash) — arbitrary command surface; user-only')",
    )
    .execute(pool)
    .await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (32)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v32 (supervised-CC-gate risk_policy classes seeded)");
    Ok(())
}

/// Apply notification routing policy and the durable digest queue (#66).
/// Severity is ordered info=1, warning=2, critical=3; NULL disables a channel.
/// Keeping thresholds in rows (rather than process configuration) makes policy
/// genuinely per-user and gives future multi-user installs the same behavior.
pub async fn apply_notification_routing_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS notification_preferences (
            user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
            push_min_severity INTEGER CHECK (push_min_severity BETWEEN 1 AND 3),
            in_app_min_severity INTEGER CHECK (in_app_min_severity BETWEEN 1 AND 3),
            digest_min_severity INTEGER CHECK (digest_min_severity BETWEEN 1 AND 3),
            digest_hour_local INTEGER NOT NULL DEFAULT 8 CHECK (digest_hour_local BETWEEN 0 AND 23),
            last_digest_delivery_date TEXT,
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO notification_preferences
            (user_id, push_min_severity, in_app_min_severity, digest_min_severity)
         SELECT id, 3, 2, 1 FROM users",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS notification_digest_entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            source_event_id TEXT NOT NULL,
            severity INTEGER NOT NULL CHECK (severity BETWEEN 1 AND 3),
            source_type TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            delivered_at TEXT,
            UNIQUE(user_id, source_event_id)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_notification_digest_pending
         ON notification_digest_entries(user_id, delivered_at, created_at)",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// v33 (#66): per-user severity thresholds and durable daily digest queue.
/// New tables only, base-independent, and safe to run repeatedly.
pub async fn migrate_v32_to_v33(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v32 -> v33 (notification routing, #66)");
    apply_notification_routing_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (33)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v33 (notification routing)");
    Ok(())
}

/// v34: remember the local date of the last committed digest delivery.
/// PRAGMA guarding makes the additive migration idempotent on every database.
pub async fn migrate_v33_to_v34(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v33 -> v34 (digest catch-up tracking)");
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('notification_preferences')
         WHERE name = 'last_digest_delivery_date'",
    )
    .fetch_one(pool)
    .await?;
    if has_column == 0 {
        sqlx::query(
            "ALTER TABLE notification_preferences ADD COLUMN last_digest_delivery_date TEXT",
        )
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (34)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v34 (digest catch-up tracking)");
    Ok(())
}

/// Apply the project-intelligence schema (v35, #889): the `project_intel`
/// table + its project index. Shared by `migrate_v34_to_v35` (existing DBs) and
/// `init_spectral_db` (fresh installs) so a brand-new database gets the table on
/// its first boot — not only after a later upgrade pass. Fully idempotent
/// (IF NOT EXISTS), so it is safe on every boot and on fresh installs.
pub async fn apply_project_intel_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_intel (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('competitor','partner','adjacent')),
            name TEXT NOT NULL,
            note TEXT,
            source_url TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_project_intel_project ON project_intel(project_id)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v35 (#889): cited project ecosystem and competitive-intelligence findings.
/// New table and index only; safe to run repeatedly on every database.
pub async fn migrate_v34_to_v35(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v34 -> v35 (project intelligence, #889)");
    apply_project_intel_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (35)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v35 (project intelligence)");
    Ok(())
}

/// Apply the per-call cost-ledger schema (v28, cost-transparency workstream):
/// `cost_ledger` — one append-only row per provider response — plus the O(1)
/// cost-rollup columns on `sessions`.
///
/// `cost_ledger` is high-frequency numeric telemetry (SUM / GROUP BY over
/// session, task, provider) and deliberately lives here in the session DB rather
/// than the activity journal, which is a low-frequency semantic-enrichment log.
/// Every money column is the output of the single canonical
/// [`crate::providers::canonical::cost_of`] function, so the ledger, the live
/// meter, and the verification digest can never disagree. `cost_tier` is a plain
/// TEXT (no `CHECK` constraint — validated in Rust via `CostTier`) to avoid the
/// widen-the-constraint-in-two-places footgun.
///
/// The `sessions` cost columns are an O(1) running rollup updated inside the
/// same transaction as each ledger append: `accumulated_cost_usd` (= Σ
/// `cost_usd`), `cost_usd` (the most recent turn), the cache-read/write token
/// accumulators, and `accumulated_cache_savings_usd` (the visible "cache saved"
/// trust signal). The `ALTER`s are PRAGMA-guarded so this is a no-op on fresh
/// installs (which get the columns from the `sessions` CREATE) and fills them in
/// on existing DBs. Fully idempotent — safe on every boot and on fresh installs.
pub async fn apply_cost_ledger_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS cost_ledger (
            call_id            TEXT PRIMARY KEY,
            ts                 TEXT NOT NULL,
            session_id         TEXT NOT NULL,
            parent_session_id  TEXT,
            task_id            TEXT,
            goal_id            TEXT,
            subagent_id        TEXT,
            turn_index         INTEGER,
            provider           TEXT,
            model              TEXT,
            cost_tier          TEXT NOT NULL DEFAULT 'paid_api',
            is_chargeable      INTEGER NOT NULL DEFAULT 1,
            is_headless        INTEGER NOT NULL DEFAULT 0,
            input_tokens       INTEGER NOT NULL DEFAULT 0,
            output_tokens      INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            input_cost         REAL NOT NULL DEFAULT 0,
            output_cost        REAL NOT NULL DEFAULT 0,
            cache_read_cost    REAL NOT NULL DEFAULT 0,
            cache_write_cost   REAL NOT NULL DEFAULT 0,
            cost_usd           REAL NOT NULL DEFAULT 0,
            is_estimated       INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_cost_ledger_session ON cost_ledger(session_id, ts)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_cost_ledger_task ON cost_ledger(task_id, ts)")
        .execute(&mut *tx)
        .await?;
    // "What have I spent today, across everything?" has no session or task to
    // key on, so neither index above serves it and the query degrades to a full
    // scan of every call ever made. The Build meter asks it once per turn while
    // the user codes, which is exactly the shape of the Automate tab's polling
    // storm (an unindexed `WHERE schedule_id = ?` on `sessions`, 1-3.7s a poll)
    // — cheap to prevent here, expensive to discover later.
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_cost_ledger_ts ON cost_ledger(ts)")
        .execute(&mut *tx)
        .await?;

    // PRAGMA-guarded rollup columns on `sessions` (idempotent ADD COLUMN).
    for (col, ddl) in [
        ("cost_usd", "ALTER TABLE sessions ADD COLUMN cost_usd REAL"),
        (
            "accumulated_cost_usd",
            "ALTER TABLE sessions ADD COLUMN accumulated_cost_usd REAL",
        ),
        (
            "accumulated_cache_read_tokens",
            "ALTER TABLE sessions ADD COLUMN accumulated_cache_read_tokens INTEGER",
        ),
        (
            "accumulated_cache_write_tokens",
            "ALTER TABLE sessions ADD COLUMN accumulated_cache_write_tokens INTEGER",
        ),
        (
            "accumulated_cache_savings_usd",
            "ALTER TABLE sessions ADD COLUMN accumulated_cache_savings_usd REAL",
        ),
    ] {
        let has_column: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = ?")
                .bind(col)
                .fetch_one(&mut *tx)
                .await?;
        if has_column == 0 {
            sqlx::query(ddl).execute(&mut *tx).await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Migrate an existing database to the cost-ledger schema (schema v28).
///
/// Additive (CREATE TABLE / INDEX IF NOT EXISTS + PRAGMA-guarded ADD COLUMN) and
/// base-version independent, so it applies cleanly over any earlier base. Records
/// v28 in `schema_version`.
pub async fn migrate_v27_to_v28(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v27 -> v28 (per-call cost ledger)");

    apply_cost_ledger_schema(pool).await?;

    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (28)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v28 (per-call cost ledger)");

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
            ('goal_approve_standard', 1, 'Review->Complete requires a recorded decision (agent policy or the user)'),
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

/// Ensure the recognition replay columns + v22 feed table exist,
/// **independent of the global schema version**. Idempotent (PRAGMA-guarded
/// `ADD COLUMN` + `CREATE TABLE IF NOT EXISTS`) and safe on every boot.
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
        (
            "injected_memory_ids",
            "ALTER TABLE recognition_events ADD COLUMN injected_memory_ids TEXT",
        ),
        (
            "injected_memory_ids_source",
            "ALTER TABLE recognition_events ADD COLUMN injected_memory_ids_source TEXT",
        ),
        (
            "citation_checked_at",
            "ALTER TABLE recognition_events ADD COLUMN citation_checked_at TEXT",
        ),
        (
            "outcome_label",
            "ALTER TABLE recognition_events ADD COLUMN outcome_label TEXT",
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

    // A short-lived pre-release repair incorrectly treated the historical
    // default cited_memory_ids='[]' as a measured no-match. Without the
    // detector-ran marker an ignored label is ungrounded, so clear it.
    let cleared = sqlx::query(
        "UPDATE recognition_events
            SET outcome_label = NULL
          WHERE outcome_label = 'ignored'
            AND citation_checked_at IS NULL",
    )
    .execute(pool)
    .await?
    .rows_affected();
    if cleared > 0 {
        info!(
            "recognition schema repair: cleared {cleared} unmeasured historical ignored label(s)"
        );
    }

    // Historical injection is exactly reconstructible: the production filter
    // was pure and stable for the whole data window (score >= 0.7, rank order,
    // top 3), and both rank and signal_score were persisted for every member.
    let reconstructed = sqlx::query(
        "UPDATE recognition_events
            SET injected_memory_ids = COALESCE((
                    SELECT json_group_array(memory_id)
                      FROM (
                        SELECT memory_id
                          FROM recognition_set_members members
                         WHERE members.retrieval_id = recognition_events.retrieval_id
                           AND members.signal_score >= 0.7
                         ORDER BY members.rank
                         LIMIT 3
                      )
                ), '[]'),
                injected_memory_ids_source = 'reconstructed'
          WHERE injected_memory_ids IS NULL
            AND strategy = 'cascade'",
    )
    .execute(pool)
    .await?
    .rows_affected();
    if reconstructed > 0 {
        info!(
            "recognition schema repair: reconstructed exact injected sets for {reconstructed} historical event(s)"
        );
    }
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
    // headline/detail are amendment A1: two separate REQUIRED text fields.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decisions (
            id            TEXT PRIMARY KEY,
            kind          TEXT NOT NULL CHECK (kind IN
                            ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','project_intel_proposal','file_to_project','model_upgrade','tool_approval','session_gate','capability_gap','regression_proposal','malformed')),
            goal_id       TEXT REFERENCES cards(id) ON DELETE SET NULL,
            project_id    TEXT REFERENCES projects(id) ON DELETE CASCADE,
            tier          INTEGER NOT NULL CHECK (tier IN (0,1,2)),
            headline      TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
            detail        TEXT NOT NULL CHECK (length(detail) > 0),
            payload_json  TEXT NOT NULL DEFAULT '{}',
            rank          REAL,
            status        TEXT NOT NULL DEFAULT 'open'
                          CHECK (status IN ('open','answered','expired','superseded')),
            answer        TEXT CHECK (answer IN ('approve','reject','choice','input','edit')),
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
    // Inbox), 'enrichment_proposal' (the Enricher, #495 slice 4), 'tool_approval'
    // (needs-approval tool calls routed to the inbox), 'file_to_project'
    // (the file_to_project tool's review-gated note+people proposal), and
    // 'session_gate' (supervised-terminal gates, S3 #429), AND the `answer`
    // CHECK to admit 'edit' (approve-with-edits / edit-as-training).
    // SQLite cannot ALTER a CHECK, so an older table is rebuilt in place.
    // FK-safe: nothing references `decisions` via a foreign key (decision_audit
    // stores a plain TEXT id; the complete-guard trigger resolves by name after
    // the rename). Gated on the widened constraints' marker tokens, so it runs
    // at most once per widening: a table missing 'enrichment_proposal', the
    // 'edit' answer value, 'tool_approval', 'file_to_project', OR 'session_gate'
    // is rebuilt to the current DDL, which widens all of them together (an older
    // DB only ever holds legacy values, all valid under the new CHECK, so the
    // row copy is lossless).
    let decisions_ddl: Option<String> =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'")
            .fetch_optional(&mut *tx)
            .await?;
    if decisions_ddl
        .map(|sql| {
            !sql.contains("enrichment_proposal")
                || !sql.contains("project_intel_proposal")
                || !sql.contains("'edit'")
                || !sql.contains("tool_approval")
                || !sql.contains("file_to_project")
                || !sql.contains("session_gate")
                || !sql.contains("model_upgrade")
                || !sql.contains("capability_gap")
                || !sql.contains("regression_proposal")
        })
        .unwrap_or(false)
    {
        info!("Widening decisions kind/answer CHECK constraints (in-place rebuild)");
        // Indexes on the old table are dropped with it; recreated below.
        sqlx::query("DROP INDEX IF EXISTS idx_decisions_open")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DROP INDEX IF EXISTS idx_decisions_goal")
            .execute(&mut *tx)
            .await?;
        // Drop the goal-complete guard trigger before the rename. It references
        // `decisions` in its WHEN subquery, and modern SQLite (bundled via
        // rusqlite since #713) re-parses every trigger during
        // `ALTER TABLE decisions_new RENAME TO decisions`; while `decisions` is
        // transiently dropped that parse fails with "no such table: decisions"
        // and aborts the widening. It is recreated (CREATE TRIGGER IF NOT EXISTS)
        // below, inside this same transaction. Existing DBs already carry this
        // trigger (co-shipped with the enrichment_proposal widening), so the
        // 'edit' widening MUST clear it first or it cannot upgrade them.
        sqlx::query("DROP TRIGGER IF EXISTS trg_goal_complete_guard")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE TABLE decisions_new (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL CHECK (kind IN
                                ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','project_intel_proposal','file_to_project','model_upgrade','tool_approval','session_gate','capability_gap','regression_proposal','malformed')),
                goal_id       TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id    TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier          INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline      TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail        TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json  TEXT NOT NULL DEFAULT '{}',
                rank          REAL,
                status        TEXT NOT NULL DEFAULT 'open'
                              CHECK (status IN ('open','answered','expired','superseded')),
                answer        TEXT CHECK (answer IN ('approve','reject','choice','input','edit')),
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
            principal       TEXT,
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
    // The `cc_*` classes are the S4 (#430) supervised-CC-gate policy — kept in
    // sync with `platform_extensions::gate_classifier::SEEDED_CLASSES` and
    // reconciled onto existing DBs by `migrate_v31_to_v32`. The `repo_*`
    // classes are the Steward git-health lane (Tier 2, user-only) —
    // reconciled onto existing DBs by `migrate_v40_to_v41`.
    sqlx::query(
        "INSERT OR IGNORE INTO risk_policy (action_class, tier, rationale) VALUES
            ('goal_ready', 0, 'Triage->Ready promotion is reversible'),
            ('goal_dispatch', 0, 'Dispatching a ready goal to a worker is reversible'),
            ('goal_review', 0, 'Worker reporting completion is informational'),
            ('goal_complete_confined', 0, 'Completion check passed, diff confined to declared paths, reversible class'),
            ('goal_approve_standard', 1, 'Review->Complete requires a recorded decision (agent policy or the user)'),
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
            ('policy_edit', 2, 'Changes to this table are themselves Tier 2'),
            ('cc_read_only', 0, 'Supervised CC read-only tool (Read/Glob/Grep/LS/NotebookRead/BashOutput/TodoWrite) — no effect outside the session'),
            ('cc_workspace_edit', 1, 'Supervised CC file edit (Write/Edit/MultiEdit/NotebookEdit) — confined, git-reversible; recorded decision'),
            ('cc_shell', 2, 'Supervised CC shell (Bash/KillBash) — arbitrary command surface; user-only'),
            ('repo_worktree_reap', 2, 'Removes a merged, clean worktree directory — user-only'),
            ('repo_branch_delete', 2, 'Deletes a local branch merged into the trunk — user-only')",
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

/// Add queryable credential attribution to Decision-Inbox audit rows.
///
/// Existing rows remain NULL and retain their legacy hash format. New
/// authenticated answer rows hash the principal into `row_hash`.
pub async fn apply_decision_audit_principal_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('decision_audit')")
            .fetch_all(pool)
            .await?;
    if !columns.iter().any(|column| column == "principal") {
        sqlx::query("ALTER TABLE decision_audit ADD COLUMN principal TEXT")
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_decision_audit_principal \
         ON decision_audit(principal) WHERE principal IS NOT NULL",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Migrate an existing database to queryable Decision-Inbox principal
/// attribution (schema v36).
pub async fn migrate_v35_to_v36(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v35 -> v36 (decision audit principal)");
    apply_decision_audit_principal_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (36)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v36 (decision audit principal)");
    Ok(())
}

/// Apply the durable-effect outbox schema (v37): the `effect_outbox` table +
/// its drain index. Shared by `migrate_v36_to_v37` (existing DBs) and
/// `init_spectral_db` (fresh installs) so a brand-new database gets the table
/// on its first boot — not only after a later upgrade pass. Fully idempotent
/// (IF NOT EXISTS), so it is safe on every boot and on fresh installs.
pub async fn apply_effect_outbox_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS effect_outbox (
            id              TEXT PRIMARY KEY,
            claim_key       TEXT NOT NULL UNIQUE,
            kind            TEXT NOT NULL,
            decision_id     TEXT,
            payload_json    TEXT NOT NULL DEFAULT '{}',
            status          TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','running','applied','failed','dead')),
            attempts        INTEGER NOT NULL DEFAULT 0,
            max_attempts    INTEGER NOT NULL DEFAULT 5,
            last_error      TEXT,
            next_attempt_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_effect_outbox_drain
         ON effect_outbox(status, next_attempt_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// v37: durable outbox for effects authorized by answered decisions.
/// New table and index only; safe to run repeatedly on every database.
pub async fn migrate_v36_to_v37(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v36 -> v37 (durable effect outbox)");
    apply_effect_outbox_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (37)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v37 (durable effect outbox)");
    Ok(())
}

/// Apply the first-party analytics schema (v38): raw web-analytics events
/// ingested by the daemon's own collector endpoint (#23 — no third-party
/// analytics dependency). Shared by `migrate_v37_to_v38` (existing DBs) and
/// `init_spectral_db` (fresh installs). Fully idempotent (IF NOT EXISTS).
pub async fn apply_analytics_events_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS analytics_events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id   TEXT NOT NULL,
            kind         TEXT NOT NULL DEFAULT 'pageview'
                         CHECK (kind IN ('pageview','event')),
            path         TEXT NOT NULL DEFAULT '/',
            referrer     TEXT,
            name         TEXT,
            visitor_hash TEXT,
            created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_analytics_events_project_time
         ON analytics_events(project_id, created_at)",
    )
    .execute(pool)
    .await?;

    // Drain ingest (v39): events pulled from a site's own relay carry the
    // relay's row id. Without it a retried or overlapping drain re-inserts the
    // same traffic and inflates every count — the UNIQUE index makes
    // `INSERT OR IGNORE` exactly-once. NULL for locally-beaconed rows, and
    // SQLite treats NULLs as distinct, so direct collection is unaffected.
    let has_source: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('analytics_events') WHERE name = 'source_event_id'",
    )
    .fetch_one(pool)
    .await?;
    if has_source == 0 {
        sqlx::query("ALTER TABLE analytics_events ADD COLUMN source_event_id TEXT")
            .execute(pool)
            .await?;
    }
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_analytics_events_source
         ON analytics_events(project_id, source_event_id)",
    )
    .execute(pool)
    .await?;

    // v40: the dimensions that make the data answerable rather than decorative.
    // Every one is additive and nullable (or defaulted), so old rows stay valid
    // and a database at any prior version reaches the same shape.
    //
    // properties — event payloads. Until this existed `window.permagent.event`
    //   took a NAME only, so migrating a site off PostHog (which the install
    //   brief mandates) silently destroyed every property on every call site.
    //   JSON text rather than a column per key: the shape is the site's, not
    //   ours.
    // is_bot    — classified server-side at collect time from the user agent.
    //   An SEO site's crawler traffic otherwise drowns the real numbers.
    // session_id — first-party, sessionStorage, no cookie. visitor_hash rotates
    //   daily, so without this there is no bounce rate, pages per session, or
    //   entry/exit page.
    // utm_*     — an ALLOWLIST, never the whole query string, which would drag
    //   PII in. Campaign attribution is impossible without it.
    // country   — resolved at collect time and stored alone; the IP is never
    //   persisted, so the no-IP guarantee holds.
    for (column, ddl) in [
        ("properties", "properties TEXT"),
        ("is_bot", "is_bot INTEGER NOT NULL DEFAULT 0"),
        ("session_id", "session_id TEXT"),
        ("utm_source", "utm_source TEXT"),
        ("utm_medium", "utm_medium TEXT"),
        ("utm_campaign", "utm_campaign TEXT"),
        ("country", "country TEXT"),
    ] {
        // `column`/`ddl` come only from the fixed literal array above.
        let present: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM pragma_table_info('analytics_events') WHERE name = '{column}'"
        )))
        .fetch_one(pool)
        .await?;
        if present == 0 {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE analytics_events ADD COLUMN {ddl}"
            )))
            .execute(pool)
            .await?;
        }
    }
    // Bots are excluded from every default figure, so the filter belongs in the
    // index the dashboard actually queries through.
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_analytics_events_project_bot_time
         ON analytics_events(project_id, is_bot, created_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Apply the durable growth action + outcome schema (v42, ratified in
/// docs/proposals/grow-action-outcome-loop.md). Shared by `migrate_v41_to_v42`
/// (existing DBs) and `init_spectral_db` (fresh installs) so a brand-new
/// database gets the tables on its first boot — the `version < N` ladder in
/// `SessionStorage::pool` only runs under `is_schema_initialized`
/// (session_manager.rs:799), so a fresh DB would otherwise not see them until
/// the second boot. Fully idempotent (IF NOT EXISTS).
pub async fn apply_growth_actions_schema(pool: &Pool<Sqlite>) -> Result<()> {
    // `growth_actions` is created FIRST: `growth_action_outcomes.action_id`
    // REFERENCES it (proposal "Schema" section) and the pool is opened with
    // `.foreign_keys(true)` (session_manager.rs:750), the same ordering
    // constraint that puts apply_incidents_schema before apply_lessons_schema.
    //
    // UNIQUE(project_id, fingerprint) is the whole point of the table: both
    // producers recompute their advice on every load, so without it the same
    // suggestion regenerated tomorrow would insert a duplicate card and orphan
    // the outcome rows already attached to yesterday's copy. fingerprint is
    // hash(project_id, title, recommendation).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS growth_actions (
            id             TEXT PRIMARY KEY,
            project_id     TEXT NOT NULL,
            fingerprint    TEXT NOT NULL,
            title          TEXT NOT NULL,
            recommendation TEXT NOT NULL,
            category       TEXT,
            artifact_kind  TEXT,
            artifact       TEXT,
            target_metric  TEXT,
            target_dir     TEXT,
            baseline_json  TEXT,
            status         TEXT NOT NULL,
            verified_by    TEXT,
            verified_at    TEXT,
            verified_commit TEXT,
            verified_detail TEXT,
            created_at     TEXT NOT NULL,
            UNIQUE(project_id, fingerprint)
        )",
    )
    .execute(pool)
    .await?;

    // The Grow board lists one project's cards split by status, so that is the
    // index — mirrors idx_project_intel_project.
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_growth_actions_project
         ON growth_actions(project_id, status)",
    )
    .execute(pool)
    .await?;

    // PERSISTED VERIFICATION EVIDENCE. `verified_by` records only WHICH
    // strategy confirmed the change ("git"), never WHAT it found, so the
    // commit that earned the verification lived for exactly one HTTP response
    // — in the `checks[].detail` prose of the verify reply — and was gone on
    // the next board load. A completed action that cannot name the commit it
    // shipped in is a claim without a receipt, and re-running the check later
    // cannot recover it: `verify_git` searches `--since=created_at` and would
    // happily name a DIFFERENT, later commit.
    //
    // `verified_commit` is the full sha (the UI shortens it); `verified_detail`
    // is the passing check's own sentence, stored verbatim so the card shows
    // the evidence the check actually gave rather than a re-derived summary.
    // Both are nullable: every row verified before this column existed, and
    // every non-git strategy, legitimately has no commit.
    //
    // PRAGMA-guarded ADD COLUMN, applied here rather than in a version-gated
    // migration because `apply_growth_actions_schema` already runs on EVERY
    // boot (session_manager.rs) — the same version-independent safety net the
    // recognition columns use, and the reason a stamped-but-missing column
    // cannot strand the Grow board.
    for (col, ddl) in [
        (
            "verified_commit",
            "ALTER TABLE growth_actions ADD COLUMN verified_commit TEXT",
        ),
        (
            "verified_detail",
            "ALTER TABLE growth_actions ADD COLUMN verified_detail TEXT",
        ),
    ] {
        let has_column: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('growth_actions') WHERE name = ?",
        )
        .bind(col)
        .fetch_one(pool)
        .await?;
        if has_column == 0 {
            sqlx::query(ddl).execute(pool).await?;
        }
    }

    // PRIMARY KEY(action_id, window_days): the proposal measures the same
    // action over several whole-week windows (7/14/28) and a nightly job
    // re-evaluates open ones, so a re-judge must overwrite that window's row
    // rather than append a second verdict for it.
    //
    // `rationale` is NOT NULL by design — the proposal requires a verdict to
    // always carry its one-sentence why, including for `inconclusive`, which is
    // the expected outcome at MIN_PAGEVIEWS=20 traffic
    // (growth_actions.rs:147), not a failure.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS growth_action_outcomes (
            action_id      TEXT NOT NULL REFERENCES growth_actions(id),
            window_days    INTEGER NOT NULL,
            before_json    TEXT NOT NULL,
            after_json     TEXT NOT NULL,
            delta_pct      REAL,
            verdict        TEXT NOT NULL,
            rationale      TEXT NOT NULL,
            confounders    TEXT,
            judged_at      TEXT NOT NULL,
            PRIMARY KEY(action_id, window_days)
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// v42: durable growth actions + pre-registered outcomes. New tables and index
/// only; safe to run repeatedly on every database.
pub async fn migrate_v41_to_v42(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v41 -> v42 (growth actions + outcomes)");
    apply_growth_actions_schema(pool).await?;
    // Hardcoded literal, never SPECTRAL_SCHEMA_VERSION: that const is the
    // fresh-init base stamp (14), not "latest" — see its doc comment.
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (42)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v42 (growth actions + outcomes)");
    Ok(())
}

/// v40: analytics dimensions — event `properties`, `is_bot`, `session_id`,
/// `utm_*` and `country`. All additive (PRAGMA-guarded ADD COLUMN); the work is
/// in `apply_analytics_events_schema`, which runs on EVERY boot, so a database
/// at any prior version converges without depending on this migration firing.
pub async fn migrate_v39_to_v40(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v39 -> v40 (analytics dimensions)");
    apply_analytics_events_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (40)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v40 (analytics dimensions)");
    Ok(())
}

/// v41: reconcile the Steward git-health `risk_policy` classes onto existing
/// DBs (`repo_worktree_reap`, `repo_branch_delete` — both Tier 2, user-only,
/// so henry-policy can never auto-approve a deletion). Same posture as the v32
/// reconcile: `INSERT OR IGNORE` so any user tier customization on an
/// already-present row survives; purely additive to a free-text-PK table and
/// base-independent. Records v41 in `schema_version`.
pub async fn migrate_v40_to_v41(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v40 -> v41 (seed Steward git-health risk_policy classes)");
    sqlx::query(
        "INSERT OR IGNORE INTO risk_policy (action_class, tier, rationale) VALUES
            ('repo_worktree_reap', 2, 'Removes a merged, clean worktree directory — user-only'),
            ('repo_branch_delete', 2, 'Deletes a local branch merged into the trunk — user-only')",
    )
    .execute(pool)
    .await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (41)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v41 (Steward git-health risk_policy classes seeded)");
    Ok(())
}

/// Apply the daemon control-plane auth-audit schema (schema v43):
/// `daemon_auth_audit` — one row per admitted consequential request and per
/// refused request on the daemon's HTTP control plane.
///
/// Why it exists: the daemon's bearer token lives in a `0600` file inside a
/// `0700` directory, which separates OTHER USERS from it but cannot separate
/// OTHER PROCESSES RUNNING AS THIS USER — Unix permissions have no sub-user
/// granularity. Same-user misuse therefore cannot be prevented, only made
/// visible; this table is that visibility. See
/// `docs/design/daemon-trust-boundary.md` and `crate::security::auth_audit`.
///
/// Append-only is enforced *at the DB* by `BEFORE UPDATE` / `BEFORE DELETE`
/// triggers, matching `egress_audit` and `decision_audit`. That stops rewriting
/// through SQL; it does not stop an attacker who can already run code as this
/// user from deleting the database file, and the design doc says so plainly.
///
/// Purely additive and base-version independent (`CREATE ... IF NOT EXISTS`),
/// so it applies cleanly over any earlier base and is safe on every boot.
pub async fn apply_daemon_auth_audit_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS daemon_auth_audit (
            id         TEXT PRIMARY KEY,
            ts         TEXT NOT NULL,
            outcome    TEXT NOT NULL,
            principal  TEXT,
            credential TEXT NOT NULL,
            class      TEXT NOT NULL,
            method     TEXT NOT NULL,
            path       TEXT NOT NULL,
            status     INTEGER,
            peer       TEXT
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_daemon_auth_audit_ts ON daemon_auth_audit(ts DESC)",
    )
    .execute(&mut *tx)
    .await?;
    // Denials are the rows an investigation starts from; give them their own
    // partial index so "what was refused, and when" stays cheap as the
    // admitted-mutation rows accumulate around them.
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_daemon_auth_audit_denied
            ON daemon_auth_audit(ts DESC) WHERE outcome = 'denied'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_daemon_auth_audit_no_update
            BEFORE UPDATE ON daemon_auth_audit
            BEGIN SELECT RAISE(ABORT, 'daemon_auth_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS trg_daemon_auth_audit_no_delete
            BEFORE DELETE ON daemon_auth_audit
            BEGIN SELECT RAISE(ABORT, 'daemon_auth_audit is append-only'); END",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// v43: the daemon control-plane auth audit. New table, indexes and triggers
/// only; additive and base-version independent, so it is safe on every
/// database and on every boot. Fresh installs get the same table from
/// `init_spectral_db`, which never reaches the migration ladder.
pub async fn migrate_v42_to_v43(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v42 -> v43 (daemon auth audit)");
    apply_daemon_auth_audit_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (43)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v43 (daemon auth audit)");
    Ok(())
}

/// v39: drain-ingest idempotency key on `analytics_events`. Additive
/// (PRAGMA-guarded ADD COLUMN + unique index); safe on every database.
pub async fn migrate_v38_to_v39(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v38 -> v39 (analytics drain idempotency)");
    apply_analytics_events_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (39)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v39 (analytics drain idempotency)");
    Ok(())
}

/// v38: first-party analytics events. New table and index only; safe to run
/// repeatedly on every database.
pub async fn migrate_v37_to_v38(pool: &Pool<Sqlite>) -> Result<()> {
    info!("Migrating Spectral schema v37 -> v38 (first-party analytics events)");
    apply_analytics_events_schema(pool).await?;
    sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (38)")
        .execute(pool)
        .await?;
    info!("Spectral schema migrated to v38 (first-party analytics events)");
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

    // SPECTRAL_SCHEMA_VERSION is the fresh-init BASE stamp, not "latest" (see its
    // doc comment). Every migrated database legitimately sits *above* it once the
    // hardcoded `version < N` chain in `SessionStorage::pool` has run, so `!=`
    // fired on every real install — a permanent false alarm. Only a DB *below*
    // the base is genuinely behind.
    if version < SPECTRAL_SCHEMA_VERSION {
        warn!(
            "Spectral schema below the fresh-init base: found v{}, expected at least v{}",
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
        sqlx::query(
            "CREATE TABLE recognition_events (
                retrieval_id TEXT PRIMARY KEY,
                query TEXT,
                strategy TEXT NOT NULL,
                outcome_polarity TEXT,
                cited_memory_ids TEXT NOT NULL DEFAULT '[]'
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE recognition_set_members (
                retrieval_id TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                signal_score REAL,
                rank INTEGER
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recognition_events (retrieval_id, query, strategy, outcome_polarity)
             VALUES ('ignored', 'q1', 'cascade', 'Positive'),
                    ('unknown', 'q2', 'cascade', NULL),
                    ('historical-negative', 'q3', 'cascade', 'Negative')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO recognition_set_members (retrieval_id, memory_id, signal_score, rank)
             VALUES ('ignored', 'm1', 0.9, 0), ('unknown', 'm2', 0.6, 0),
                    ('historical-negative', 'm3', 0.8, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let cols = |pool: Pool<Sqlite>| async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pragma_table_info('recognition_events') \
                 WHERE name IN ('recognition_verdict','familiarity','injected_memory_ids',
                                'injected_memory_ids_source','citation_checked_at','outcome_label')",
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
            6,
            "recognition columns applied even though schema_version is stamped at 23"
        );
        assert!(
            table_exists(&pool, "recognition_tool_events").await,
            "feed table also ensured"
        );

        // Simulate the faulty pre-release historical backfill. With no detector
        // marker, the next boot must clear it rather than preserve fabrication.
        sqlx::query("UPDATE recognition_events SET outcome_label = 'ignored' WHERE retrieval_id = 'ignored'")
            .execute(&pool)
            .await
            .unwrap();

        // Idempotent schema; corrective data pass clears the unmeasured label.
        apply_recognition_v22_columns(&pool).await.unwrap();
        assert_eq!(cols(pool.clone()).await, 6, "idempotent on re-run");

        let labels: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT retrieval_id, outcome_label FROM recognition_events ORDER BY retrieval_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            labels,
            vec![
                ("historical-negative".into(), None),
                ("ignored".into(), None),
                ("unknown".into(), None),
            ],
            "historical labels remain unmeasured"
        );

        let injections: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT retrieval_id, injected_memory_ids, injected_memory_ids_source
               FROM recognition_events ORDER BY retrieval_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            injections,
            vec![
                (
                    "historical-negative".into(),
                    "[\"m3\"]".into(),
                    Some("reconstructed".into())
                ),
                (
                    "ignored".into(),
                    "[\"m1\"]".into(),
                    Some("reconstructed".into())
                ),
                ("unknown".into(), "[]".into(), Some("reconstructed".into())),
            ],
            "historical injection is replayed from rank, score floor, and top-K"
        );
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
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT name FROM sqlite_master WHERE type='table' AND name='person_meetings')",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        );
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

    /// #595: `projects.graph_entity_id` is present on fresh installs (base
    /// CREATE) and applied by column-existence on older DBs via the
    /// version-independent `apply_project_graph_entity_column` (the
    /// `apply_skill_path_column` precedent). Idempotent both ways.
    #[tokio::test]
    async fn projects_graph_entity_column_applied_and_idempotent() {
        async fn has_col(pool: &Pool<Sqlite>) -> i64 {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'graph_entity_id'",
            )
            .fetch_one(pool)
            .await
            .unwrap()
        }

        // Fresh install: column present from the base CREATE; the guarded
        // apply is a clean no-op over it.
        let fresh = mem_pool().await;
        init_spectral_db(&fresh).await.unwrap();
        assert_eq!(has_col(&fresh).await, 1);
        apply_project_graph_entity_column(&fresh).await.unwrap();
        assert_eq!(has_col(&fresh).await, 1);

        // Old DB: a projects table WITHOUT the column gains it, idempotently.
        let old = mem_pool().await;
        sqlx::query(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, user_id TEXT NOT NULL DEFAULT 'default',
                 slug TEXT NOT NULL, name TEXT NOT NULL)",
        )
        .execute(&old)
        .await
        .unwrap();
        assert_eq!(has_col(&old).await, 0);
        apply_project_graph_entity_column(&old).await.unwrap();
        assert_eq!(has_col(&old).await, 1);
        apply_project_graph_entity_column(&old).await.unwrap();
        assert_eq!(has_col(&old).await, 1);
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

    /// migrate_v29_to_v30 (#250) backfills the Failed lifecycle column on a
    /// pre-existing goal board, stamps v30, and is idempotent.
    #[tokio::test]
    async fn migrate_v29_to_v30_backfills_failed_and_stamps() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Seed the lifecycle columns, then delete the failed one to simulate a
        // board seeded before #250, and rewind the recorded version to 29.
        crate::cards::seed_goal_columns(&pool, crate::projects::PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        sqlx::query("DELETE FROM board_columns WHERE state_binding = 'failed'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (29)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v29_to_v30(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 30);
        let failed: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM board_columns WHERE state_binding = 'failed'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(failed, 1, "failed column backfilled exactly once");

        // Idempotent: a second run adds nothing and does not error.
        migrate_v29_to_v30(&pool).await.unwrap();
        let failed_again: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM board_columns WHERE state_binding = 'failed'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(failed_again, 1, "no duplicate failed column");
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
                "cancelled",
                "failed"
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
        assert_eq!(state_count, 7, "no duplicate lifecycle columns on re-run");
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

    /// migrate_v31_to_v32 (#430, S4): a pre-S4 DB has no `cc_*` risk_policy
    /// classes, so every supervised-CC gate fails closed to Tier 2. After v32
    /// the three classes exist at their intended tiers, a user customization on
    /// one is preserved, the fail-closed sentinel stays unseeded, and a re-run
    /// changes nothing.
    #[tokio::test]
    async fn migrate_v31_to_v32_seeds_supervised_cc_gate_classes() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Simulate a pre-S4 DB: drop the fresh-install cc_* seed, stamp v31.
        sqlx::query("DELETE FROM risk_policy WHERE action_class LIKE 'cc_%'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (31)")
            .execute(&pool)
            .await
            .unwrap();

        // Precondition: the classes are genuinely absent (would fail closed to
        // Tier 2 via tier_for_action_class).
        let before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM risk_policy WHERE action_class LIKE 'cc_%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(before, 0, "cc_* classes absent on a pre-S4 DB");

        migrate_v31_to_v32(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 32);

        for (class, want) in [
            ("cc_read_only", 0),
            ("cc_workspace_edit", 1),
            ("cc_shell", 2),
        ] {
            let tier: i64 =
                sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = ?")
                    .bind(class)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(tier, want, "{class} seeded at tier {want}");
        }

        // The fail-closed sentinel is deliberately NEVER seeded.
        let unclassified: Option<i64> = sqlx::query_scalar(
            "SELECT tier FROM risk_policy WHERE action_class = 'cc_unclassified'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(
            unclassified, None,
            "cc_unclassified must stay unseeded so it fails closed to Tier 2"
        );

        // A user customization on a cc_* row survives a re-run (INSERT OR IGNORE).
        sqlx::query("UPDATE risk_policy SET tier = 2 WHERE action_class = 'cc_workspace_edit'")
            .execute(&pool)
            .await
            .unwrap();
        migrate_v31_to_v32(&pool).await.unwrap();
        let edit_tier: i64 = sqlx::query_scalar(
            "SELECT tier FROM risk_policy WHERE action_class = 'cc_workspace_edit'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            edit_tier, 2,
            "user-customized cc_workspace_edit tier preserved, not reset to seed"
        );

        // Idempotent: no duplicate rows after re-run.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM risk_policy WHERE action_class LIKE 'cc_%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 3, "exactly the three cc_* rows, no duplicates");
    }

    /// migrate_v40_to_v41: a pre-Steward-git-health DB has no `repo_*`
    /// risk_policy rows, so the classes fail closed to Tier 2 anyway — but the
    /// seed makes the user-only intent explicit and user-tunable. After v41
    /// both classes exist at Tier 2, a user customization survives a re-run,
    /// and no duplicates appear.
    #[tokio::test]
    async fn migrate_v40_to_v41_seeds_steward_git_health_classes() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Simulate a pre-lane DB: drop the fresh-install repo_* seed, stamp v40.
        sqlx::query("DELETE FROM risk_policy WHERE action_class LIKE 'repo_%'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (40)")
            .execute(&pool)
            .await
            .unwrap();

        let before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM risk_policy WHERE action_class LIKE 'repo_%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(before, 0, "repo_* classes absent on a pre-lane DB");

        migrate_v40_to_v41(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 41);

        for class in ["repo_worktree_reap", "repo_branch_delete"] {
            let tier: i64 =
                sqlx::query_scalar("SELECT tier FROM risk_policy WHERE action_class = ?")
                    .bind(class)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(tier, 2, "{class} must be Tier 2 (user-only)");
        }

        // A user customization survives a re-run (INSERT OR IGNORE posture).
        sqlx::query("UPDATE risk_policy SET tier = 1 WHERE action_class = 'repo_branch_delete'")
            .execute(&pool)
            .await
            .unwrap();
        migrate_v40_to_v41(&pool).await.unwrap();
        let tier: i64 = sqlx::query_scalar(
            "SELECT tier FROM risk_policy WHERE action_class = 'repo_branch_delete'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tier, 1, "user-customized tier preserved, not reset to seed");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM risk_policy WHERE action_class LIKE 'repo_%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 2, "exactly the two repo_* rows, no duplicates");
    }

    /// v33 (#66) is additive, seeds every existing user without overwriting a
    /// customization, and remains idempotent on repeated boots.
    #[tokio::test]
    async fn migrate_v32_to_v33_adds_idempotent_notification_routing_schema() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        sqlx::query("DROP TABLE notification_digest_entries")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE notification_preferences")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (32)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v32_to_v33(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 33);
        let defaults: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT push_min_severity, in_app_min_severity, digest_min_severity
             FROM notification_preferences WHERE user_id = 'default'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(defaults, (Some(3), Some(2), Some(1)));

        sqlx::query(
            "UPDATE notification_preferences SET push_min_severity = NULL
             WHERE user_id = 'default'",
        )
        .execute(&pool)
        .await
        .unwrap();
        apply_notification_routing_schema(&pool).await.unwrap();
        migrate_v32_to_v33(&pool).await.unwrap();
        let push: Option<i64> = sqlx::query_scalar(
            "SELECT push_min_severity FROM notification_preferences WHERE user_id = 'default'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(push, None, "re-run preserves the user's disabled channel");

        sqlx::query(
            "INSERT OR IGNORE INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'same-event', 1, 'task_completed', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO notification_digest_entries
             (user_id, source_event_id, severity, source_type, payload_json)
             VALUES ('default', 'same-event', 1, 'task_completed', '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_digest_entries WHERE source_event_id='same-event'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            queued, 1,
            "one source event is queued at most once per user"
        );
    }

    #[tokio::test]
    async fn migrate_v33_to_v34_adds_digest_date_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        sqlx::query("ALTER TABLE notification_preferences DROP COLUMN last_digest_delivery_date")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR REPLACE INTO schema_version (version) VALUES (33)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v33_to_v34(&pool).await.unwrap();
        migrate_v33_to_v34(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 34);
        let columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('notification_preferences')
             WHERE name = 'last_digest_delivery_date'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(columns, 1, "re-running v34 cannot duplicate the column");
    }

    #[tokio::test]
    async fn migrate_v34_to_v35_adds_project_intel_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        migrate_v34_to_v35(&pool).await.unwrap();
        migrate_v34_to_v35(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 35);
        assert!(object_exists(&pool, "project_intel").await);
        assert!(object_exists(&pool, "idx_project_intel_project").await);
        sqlx::query(
            "INSERT INTO project_intel
             (id, project_id, kind, name, note, source_url, created_at)
             VALUES ('intel-1','project-1','competitor','Rival',NULL,'https://rival.example','now')",
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    /// Fresh installs never run the `version < N` ladder, so the tables have
    /// to be there from `init_spectral_db`; existing installs reach them
    /// through the migration, twice over, without error or duplication.
    #[tokio::test]
    async fn migrate_v47_to_v48_adds_forecaster_tables_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        // Fresh-install path.
        assert!(object_exists(&pool, "forecaster_series").await);
        assert!(object_exists(&pool, "forecaster_points").await);
        assert!(object_exists(&pool, "forecaster_forecasts").await);
        assert!(object_exists(&pool, "forecaster_briefs").await);

        migrate_v47_to_v48(&pool).await.unwrap();
        migrate_v47_to_v48(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 48);
        assert!(object_exists(&pool, "idx_forecaster_series_subject").await);
        assert!(object_exists(&pool, "idx_forecaster_forecasts_series").await);

        sqlx::query(
            "INSERT INTO forecaster_series
             (id, project_id, intel_id, source_kind, subject, cadence, label, status, created_at)
             VALUES ('s1','p1','intel-1','npm','langchain','daily','langchain downloads','active','now')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // One subject, one series: a second propose for the same triple is a
        // constraint violation, not a forked history.
        let dup = sqlx::query(
            "INSERT INTO forecaster_series
             (id, project_id, source_kind, subject, cadence, status, created_at)
             VALUES ('s2','p1','npm','langchain','daily','proposed','now')",
        )
        .execute(&pool)
        .await;
        assert!(
            dup.is_err(),
            "duplicate (project, source_kind, subject) must be refused"
        );

        // Points are append-only and re-collection is a no-op, not a double.
        for _ in 0..2 {
            sqlx::query(
                "INSERT OR IGNORE INTO forecaster_points (series_id, ts, value)
                 VALUES ('s1','2026-08-01',10.0)",
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM forecaster_points")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "re-collecting the same timestamp inserts nothing");
    }

    #[tokio::test]
    async fn migrate_v36_to_v37_adds_effect_outbox_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        assert!(object_exists(&pool, "effect_outbox").await);
        assert!(object_exists(&pool, "idx_effect_outbox_drain").await);

        migrate_v36_to_v37(&pool).await.unwrap();
        migrate_v36_to_v37(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 37);
        assert!(object_exists(&pool, "effect_outbox").await);
        assert!(object_exists(&pool, "idx_effect_outbox_drain").await);
    }

    #[tokio::test]
    async fn migrate_v37_to_v38_adds_analytics_events_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        assert!(object_exists(&pool, "analytics_events").await);
        assert!(object_exists(&pool, "idx_analytics_events_project_time").await);

        migrate_v37_to_v38(&pool).await.unwrap();
        migrate_v37_to_v38(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 38);
        assert!(object_exists(&pool, "analytics_events").await);
        assert!(object_exists(&pool, "idx_analytics_events_project_time").await);
    }

    #[tokio::test]
    async fn migrate_v41_to_v42_adds_growth_actions_idempotently() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();
        // Fresh-install proof: init alone must yield the tables. The
        // `version < 42` ladder never runs on a fresh DB
        // (session_manager.rs:1084), so if this fails a first-boot install
        // spends its whole first session failing on `no such table`.
        assert!(object_exists(&pool, "growth_actions").await);
        assert!(object_exists(&pool, "growth_action_outcomes").await);
        assert!(object_exists(&pool, "idx_growth_actions_project").await);

        migrate_v41_to_v42(&pool).await.unwrap();
        migrate_v41_to_v42(&pool).await.unwrap();

        assert_eq!(current_version(&pool).await, 42);
        assert!(object_exists(&pool, "growth_actions").await);
        assert!(object_exists(&pool, "growth_action_outcomes").await);
        assert!(object_exists(&pool, "idx_growth_actions_project").await);

        // mem_pool() does not set `.foreign_keys(true)` the way
        // SessionStorage::pool does (session_manager.rs:750), and SQLite
        // defaults the pragma OFF, so enable it here or the FK assertion below
        // would pass vacuously.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();

        // The full ratified column list must actually accept a row.
        sqlx::query(
            "INSERT INTO growth_actions
             (id, project_id, fingerprint, title, recommendation, category,
              artifact_kind, artifact, target_metric, target_dir, baseline_json,
              status, verified_by, verified_at, created_at)
             VALUES ('act-1','project-1','fp-1','Add FAQ schema','Ship an FAQ block',
                     'seo','prompt','Write an FAQ...','sessions','up','{}',
                     'suggested',NULL,NULL,'2026-08-11T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO growth_action_outcomes
             (action_id, window_days, before_json, after_json, delta_pct,
              verdict, rationale, confounders, judged_at)
             VALUES ('act-1',28,'{}','{}',NULL,'inconclusive',
                     '30 pageviews/week; a change under ~40% is indistinguishable from variance.',
                     NULL,'2026-09-08T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // UNIQUE(project_id, fingerprint): regenerated advice must collide with
        // the existing row instead of duplicating the card.
        let dup = sqlx::query(
            "INSERT INTO growth_actions
             (id, project_id, fingerprint, title, recommendation, status, created_at)
             VALUES ('act-2','project-1','fp-1','Add FAQ schema','Ship an FAQ block',
                     'suggested','2026-08-12T00:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(dup.is_err(), "duplicate (project_id, fingerprint) accepted");

        // Same fingerprint under a different project is a different action.
        sqlx::query(
            "INSERT INTO growth_actions
             (id, project_id, fingerprint, title, recommendation, status, created_at)
             VALUES ('act-3','project-2','fp-1','Add FAQ schema','Ship an FAQ block',
                     'suggested','2026-08-12T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // PRIMARY KEY(action_id, window_days): one verdict per window, so the
        // nightly re-judge overwrites rather than appending a second verdict.
        let dup_window = sqlx::query(
            "INSERT INTO growth_action_outcomes
             (action_id, window_days, before_json, after_json, verdict, rationale, judged_at)
             VALUES ('act-1',28,'{}','{}','helped','sessions +34%','2026-09-09T00:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(
            dup_window.is_err(),
            "duplicate (action_id, window_days) accepted"
        );

        // FK is live: an outcome cannot dangle off an action that never existed.
        let orphan = sqlx::query(
            "INSERT INTO growth_action_outcomes
             (action_id, window_days, before_json, after_json, verdict, rationale, judged_at)
             VALUES ('no-such-action',7,'{}','{}','helped','x','2026-09-09T00:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(orphan.is_err(), "outcome accepted for unknown action_id");
    }

    /// migrate_v41_to_v42 is base-independent: it must reach the same shape and
    /// stamp v42 over any earlier recorded base, and be a no-op on a re-run.
    #[tokio::test]
    async fn migrate_v41_to_v42_is_base_independent() {
        for base in [38, 40, 41] {
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

            assert!(
                !object_exists(&pool, "growth_actions").await,
                "base v{base}: pre"
            );

            migrate_v41_to_v42(&pool).await.unwrap();

            assert!(
                object_exists(&pool, "growth_actions").await,
                "base v{base}: growth_actions"
            );
            assert!(
                object_exists(&pool, "growth_action_outcomes").await,
                "base v{base}: growth_action_outcomes"
            );
            assert_eq!(current_version(&pool).await, 42, "base v{base}: version");

            // Idempotent: a second run is a no-op, not an error.
            migrate_v41_to_v42(&pool).await.unwrap();
            assert_eq!(current_version(&pool).await, 42, "base v{base}: rerun");
        }
    }

    /// migrate_v42_to_v43 is base-independent: it must reach the same shape and
    /// stamp v43 over any earlier recorded base, and be a no-op on a re-run.
    #[tokio::test]
    async fn migrate_v42_to_v43_is_base_independent() {
        for base in [38, 41, 42] {
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

            assert!(
                !object_exists(&pool, "daemon_auth_audit").await,
                "base v{base}: pre"
            );

            migrate_v42_to_v43(&pool).await.unwrap();

            assert!(
                object_exists(&pool, "daemon_auth_audit").await,
                "base v{base}: daemon_auth_audit"
            );
            // The append-only guard is part of the schema, not a convention:
            // assert the triggers exist, not merely the table.
            assert!(
                object_exists(&pool, "trg_daemon_auth_audit_no_update").await,
                "base v{base}: no_update trigger"
            );
            assert!(
                object_exists(&pool, "trg_daemon_auth_audit_no_delete").await,
                "base v{base}: no_delete trigger"
            );
            assert_eq!(current_version(&pool).await, 43, "base v{base}: version");

            // Idempotent: a second run is a no-op, not an error.
            migrate_v42_to_v43(&pool).await.unwrap();
            assert_eq!(current_version(&pool).await, 43, "base v{base}: rerun");
        }
    }

    /// migrate_v43_to_v44 is base-independent: it must add person_meetings and
    /// stamp v44 over any earlier recorded base, and be a no-op on a re-run.
    #[tokio::test]
    async fn migrate_v43_to_v44_is_base_independent() {
        for base in [38, 41, 43] {
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

            assert!(
                !object_exists(&pool, "person_meetings").await,
                "base v{base}: pre"
            );

            migrate_v43_to_v44(&pool).await.unwrap();

            assert!(
                object_exists(&pool, "person_meetings").await,
                "base v{base}: person_meetings"
            );
            assert_eq!(current_version(&pool).await, 44, "base v{base}: version");

            migrate_v43_to_v44(&pool).await.unwrap();
            assert_eq!(current_version(&pool).await, 44, "base v{base}: rerun");
        }
    }

    /// migrate_v44_to_v45 is base-independent: it must add follow-up / project /
    /// calendar_uid columns over a v44 table and be a no-op on a re-run.
    #[tokio::test]
    async fn migrate_v44_to_v45_is_base_independent() {
        for base in [38, 43, 44] {
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

            migrate_v43_to_v44(&pool).await.unwrap();
            migrate_v44_to_v45(&pool).await.unwrap();

            let has_follow_up: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('person_meetings') WHERE name = 'follow_up_at'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(has_follow_up, 1, "base v{base}: follow_up_at");
            let has_project: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('person_meetings') WHERE name = 'project_id'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(has_project, 1, "base v{base}: project_id");
            assert_eq!(current_version(&pool).await, 45, "base v{base}: version");

            migrate_v44_to_v45(&pool).await.unwrap();
            assert_eq!(current_version(&pool).await, 45, "base v{base}: rerun");
        }
    }

    /// migrate_v45_to_v46 is base-independent: it must add the three ledger
    /// tables and be a no-op on a re-run.
    #[tokio::test]
    async fn migrate_v45_to_v46_is_base_independent() {
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
        sqlx::query("INSERT INTO schema_version (version) VALUES (45)")
            .execute(&pool)
            .await
            .unwrap();

        migrate_v45_to_v46(&pool).await.unwrap();
        for table in ["finance_watchlist", "finance_notes", "finance_positions"] {
            assert!(object_exists(&pool, table).await, "{table}");
        }
        assert_eq!(current_version(&pool).await, 46);
        migrate_v45_to_v46(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 46);
    }

    #[tokio::test]
    async fn migrate_v46_to_v47_is_base_independent() {
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
        sqlx::query("INSERT INTO schema_version (version) VALUES (46)")
            .execute(&pool)
            .await
            .unwrap();
        migrate_v46_to_v47(&pool).await.unwrap();
        for table in [
            "finance_watchlist",
            "finance_notes",
            "finance_positions",
            "finance_transactions",
            "finance_rsi_alerts",
            "finance_daily_picks",
        ] {
            assert!(object_exists(&pool, table).await, "{table}");
        }
        assert_eq!(current_version(&pool).await, 47);
        migrate_v46_to_v47(&pool).await.unwrap();
        assert_eq!(current_version(&pool).await, 47);
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

    /// approve-with-edits (`answer='edit'`) widening: an existing DB whose
    /// decisions table predates 'edit' — even one already carrying the widened
    /// `kind` CHECK — must be rebuilt in place by `apply_decision_inbox_schema`
    /// so `answer='edit'` is accepted, existing rows copied losslessly, and the
    /// rebuild idempotent on re-run. This is the migration Phase 0 missed: the
    /// `answer` column carries a CHECK constraint, so the reused-column design
    /// still needed a schema widening.
    #[tokio::test]
    async fn decisions_answer_check_widens_for_edit_in_place() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Downgrade decisions to a pre-'edit' shape: the widened `kind` CHECK
        // (already has enrichment_proposal) but the OLD `answer` CHECK — exactly
        // the case the broadened rebuild gate must still catch.
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE decisions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN
                    ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','malformed')),
                goal_id TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json TEXT NOT NULL DEFAULT '{}',
                rank REAL,
                status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','answered','expired','superseded')),
                answer TEXT CHECK (answer IN ('approve','reject','choice','input')),
                answer_note TEXT, answer_choice_id TEXT, answer_input TEXT,
                acted_by TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at TEXT,
                CHECK (status != 'answered'
                       OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A legacy answered row (NULL goal/project → no FK dependency).
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail, status, answer, acted_by, resolved_at)
             VALUES ('d-legacy', 'choice', 2, 'Legacy answered row', 'detail', 'answered',
                     'choice', 'jesse', '2026-01-01T00:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Precondition: the old CHECK rejects 'edit'.
        assert!(
            sqlx::query("UPDATE decisions SET answer='edit' WHERE id='d-legacy'")
                .execute(&pool)
                .await
                .is_err(),
            "old answer CHECK must reject 'edit' before the widening runs"
        );

        // The idempotent decision-inbox schema must rebuild to widen `answer`.
        apply_decision_inbox_schema(&pool).await.unwrap();

        // The constraint now admits 'edit'...
        let ddl: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ddl.contains("'edit'"),
            "answer CHECK must be widened to admit 'edit': {ddl}"
        );

        // ...the legacy row survived the rebuild losslessly...
        let survived: String =
            sqlx::query_scalar("SELECT headline FROM decisions WHERE id='d-legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(survived, "Legacy answered row");

        // ...and an edit answer is now accepted by the constraint.
        sqlx::query(
            "UPDATE decisions SET answer='edit', answer_input='revised' WHERE id='d-legacy'",
        )
        .execute(&pool)
        .await
        .expect("widened CHECK must accept answer='edit'");

        // Idempotent: a second run neither errors nor rebuilds away the edit.
        apply_decision_inbox_schema(&pool).await.unwrap();
        let still_edit: String =
            sqlx::query_scalar("SELECT answer FROM decisions WHERE id='d-legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            still_edit, "edit",
            "the edited row is stable across a re-run"
        );
    }

    /// `tool_approval` kind widening: an existing DB that already carries the
    /// enrichment_proposal + 'edit' widenings but whose `kind` CHECK predates
    /// 'tool_approval' must still be rebuilt by `apply_decision_inbox_schema`.
    /// This isolates the new marker token — the other two gate clauses are
    /// already satisfied, so only the tool_approval clause can trigger the
    /// rebuild — proving the gate was broadened (not just the DDL).
    #[tokio::test]
    async fn decisions_kind_check_widens_for_tool_approval_in_place() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Downgrade: widened `answer` (has 'edit') and enrichment_proposal in
        // `kind`, but NO 'tool_approval' — the exact case only the new gate
        // clause catches.
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE decisions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN
                    ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','malformed')),
                goal_id TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json TEXT NOT NULL DEFAULT '{}',
                rank REAL,
                status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','answered','expired','superseded')),
                answer TEXT CHECK (answer IN ('approve','reject','choice','input','edit')),
                answer_note TEXT, answer_choice_id TEXT, answer_input TEXT,
                acted_by TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at TEXT,
                CHECK (status != 'answered'
                       OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A legacy row (NULL goal/project → no FK dependency) to prove lossless copy.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail, status)
             VALUES ('d-legacy', 'choice', 2, 'Legacy open row', 'detail', 'open')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Precondition: the old kind CHECK rejects 'tool_approval'.
        assert!(
            sqlx::query(
                "INSERT INTO decisions (id, kind, tier, headline, detail)
                 VALUES ('d-ta', 'tool_approval', 2, 'x', 'y')",
            )
            .execute(&pool)
            .await
            .is_err(),
            "old kind CHECK must reject 'tool_approval' before the widening runs"
        );

        // The idempotent decision-inbox schema rebuilds to widen `kind`.
        apply_decision_inbox_schema(&pool).await.unwrap();

        let ddl: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ddl.contains("tool_approval"),
            "kind CHECK must be widened to admit 'tool_approval': {ddl}"
        );

        // Legacy row survived the rebuild losslessly...
        let survived: String =
            sqlx::query_scalar("SELECT headline FROM decisions WHERE id='d-legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(survived, "Legacy open row");

        // ...and a tool_approval row is now accepted by the constraint.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail)
             VALUES ('d-ta', 'tool_approval', 2, 'Approve tool', 'run ls')",
        )
        .execute(&pool)
        .await
        .expect("widened CHECK must accept kind='tool_approval'");

        // Idempotent: a second run neither errors nor rebuilds it away.
        apply_decision_inbox_schema(&pool).await.unwrap();
        let still_there: String = sqlx::query_scalar("SELECT kind FROM decisions WHERE id='d-ta'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(still_there, "tool_approval");
    }

    /// `file_to_project` kind widening (call-notes MVP 2A): an existing DB that
    /// already carries the enrichment_proposal + 'edit' + tool_approval +
    /// session_gate widenings but whose `kind` CHECK predates 'file_to_project'
    /// must still be rebuilt by `apply_decision_inbox_schema`. Isolates the new
    /// marker token — the other gate clauses are satisfied, so only the
    /// file_to_project clause can trigger the rebuild.
    #[tokio::test]
    async fn decisions_kind_check_widens_for_file_to_project_in_place() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Downgrade: everything widened EXCEPT 'file_to_project'.
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE decisions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN
                    ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','tool_approval','session_gate','malformed')),
                goal_id TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json TEXT NOT NULL DEFAULT '{}',
                rank REAL,
                status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','answered','expired','superseded')),
                answer TEXT CHECK (answer IN ('approve','reject','choice','input','edit')),
                answer_note TEXT, answer_choice_id TEXT, answer_input TEXT,
                acted_by TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at TEXT,
                CHECK (status != 'answered'
                       OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A legacy row (NULL goal/project → no FK dependency) to prove lossless copy.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail, status)
             VALUES ('d-legacy', 'choice', 2, 'Legacy open row', 'detail', 'open')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Precondition: the old kind CHECK rejects 'file_to_project'.
        assert!(
            sqlx::query(
                "INSERT INTO decisions (id, kind, tier, headline, detail)
                 VALUES ('d-ftp', 'file_to_project', 2, 'x', 'y')",
            )
            .execute(&pool)
            .await
            .is_err(),
            "old kind CHECK must reject 'file_to_project' before the widening runs"
        );

        // The idempotent decision-inbox schema rebuilds to widen `kind`.
        apply_decision_inbox_schema(&pool).await.unwrap();

        let ddl: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ddl.contains("file_to_project"),
            "kind CHECK must be widened to admit 'file_to_project': {ddl}"
        );

        // Legacy row survived the rebuild losslessly...
        let survived: String =
            sqlx::query_scalar("SELECT headline FROM decisions WHERE id='d-legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(survived, "Legacy open row");

        // ...and a file_to_project row is now accepted by the constraint.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail)
             VALUES ('d-ftp', 'file_to_project', 2, 'File an email', 'to Acme')",
        )
        .execute(&pool)
        .await
        .expect("widened CHECK must accept kind='file_to_project'");

        // Idempotent: a second run neither errors nor rebuilds it away.
        apply_decision_inbox_schema(&pool).await.unwrap();
        let still_there: String = sqlx::query_scalar("SELECT kind FROM decisions WHERE id='d-ftp'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(still_there, "file_to_project");
    }

    /// `session_gate` kind widening (S3, #429): an existing DB that already
    /// carries the enrichment_proposal + 'edit' + tool_approval + file_to_project
    /// widenings but whose `kind` CHECK predates 'session_gate' must still be
    /// rebuilt by `apply_decision_inbox_schema`. Isolates the new marker token —
    /// the other gate clauses are satisfied, so only the session_gate clause can
    /// trigger the rebuild — proving the gate was broadened (not just the DDL).
    #[tokio::test]
    async fn decisions_kind_check_widens_for_session_gate_in_place() {
        let pool = mem_pool().await;
        init_spectral_db(&pool).await.unwrap();

        // Downgrade: everything widened EXCEPT 'session_gate' in `kind` — the
        // exact shape a DB freshly upgraded through file_to_project has today.
        sqlx::query("DROP TABLE decisions")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE decisions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN
                    ('approve_review','unblock','choice','risk_gate','automation_proposal','enrichment_proposal','file_to_project','tool_approval','malformed')),
                goal_id TEXT REFERENCES cards(id) ON DELETE SET NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                tier INTEGER NOT NULL CHECK (tier IN (0,1,2)),
                headline TEXT NOT NULL CHECK (length(headline) > 0 AND length(headline) <= 80),
                detail TEXT NOT NULL CHECK (length(detail) > 0),
                payload_json TEXT NOT NULL DEFAULT '{}',
                rank REAL,
                status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','answered','expired','superseded')),
                answer TEXT CHECK (answer IN ('approve','reject','choice','input','edit')),
                answer_note TEXT, answer_choice_id TEXT, answer_input TEXT,
                acted_by TEXT CHECK (acted_by IN ('jesse','henry-policy','system')),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at TEXT,
                CHECK (status != 'answered'
                       OR (answer IS NOT NULL AND acted_by IS NOT NULL AND resolved_at IS NOT NULL))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A legacy row (NULL goal/project → no FK dependency) to prove lossless copy.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail, status)
             VALUES ('d-legacy-sg', 'tool_approval', 2, 'Legacy open row', 'detail', 'open')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Precondition: the old kind CHECK rejects 'session_gate'.
        assert!(
            sqlx::query(
                "INSERT INTO decisions (id, kind, tier, headline, detail)
                 VALUES ('d-sg', 'session_gate', 2, 'x', 'y')",
            )
            .execute(&pool)
            .await
            .is_err(),
            "old kind CHECK must reject 'session_gate' before the widening runs"
        );

        // The idempotent decision-inbox schema rebuilds to widen `kind`.
        apply_decision_inbox_schema(&pool).await.unwrap();

        let ddl: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='decisions'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ddl.contains("session_gate"),
            "kind CHECK must be widened to admit 'session_gate': {ddl}"
        );

        // Legacy row survived the rebuild losslessly...
        let survived: String =
            sqlx::query_scalar("SELECT headline FROM decisions WHERE id='d-legacy-sg'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(survived, "Legacy open row");

        // ...and a session_gate row is now accepted by the constraint.
        sqlx::query(
            "INSERT INTO decisions (id, kind, tier, headline, detail)
             VALUES ('d-sg', 'session_gate', 2, 'Terminal session gate', 'Write foo.txt')",
        )
        .execute(&pool)
        .await
        .expect("widened CHECK must accept kind='session_gate'");

        // Idempotent: a second run neither errors nor rebuilds it away.
        apply_decision_inbox_schema(&pool).await.unwrap();
        let still_there: String = sqlx::query_scalar("SELECT kind FROM decisions WHERE id='d-sg'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(still_there, "session_gate");
    }
}
