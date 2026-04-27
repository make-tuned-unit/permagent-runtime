# Permagent Phase 1 — Architecture Specification

**Date:** 2026-04-16
**Status:** COMPLETE — shipped 2026-04-27
**Based on:** Goose @ `block/goose` (upstream remote)

---

## Phase 1 Shipped

All Phase 1 deliverables are merged to `main`:

| Deliverable | Commit | Date |
|-------------|--------|------|
| Goose fork + Electron excision, crate renames | `727dca2` | 2026-04-21 |
| Config replacement, ~/.permagent/, keyring service | `e1a0730` | 2026-04-21 |
| Spectral schema, session storage, 17 tables, FTS | `7ded9f9` | 2026-04-22 |
| WebSocket event bus, /events, task logging | `3293caa` | 2026-04-22 |
| Command Center UI imported and pruned | `e53f33e`, `bce83380` | 2026-04-23 |
| Auto-skills detection (repetition_candidates view) | `5b2fe4b` | 2026-04-23 |
| CLI setup wizard, daemon lifecycle commands | `03edcbb` | 2026-04-24 |
| Gmail MCP extension with OAuth | `db7b30f` | 2026-04-24 |
| Embedded terminal (xterm.js + WebGL + portable-pty) | `c8d359e` | 2026-04-25 |
| Embedded browser (Tauri child webview) | `3780174` | 2026-04-25 |
| Workspaces system, three presets, schema v4 | `d5f3b02` | 2026-04-25 |
| Daemon hardening: config-driven port, plain HTTP, pool fixes | `77f1f23` | 2026-04-26 |
| Setup wizard: non-interactive daemon launch, plist fix | `38d792d` | 2026-04-26 |
| Browser embed, nav fix, terminal race fix | `62734d8`–`57fc6da` | 2026-04-27 |
| Capability inventory for Phase 2 planning | `44b088b98` | 2026-04-27 |

### Phase 1.5 punch list (known polish bugs, not blockers)

- **Keychain service name mismatch:** setup wizard stores keys under `"permagent"` but some provider code reads from `"goose"` service name. Needs audit of `keyring` callsites.
- **Browser focus stealing:** child webview can capture keyboard focus away from the chat input. Needs z-order or focus management fix.
- **World View sprite restoration:** WorldView is a placeholder. The `/api/agents` endpoint exists but returns empty. Data-driven agent sprites deferred to Phase 2.
- **Vite dev server port:** hardcoded to 5273 in tauri.conf.json, should match .env or be configurable.

---

## Spec Contradictions (Resolved Here)

Before diving in, these contradictions exist between the three canonical specs:

| # | Contradiction | Resolution |
|---|--------------|------------|
| 1 | SPEC_PHASE1_CANONICAL says "Tauri shell primitives" — **Goose uses Electron, not Tauri.** The Rust backend (`goosed`) is a standalone Axum HTTP server; Electron is the desktop shell. There are no Tauri primitives. | We excise the Electron desktop shell (`ui/desktop/`). The Rust `goosed` server is the daemon. This is actually *easier* than the specs assumed. |
| 2 | FORK_STRATEGY says "Desktop packaging (.dmg, keep)" under §1.1 but CANONICAL says ".dmg deferred to Phase 2" | Phase 1: no .dmg. Keep the packaging infrastructure for Phase 2. |
| 3 | FORK_STRATEGY §2.1 lists 4 detection triggers (repetition, complexity, user feedback, pattern). CANONICAL limits Phase 1 to repetition-only. GAP_FIXES agrees. | Phase 1: repetition-only (2x in 7 days). |
| 4 | CANONICAL lists Gmail as sole integration. GAP_FIXES adds Slack (write-capable) and notes Gmail-only is insufficient for auto-skills signal. | Phase 1: Gmail (read) only. Slack removed from Phase 1 scope — all chat goes through Permagent Command Center desktop app. Gmail registers as MCP extension. |
| 5 | FORK_STRATEGY §2.2 shows skill composition (conditionals, loops, skill-to-skill). CANONICAL says Phase 1 skills are linear sequences only. | Phase 1: linear skill sequences. Composition in Phase 2. |
| 6 | GAP_FIXES says daemon scheduling recovery is deferred but needed "before Phase 1." CANONICAL says no scheduled skills in Phase 1. | No scheduling in Phase 1. Recovery policy needed before Phase 2 ships scheduled skills. |

---

## A. Fork Plan — Seam Audit Results

### A.1 Session Storage Seam (REPLACE)

**What it is:** Goose stores conversation sessions in SQLite at `~/.local/share/goose/sessions/sessions.db`.

**Crate:** `goose` (core library)
**Module:** `crates/goose/src/session/session_manager.rs`
**Data structures:** `crates/goose/src/conversation/` (`mod.rs`, `message.rs`)

**Architecture:**
- `SessionStorage` — SQLite persistence layer (private, not behind a trait)
- `SessionManager` — public API wrapping `SessionStorage`
- Global singleton via `LazyLock<Arc<SessionStorage>>`
- Two tables: `sessions` (metadata) and `messages` (conversation content)
- Schema version 9, managed by internal migrations

**Key types:**
- `Session` — id, working_dir, name, session_type (User|Scheduled|SubAgent|Hidden|Terminal|Gateway|Acp), token counts, provider_name, model_config, goose_mode
- `Conversation` — newtype `Vec<Message>` with streaming merge on push
- `Message` — role (User|Assistant), content (`Vec<MessageContent>`), metadata (user_visible, agent_visible)
- `MessageContent` — enum: Text, Image, ToolRequest, ToolResponse, ToolConfirmationRequest, ActionRequired, FrontendToolRequest, Thinking, RedactedThinking, SystemNotification

**Critical finding: There is NO trait abstraction.** `SessionStorage` is a concrete struct with direct SQLite calls. This means Spectral cannot implement an interface — we must replace the module wholesale.

**Surgery plan:**
1. Create `crates/goose/src/session/spectral_storage.rs` implementing the same public API as `SessionManager`
2. Replace the `LazyLock<Arc<SessionStorage>>` singleton to point at Spectral
3. Keep the `Session`, `Conversation`, `Message` types unchanged — they're the API contract consumed by agents and the server
4. Drop the `sessions.db` schema entirely — Spectral's schema (Section B) replaces it

**Key function signatures to preserve:**
```rust
// These are the public methods consumed by goose-server and goose-cli:
pub async fn create_session(working_dir, name, session_type, goose_mode) -> Result<Session>
pub async fn get_session(id, include_messages) -> Result<Session>
pub async fn add_message(id, message) -> Result<()>
pub async fn replace_conversation(id, conversation) -> Result<()>
pub async fn list_sessions() -> Result<Vec<Session>>
pub async fn delete_session(id) -> Result<()>
pub async fn export_session(id) -> Result<String>
pub async fn import_session(json, session_type_override) -> Result<Session>
```

**Consumers:**
- `crates/goose-server/src/routes/session.rs` — HTTP CRUD endpoints
- `crates/goose-cli/src/session/` — CLI session builder
- `crates/goose/src/agents/agent.rs` — calls `add_message()` after LLM responses, `replace_conversation()` on compaction

### A.2 MCP Toolshed Registration (KEEP + EXTEND)

**What it is:** Multi-layered extension system unified through `McpClientTrait`.

**Key files:**
| File | Purpose |
|------|---------|
| `crates/goose/src/agents/mcp_client.rs` | `McpClientTrait` — the contract |
| `crates/goose/src/agents/extension_manager.rs` | `ExtensionManager` — orchestrates all extensions |
| `crates/goose/src/agents/extension.rs` | `ExtensionConfig` enum (7 variants) |
| `crates/goose/src/builtin_extension.rs` | Global registry for built-in MCP servers |
| `crates/goose-mcp/src/lib.rs` | Built-in MCP server implementations |
| `crates/goose/src/agents/platform_extensions/mod.rs` | Platform extension registry |
| `crates/goose/src/config/extensions.rs` | Extension config YAML handling |

**The trait contract:**
```rust
#[async_trait]
pub trait McpClientTrait: Send + Sync {
    async fn list_tools(&self, session_id: &str, ...) -> Result<ListToolsResult>;
    async fn call_tool(&self, ctx: &ToolCallContext, name: &str, arguments: ...) -> Result<CallToolResult>;
    fn get_info(&self) -> Option<&InitializeResult>;
    // + optional: list_resources, read_resource, list_prompts, get_prompt, subscribe
}
```

**Extension types (via `ExtensionConfig`):**
1. `Stdio` — CLI tool wrapper (spawns subprocess)
2. `Builtin` — Bundled Rust MCP servers (in-process via DuplexStream)
3. `Platform` — In-process platform extensions (developer, todo, summon, etc.)
4. `StreamableHttp` — HTTP MCP endpoints
5. `Frontend` — Frontend-provided tool definitions
6. `InlinePython` — Python MCP via uvx
7. `Sse` — Deprecated

**Built-in MCP servers (4):** autovisualiser, computercontroller, memory, tutorial
**Platform extensions (11):** analyze, todo, apps, chatrecall, extensionmanager, summon, summarize, code_execution, developer, orchestrator, tom

**Surgery plan:**
1. Keep `McpClientTrait`, `ExtensionManager`, and all extension infrastructure as-is
2. Gmail integration registers as `Stdio` or `StreamableHttp` extension (Slack removed from Phase 1 scope)
3. Replace the built-in `memory` MCP server with a Spectral-backed implementation
4. Skills engine registers its own platform extension for skill execution
5. Remove `computercontroller` and `autovisualiser` (not needed for Phase 1)

### A.3 Electron UI Entry (EXCISE)

**Critical correction:** Goose uses **Electron** (v41), not Tauri. The specs incorrectly reference "Tauri shell primitives."

**What to remove:**
| Path | Description | Lines |
|------|-------------|-------|
| `ui/desktop/` | Entire Electron app | ~75K+ TS/TSX |
| `ui/desktop/src/main.ts` | Electron main process (45 IPC handlers) |  |
| `ui/desktop/src/preload.ts` | IPC bridge (`window.electron` API) |  |
| `ui/desktop/src/renderer.tsx` | React entry point |  |
| `ui/desktop/src/components/` | 297 React components across 85 dirs |  |
| `ui/desktop/forge.config.ts` | Electron Forge packaging config |  |
| `ui/desktop/src/api/` | Auto-generated API client from OpenAPI |  |

**What the Electron shell does (that we replace):**
- 45 IPC commands for file ops, settings, system tray, notifications, goosed lifecycle
- Spawns `goosed` binary as child process with TLS
- Auto-generated REST client from OpenAPI spec (219KB)
- Deep-link protocol handler (`goose://`)

**What we keep from `goosed`:**
The `goosed` binary (`crates/goose-server/`) is the Axum HTTP server that runs independently. It has:
- 90+ REST endpoints across 19 route modules
- WebSocket support (session event streaming, MCP UI proxy)
- TLS with self-signed certificates
- Session, agent, config, recipe, schedule management

**Surgery plan:**
1. Delete `ui/desktop/` entirely
2. Keep `crates/goose-server/` (this becomes the daemon)
3. Strip Electron-specific server routes: `mcp_ui_proxy.rs`, `mcp_app_proxy.rs` (app launch/close), `dictation.rs`
4. Add WebSocket endpoint for Command Center at `ws://localhost:3000/events`
5. Remove TLS requirement (localhost only in Phase 1; add back for Phase 2 remote access)
6. Remove the secret key auth (`X-Secret-Key` header) — replace with localhost-only binding

### A.4 LLM Provider Trait (KEEP AS-IS)

**File:** `crates/goose/src/providers/base.rs`

**Two-tier design:**
1. `ProviderDef` trait — factory interface (metadata + `from_env()` constructor)
2. `Provider` trait — runtime interface (stream, complete, model discovery)

**Core `Provider` methods:**
```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn get_name(&self) -> &str;
    async fn stream(&self, model_config, session_id, system, messages, tools) -> Result<MessageStream>;
    async fn complete(&self, ...) -> Result<(Message, ProviderUsage)>;
    fn get_model_config(&self) -> ModelConfig;
    fn retry_config(&self) -> RetryConfig;
    async fn fetch_supported_models(&self) -> Result<Vec<String>>;
    fn supports_embeddings(&self) -> bool;
    // + OAuth, credential refresh, embeddings, session naming
}
```

**Registry:** `crates/goose/src/providers/init.rs` — `init_registry()` registers all providers
**Provider count:** 31+ (12 preferred, 19+ builtin, plus runtime declarative providers)

**Preferred providers:** Anthropic, ChatGPT Codex, Claude Code, Codex, Databricks, Gemini OAuth, Google, NanoGPT, Ollama, OpenAI, OpenRouter, Tetrate

**Builtin providers:** Azure, Bedrock (feature-gated), GCP Vertex, GitHub Copilot, LiteLLM, Snowflake, Venice, xAI, and 11 more

**No changes needed.** The CLI wizard (Section E) will prompt for provider selection and write the API key to Permagent config.

### A.5 Config System (REPLACE)

**Current Goose config:**
- Location: `~/Library/Application Support/Block/goose/config.yaml` (macOS)
- Module: `crates/goose/src/config/base.rs` (69KB)
- Secrets: System keyring (`KEYRING_SERVICE: "goose"`) with file fallback (`secrets.yaml`)
- Hot reload, file locking, backup/recovery, YAML validation
- Uses `etcetera` crate for platform-specific paths

**Key files:**
| File | Purpose |
|------|---------|
| `crates/goose/src/config/base.rs` | Main Config struct, YAML I/O, secret management |
| `crates/goose/src/config/paths.rs` | Path resolution via `etcetera` |
| `crates/goose/src/config/extensions.rs` | Extension config loading |
| `crates/goose/src/config/migrations.rs` | Config schema migrations |
| `crates/goose-server/src/routes/config_management.rs` | HTTP config API (955 lines) |

**Surgery plan:**
1. Change `Paths` to use `~/.permagent/` instead of `~/Library/Application Support/Block/goose/`
2. Config file becomes `~/.permagent/config.yaml`
3. Keep the config module's YAML parsing, hot reload, and file locking
4. Replace keyring service name: `"goose"` → `"permagent"`
5. Simplify: remove "Block" vendor prefix, custom distributions config, experiment flags
6. Keep the server-side config API for Command Center settings management

### A.6 Build System (SIMPLIFY)

**Current workspace structure (11 crates):**
| Crate | Keep/Remove | Notes |
|-------|-------------|-------|
| `goose` | KEEP | Core agent library |
| `goose-cli` | KEEP (rename) | → `permagent-cli` |
| `goose-server` | KEEP (rename) | → `permagent-daemon` |
| `goosed` | KEEP (rename) | → `permagentd` binary |
| `goose-mcp` | KEEP | MCP server implementations |
| `goose-acp` | REMOVE | Agent Client Protocol (not needed Phase 1) |
| `goose-acp-macros` | REMOVE | ACP derive macros |
| `goose-sdk` | REMOVE | ACP SDK |
| `goose-test-support` | KEEP | Test utilities |
| `goose-test` | KEEP | Integration tests |
| `bin/` | TRIM | Keep essential tools only |

**Build orchestration:** Justfile (500+ lines)
- `just release-binary` — builds release binaries
- `just run-ui` — builds binary + launches Electron (remove this)
- Binaries copied to `ui/desktop/src/bin/` for Electron embedding (remove this flow)

**Packaging (Phase 2 reference):**
- Electron Forge with ZIP/DEB/RPM/Flatpak makers
- macOS code signing + notarization (requires APPLE_TEAM_ID)
- Windows cross-compilation via Docker + mingw-w64
- CI/CD in `.github/workflows/` (bundle-desktop.yml, build-cli.yml, etc.)

**Phase 1 build:**
```bash
cargo build --release -p permagentd -p permagent-cli
# Output: target/release/permagentd, target/release/permagent
```

No Electron build. No .dmg. Just Rust binaries + Next.js dev server.

---

## B. Spectral Schema

All tables live in `~/.permagent/spectral/permagent.db`. Schema draws from brain.db patterns (temporal validity, FTS, knowledge graph) but is purpose-built for the Permagent runtime.

### B.0 Single-User Phase 1 Convention

Phase 1 is single-user, local-only. The schema includes `user_id` foreign keys on every table for Phase 2+ multi-user support, but Phase 1 does not exercise multi-user logic.

- **Phase 1 has exactly one user row**, created at `permagent setup` time with `id='default'`.
- **All Phase 1 inserts set `user_id='default'`.**
- Multi-user support is preserved in the schema for Phase 2+ but not exercised in Phase 1. No user selection UI, no user switching, no auth.
- The CLI wizard creates the default user row as **step 4** of setup (see Section E.1, "Initialize Spectral Memory").

```sql
-- ============================================================
-- USERS
-- ============================================================
CREATE TABLE users (
    id                TEXT PRIMARY KEY,
    display_name      TEXT NOT NULL,
    email             TEXT,
    provider_name     TEXT,              -- last-used LLM provider
    model_config_json TEXT,              -- serialized ModelConfig
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

-- ============================================================
-- SESSIONS (replaces Goose sessions + messages tables)
-- ============================================================
CREATE TABLE sessions (
    id                TEXT PRIMARY KEY,
    user_id           TEXT NOT NULL REFERENCES users(id),
    name              TEXT,
    working_dir       TEXT,
    session_type      TEXT NOT NULL DEFAULT 'user',  -- user|skill_execution|subagent
    provider_name     TEXT,
    model_config_json TEXT,
    total_tokens      INTEGER DEFAULT 0,
    input_tokens      INTEGER DEFAULT 0,
    output_tokens     INTEGER DEFAULT 0,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC);
CREATE INDEX idx_sessions_type ON sessions(session_type);

CREATE TABLE messages (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id        TEXT NOT NULL,
    session_id        TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role              TEXT NOT NULL,      -- user|assistant
    content_json      TEXT NOT NULL,      -- serialized Vec<MessageContent>
    metadata_json     TEXT,               -- serialized MessageMetadata
    tokens            INTEGER DEFAULT 0,
    created_timestamp INTEGER NOT NULL,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_messages_session ON messages(session_id);
CREATE INDEX idx_messages_timestamp ON messages(created_timestamp, id);
CREATE INDEX idx_messages_message_id ON messages(message_id);

-- ============================================================
-- MEMORIES (temporal knowledge graph, from brain.db patterns)
-- ============================================================
CREATE TABLE memories (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id),
    key             TEXT NOT NULL,
    content         TEXT NOT NULL,
    category        TEXT NOT NULL DEFAULT 'core',
    wing            TEXT,                -- high-level grouping
    hall            TEXT,                -- sub-grouping (fact, preference, observation)
    room            TEXT,                -- fine-grained topic
    embedding       BLOB,
    valid_from      TEXT,
    valid_until     TEXT,                -- NULL = current
    superseded_by   TEXT,
    confidence      REAL DEFAULT 1.0,
    signal_score    REAL DEFAULT 0.5,
    source_session  TEXT REFERENCES sessions(id),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_memories_user ON memories(user_id);
CREATE INDEX idx_memories_wing ON memories(wing);
CREATE INDEX idx_memories_hall ON memories(hall);
CREATE INDEX idx_memories_current ON memories(valid_until) WHERE valid_until IS NULL;
CREATE INDEX idx_memories_signal ON memories(signal_score DESC);

CREATE VIRTUAL TABLE memories_fts USING fts5(
    key, content, content=memories, content_rowid=rowid
);

-- FTS triggers (insert/update/delete) — same pattern as brain.db
CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, key, content)
    VALUES (new.rowid, new.key, new.content);
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content)
    VALUES ('delete', old.rowid, old.key, old.content);
END;

CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content)
    VALUES ('delete', old.rowid, old.key, old.content);
    INSERT INTO memories_fts(rowid, key, content)
    VALUES (new.rowid, new.key, new.content);
END;

-- ============================================================
-- KNOWLEDGE GRAPH (SPO triples with temporal validity)
-- ============================================================
CREATE TABLE knowledge_graph (
    id                TEXT PRIMARY KEY,
    subject           TEXT NOT NULL,
    predicate         TEXT NOT NULL,
    object            TEXT NOT NULL,
    valid_from        TEXT NOT NULL,
    valid_until       TEXT,
    source_memory_id  TEXT REFERENCES memories(id),
    confidence        REAL DEFAULT 1.0,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_kg_subject ON knowledge_graph(subject);
CREATE INDEX idx_kg_predicate ON knowledge_graph(predicate);
CREATE INDEX idx_kg_object ON knowledge_graph(object);
CREATE INDEX idx_kg_subject_predicate ON knowledge_graph(subject, predicate);
CREATE INDEX idx_kg_current ON knowledge_graph(valid_until) WHERE valid_until IS NULL;

CREATE VIRTUAL TABLE knowledge_graph_fts USING fts5(
    subject, predicate, object, content=knowledge_graph
);

-- ============================================================
-- TASKS (action log for auto-skills detection — from GAP_FIXES)
-- ============================================================
CREATE TABLE tasks (
    id                TEXT PRIMARY KEY,
    user_id           TEXT NOT NULL REFERENCES users(id),
    session_id        TEXT REFERENCES sessions(id),
    description       TEXT NOT NULL,
    tool_used         TEXT,              -- MCP tool name
    argument_shape_hash TEXT,            -- stable hash of (tool_used, sorted_arg_keys, arg_type_categories)
    steps_json        TEXT,              -- serialized action sequence
    status            TEXT NOT NULL DEFAULT 'pending',  -- pending|running|completed|failed
    input_json        TEXT,
    output_json       TEXT,
    error_message     TEXT,
    started_at        TEXT,
    completed_at      TEXT,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_tasks_user ON tasks(user_id);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_tool ON tasks(tool_used);
CREATE INDEX idx_tasks_completed ON tasks(completed_at DESC);
-- Repetition detection query index (matches on argument shape, not description):
CREATE INDEX idx_tasks_shape_repetition ON tasks(user_id, tool_used, argument_shape_hash, status, completed_at);

-- ============================================================
-- SKILLS
-- ============================================================
CREATE TABLE skills (
    id                TEXT PRIMARY KEY,
    user_id           TEXT NOT NULL REFERENCES users(id),
    name              TEXT NOT NULL,
    description       TEXT,
    definition_json   TEXT NOT NULL,     -- serialized skill steps
    trigger_type      TEXT NOT NULL DEFAULT 'manual',  -- manual|repetition
    trigger_value     TEXT,              -- JSON trigger config
    status            TEXT NOT NULL DEFAULT 'active',  -- active|paused|archived
    version           INTEGER NOT NULL DEFAULT 1,
    source_task_id    TEXT REFERENCES tasks(id),  -- which task spawned this skill
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_skills_user ON skills(user_id);
CREATE INDEX idx_skills_status ON skills(status);
CREATE INDEX idx_skills_trigger ON skills(trigger_type);

-- ============================================================
-- SKILL EXECUTIONS
-- ============================================================
CREATE TABLE skill_executions (
    id                TEXT PRIMARY KEY,
    skill_id          TEXT NOT NULL REFERENCES skills(id),
    user_id           TEXT NOT NULL REFERENCES users(id),
    session_id        TEXT REFERENCES sessions(id),
    status            TEXT NOT NULL DEFAULT 'running',  -- running|completed|failed
    input_json        TEXT,
    output_json       TEXT,
    error_message     TEXT,
    started_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    completed_at      TEXT
);

CREATE INDEX idx_skill_exec_skill ON skill_executions(skill_id);
CREATE INDEX idx_skill_exec_user ON skill_executions(user_id);
CREATE INDEX idx_skill_exec_status ON skill_executions(status);

-- ============================================================
-- SKILL TRIGGERS (Phase 1: only repetition triggers)
-- ============================================================
CREATE TABLE skill_triggers (
    id                TEXT PRIMARY KEY,
    skill_id          TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    trigger_type      TEXT NOT NULL,     -- repetition|manual
    trigger_config    TEXT,              -- JSON: {"threshold": 2, "window_days": 7}
    last_triggered_at TEXT,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_skill_triggers_skill ON skill_triggers(skill_id);

-- ============================================================
-- INTEGRATIONS (connection state, NOT secrets)
-- ============================================================
CREATE TABLE integrations (
    id                TEXT PRIMARY KEY,
    user_id           TEXT NOT NULL REFERENCES users(id),
    provider          TEXT NOT NULL,     -- gmail (slack removed from Phase 1)
    status            TEXT NOT NULL DEFAULT 'pending',  -- pending|connected|error|revoked
    scopes_json       TEXT,              -- granted OAuth scopes
    last_sync_at      TEXT,
    error_message     TEXT,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX idx_integrations_user ON integrations(user_id);
CREATE INDEX idx_integrations_provider ON integrations(provider);

-- ============================================================
-- CONVENIENCE VIEWS
-- ============================================================
CREATE VIEW current_memories AS
SELECT * FROM memories WHERE valid_until IS NULL ORDER BY created_at DESC;

CREATE VIEW current_knowledge AS
SELECT * FROM knowledge_graph WHERE valid_until IS NULL ORDER BY valid_from DESC;

CREATE VIEW recent_tasks AS
SELECT * FROM tasks WHERE status = 'completed' ORDER BY completed_at DESC LIMIT 100;

-- Repetition detection view (core of auto-skills):
-- Matches on argument shape (tool + arg key/type structure), NOT description text.
-- Description is kept for UI display only ("You did this before: {most_recent_description}").
CREATE VIEW repetition_candidates AS
SELECT
    user_id,
    tool_used,
    argument_shape_hash,
    COUNT(*) as occurrence_count,
    MIN(completed_at) as first_seen,
    MAX(completed_at) as last_seen,
    -- Most recent description for human-readable prompt text
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
HAVING COUNT(*) >= 2;
```

---

## C. Daemon Architecture

### C.1 Daemon Lifecycle

The daemon is the renamed `goosed` binary (`permagentd`). It runs as a background process managed by launchd.

**launchd plist:** `~/Library/LaunchAgents/ai.permagent.daemon.plist`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.permagent.daemon</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/permagentd</string>
        <string>agent</string>
        <string>--host</string>
        <string>127.0.0.1</string>
        <string>--port</string>
        <string>3001</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>StandardOutPath</key>
    <string>/Users/USER/.permagent/logs/daemon.log</string>

    <key>StandardErrorPath</key>
    <string>/Users/USER/.permagent/logs/daemon.err</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PERMAGENT_CONFIG</key>
        <string>/Users/USER/.permagent/config.yaml</string>
        <key>PERMAGENT_SPECTRAL_DB</key>
        <string>/Users/USER/.permagent/spectral/permagent.db</string>
    </dict>

    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
```

**Port allocation:**
- Daemon HTTP/WS: `localhost:3001` (API + WebSocket)
- Command Center: `localhost:3000` (Next.js dev server)

Note: The daemon binds to `127.0.0.1` only — no external access in Phase 1.

**Command Center serving:** The daemon (`permagentd`) serves the Command Center as a static export at `localhost:3001/ui/` in addition to the API at `localhost:3001/`. This eliminates the Node.js runtime dependency for end users. The Next.js build runs at package time (during `cargo build` or release packaging), not at install time. No separate dev server is needed in production — `permagent open` points the browser at `localhost:3001/ui/`.

**Agent confirmation mode (`goose_mode`):** Phase 1 defaults to `supervised` mode — the agent asks for user confirmation before executing destructive tool calls (file writes, message sends, etc.). This preserves the Goose `goose_mode` concept. Users can switch to `auto` mode via `permagent config set goose_mode auto`. The mode is stored per-session in the `sessions` table.

### C.2 WebSocket Server

The daemon adds a WebSocket endpoint at `ws://localhost:3001/events`. This supplements the existing Axum HTTP routes from goose-server.

**Connection flow:**
1. Command Center connects to `ws://localhost:3001/events` on page load
2. Daemon sends `daemon_started` event
3. All runtime events broadcast to connected clients
4. Reconnect with exponential backoff on disconnect

### C.3 Event Schema

All events are JSON with this envelope:

```typescript
interface PermagentEvent {
    id: string;            // UUIDv7 (time-sortable)
    type: string;          // event type from list below
    timestamp: string;     // ISO 8601
    payload: object;       // type-specific data
}
```

**Event types:**

```typescript
// --- Daemon lifecycle ---
interface DaemonStarted {
    type: "daemon_started";
    payload: { version: string; config_path: string; spectral_path: string; }
}

interface DaemonStopped {
    type: "daemon_stopped";
    payload: { reason: string; }  // "user_request" | "error" | "update"
}

// --- Task lifecycle ---
interface TaskCreated {
    type: "task_created";
    payload: { task_id: string; description: string; tool: string | null; }
}

interface TaskStarted {
    type: "task_started";
    payload: { task_id: string; session_id: string; }
}

interface TaskCompleted {
    type: "task_completed";
    payload: { task_id: string; output: object; duration_ms: number; }
}

interface TaskFailed {
    type: "task_failed";
    payload: { task_id: string; error: string; }
}

// --- Memory ---
interface MemoryAdded {
    type: "memory_added";
    payload: { memory_id: string; key: string; category: string; wing: string | null; }
}

// --- Skills ---
interface SkillProposed {
    type: "skill_proposed";
    payload: {
        description: string;
        tool_used: string;
        occurrence_count: number;
        source_task_ids: string[];
    }
}

interface SkillSaved {
    type: "skill_saved";
    payload: { skill_id: string; name: string; trigger_type: string; }
}

interface SkillTriggered {
    type: "skill_triggered";
    payload: { skill_id: string; execution_id: string; trigger_type: string; }
}

// --- Session / Chat ---
interface MessageReceived {
    type: "message_received";
    payload: { session_id: string; role: string; content_preview: string; }
}

interface StreamChunk {
    type: "stream_chunk";
    payload: { session_id: string; content: string; done: boolean; }
}

// --- Integration ---
interface IntegrationConnected {
    type: "integration_connected";
    payload: { provider: string; scopes: string[]; }
}

interface IntegrationError {
    type: "integration_error";
    payload: { provider: string; error: string; }
}
```

---

## D. Command Center Component Map

Next.js 14 App Router, TypeScript, Tailwind CSS. Connects to daemon at `ws://localhost:3001/events` for real-time updates and `http://localhost:3001/` for REST.

### D.1 Layout

```
┌─────────────────────────────────────────────────────┐
│  Sidebar (persistent)           │  Main Content     │
│                                 │                   │
│  ┌───────────────────────┐      │                   │
│  │ Chat                  │      │  (routed panel)   │
│  │ Skills                │      │                   │
│  │ Event Log             │      │                   │
│  └───────────────────────┘      │                   │
│                                 │                   │
│  Connection status indicator    │                   │
└─────────────────────────────────────────────────────┘
```

Phase 1 Command Center has three sections: Chat, Skills Library, and Event Log. Memory management and Settings are handled via CLI (see Section E.4). Memory Dashboard and Settings UI are deferred to Phase 1.5 (see Section D.5).

### D.2 Component Breakdown

**App Shell:**
- `RootLayout` — Next.js root layout, loads WebSocket provider
- `Sidebar` — Navigation between sections, connection status badge
- `ConnectionProvider` — React context managing WebSocket lifecycle, reconnect logic, event dispatch
- `EventBus` — Client-side event store (Zustand or React context), buffers events for components

**Chat Pane (`/chat`):**
- `ChatView` — Main chat container, message list + input
- `MessageList` — Scrollable message history, auto-scroll on new messages
- `MessageBubble` — Single message: text, tool calls (collapsible), thinking indicators
- `ToolCallCard` — Expandable card showing tool name, arguments, result
- `ChatInput` — Text input with submit, supports multiline
- `StreamingIndicator` — Typing/thinking animation while assistant streams
- `SkillPromptBanner` — Inline banner: "You've done this before. Save as skill?" with Accept/Dismiss

**Skills Library (`/skills`):**
- `SkillsListView` — Grid/list of saved skills with search/filter
- `SkillCard` — Skill name, description, trigger type, usage count, last run, status badge
- `SkillDetailPanel` — Full skill definition view, step-by-step breakdown
- `SkillEditor` — Edit skill name, description, trigger config (Phase 1: minimal editing)
- `SkillExecutionHistory` — List of past executions with status, duration, errors

**Event Log (`/events`):**
- `EventLogView` — Real-time scrolling event feed from WebSocket
- `EventRow` — Single event: type badge, timestamp, payload summary
- `EventFilter` — Filter by event type, date range
- `EventDetail` — Expandable JSON payload view

### D.3 Data Flow

```
Command Center (Next.js)
    │
    ├── WebSocket (ws://localhost:3001/events)
    │   └── Receives: all PermagentEvent types
    │   └── Sends: none (read-only stream)
    │
    └── REST (http://localhost:3001/)
        ├── POST /reply          — Send chat message
        ├── GET  /sessions       — List sessions
        ├── GET  /sessions/:id   — Get session with messages
        ├── GET  /config         — Read config
        ├── POST /config/upsert  — Update config
        ├── GET  /skills         — List skills (NEW)
        ├── POST /skills         — Create skill (NEW)
        ├── PUT  /skills/:id     — Update skill (NEW)
        ├── DELETE /skills/:id   — Delete skill (NEW)
        ├── POST /skills/:id/run — Execute skill (NEW)
        ├── GET  /memories       — Search memories (NEW)
        ├── POST /memories       — Add memory (NEW)
        └── GET  /events         — Event log history (NEW)
```

### D.5 Deferred to Phase 1.5

The following UI components are deferred until the core loop (install → chat → agent learns → skills get saved) ships and stabilizes. They will be added as a fast-follow after Phase 1.

**Memory Dashboard (`/memory`):**
- `MemoryDashboard` — Overview: total memories, categories, recent additions
- `MemorySearchBar` — FTS search against memories_fts
- `MemoryList` — Paginated list of memories with category/wing/hall filters
- `MemoryCard` — Single memory: key, content preview, temporal validity, confidence
- `KnowledgeGraphPanel` — Simple visualization of SPO triples (table view first, graph viz later)
- `AddMemoryForm` — Manual memory entry: key, content, category, wing/hall/room

**Settings (`/settings`):**
- `SettingsView` — Tabbed settings interface
- `ProviderConfig` — LLM provider selection, API key input, model selection
- `IntegrationList` — Connected integrations (Gmail, Slack) with connect/disconnect
- `DaemonControl` — Start/stop/restart daemon, view logs
- `SpectralInfo` — Database path, size, memory count, last backup

Phase 1 equivalents are provided as CLI commands (see Section E.4).

### D.6 Desktop App (Tauri)

The Command Center ships as a native macOS desktop app built with Tauri 2.x. Tauri wraps the existing Vite/React UI in a native window with:
- Native macOS title bar and window management
- Embedded webview for OAuth flows (Gmail, future integrations)
- Local file system access for config and secrets
- Auto-start with the daemon via launchd
- System tray icon showing agent status (Phase 1: minimal, just running/stopped indicator)
- Deep link handling for OAuth callbacks (permagent://oauth/callback)

The Tauri app replaces the browser-based access pattern. Users no longer need to open localhost:3001/ui/ in Chrome. The `permagent open` command launches the native app instead.

---

## E. CLI Wizard Flow

The CLI wizard is a subcommand of the `permagent` binary (renamed from `goose`).

```
$ permagent setup
```

### E.1 Prompt Sequence

```
Step 1/5: LLM Provider
─────────────────────
Which LLM provider will you use?

  1. Anthropic (Claude)
  2. OpenAI (GPT-4)
  3. Ollama (local)
  4. Google (Gemini)
  5. Other (OpenRouter, Azure, etc.)

Selection [1]: █

→ Writes: provider_name to config


Step 2/5: API Key
─────────────────
Enter your Anthropic API key:
(Get one at https://console.anthropic.com/settings/keys)

API Key: sk-ant-████████████████████████████

✓ Key validated successfully (claude-sonnet-4-5 available)

→ Writes: API key to system keyring (service: "permagent")


Step 3/5: Model Selection
─────────────────────────
Which model? (arrow keys to select)

  > claude-sonnet-4-5 (recommended)
    claude-opus-4-6
    claude-haiku-4-5

→ Writes: default_model to config


Step 4/5: Initialize Spectral Memory
─────────────────────────────────────
Creating memory database at ~/.permagent/spectral/permagent.db...

✓ Spectral initialized (8 tables, 3 FTS indexes)

→ Creates: ~/.permagent/spectral/permagent.db with schema from Section B
→ Creates: default user row in users table


Step 5/5: Start Daemon
──────────────────────
Register Permagent daemon with launchd?
This keeps your agent running in the background.

  [Y]es  [N]o

> Y

✓ Daemon registered: ai.permagent.daemon
✓ Daemon started on localhost:3001

Open Command Center now? [Y/n]: Y
→ Opens http://localhost:3000 in default browser
```

### E.2 Files Written on Completion

```
~/.permagent/
├── config.yaml                     # LLM provider, model, preferences
├── spectral/
│   └── permagent.db               # SQLite database (Section B schema)
├── logs/
│   ├── daemon.log                 # stdout from permagentd
│   └── daemon.err                 # stderr from permagentd
└── secrets.yaml                   # Fallback if keyring unavailable

~/Library/LaunchAgents/
└── ai.permagent.daemon.plist      # launchd service definition
```

### E.3 Config File Format

```yaml
# ~/.permagent/config.yaml
version: 1

provider:
  name: anthropic
  default_model: claude-sonnet-4-5

daemon:
  host: 127.0.0.1
  port: 3001
  auto_start: true

spectral:
  db_path: ~/.permagent/spectral/permagent.db

integrations: {}
  # gmail:
  #   enabled: true
  #   scopes: ["readonly"]
  # slack:
  #   enabled: true
  #   scopes: ["chat:write", "channels:read"]

skills:
  auto_detect: true
  repetition_threshold: 2
  repetition_window_days: 7
```

### E.4 Additional CLI Commands

```bash
# Daemon lifecycle
permagent setup              # First-time wizard
permagent start              # Start daemon (launchctl load)
permagent stop               # Stop daemon (launchctl unload)
permagent restart             # Stop + start
permagent status             # Show daemon status, port, uptime
permagent logs               # Tail daemon logs
permagent open               # Open Command Center in browser

# Config
permagent config get <key>   # Read config value
permagent config set <k> <v> # Write config value
permagent provider set <name>  # Set LLM provider
permagent provider key <key>   # Set API key (writes to keyring)

# Memory management (replaces Memory Dashboard UI in Phase 1)
permagent memory search <query>                     # FTS search against memories table
permagent memory list [--wing X] [--hall Y]         # Paginated list with filters
permagent memory add --key K --content C [--category X]  # Manual memory entry

# Integration management (replaces Settings UI in Phase 1)
permagent integrations list                    # Show connected integrations and status
permagent integrations connect <provider>      # Trigger OAuth flow (CLI fallback; primary flow is desktop app embedded webview)
permagent integrations disconnect <provider>   # Revoke tokens
# Phase 1 supported providers: gmail only (slack deferred)
```

---

## F. Auto-Skills Detection

### F.1 Algorithm

```
On every task completion:
    1. Agent logs task to `tasks` table:
       - description (human-readable, for UI display only)
       - tool_used (MCP tool name, e.g., "gmail__search")
       - argument_shape_hash = stable_hash(tool_used, sorted(arg_keys), arg_type_categories)
         Example: gmail__search with {query: "is:unread"} → hash("gmail__search", ["query"], ["string"])
         Example: slack__post_message with {channel: "C123", text: "hi"} → hash("slack__post_message", ["channel","text"], ["string","string"])
       - steps_json (serialized action sequence)
       - status = "completed"
       - completed_at = now()

    2. Query repetition_candidates view:
       SELECT tool_used, argument_shape_hash, COUNT(*) as n, latest_description
       FROM repetition_candidates
       WHERE user_id = :user

       Matching is on (tool_used, argument_shape_hash), NOT description text.
       Same tool + same argument key/type structure = same shape, regardless of
       specific argument values or how the user phrased the request.

    3. For each match NOT already linked to a skill:
       - Check skills table for existing skill with same tool_used + argument_shape_hash
       - If no existing skill → emit SkillProposed event

    4. Command Center renders SkillPromptBanner in chat:
       "You've done this before: '{latest_description}' ({n} times this week).
        Save as a skill? [Save] [Dismiss]"
       (latest_description is the most recent human-readable description for this shape)

    5. User clicks Save:
       - POST /skills with definition derived from task steps
       - Daemon creates skill row + skill_trigger row
       - Emits SkillSaved event

    6. User clicks Dismiss:
       - Record dismissal (prevent re-prompting for this argument_shape_hash for 30 days)
```

### F.2 Task Log Schema

See `tasks` table in Section B. Key fields for detection:
- `tool_used` — MCP tool name (e.g., `gmail__search`, `slack__post_message`)
- `argument_shape_hash` — stable hash of (tool_used, sorted_arg_keys, arg_type_categories). Computed at insert time. This is the primary matching key.
- `description` — human-readable task description (for UI display only, NOT used for matching)
- `steps_json` — serialized action steps (becomes the skill definition)
- `completed_at` — timestamp for the 7-day window

### F.3 Repetition Threshold

- **Minimum occurrences:** 2 within 7 days (configurable in `config.yaml`)
- **Matching:** exact match on `(tool_used, argument_shape_hash)` pair. Argument shape = same MCP tool called with same set of argument keys and same argument value types/categories. Specific argument values can vary.
- **Dismissal cooldown:** 30 days before re-prompting same argument shape
- **Phase 2 upgrade:** semantic grouping across similar argument shapes via embeddings

### F.4 UI Prompt Flow

1. `SkillProposed` event arrives via WebSocket
2. `SkillPromptBanner` renders inline in chat view (not a modal — non-blocking)
3. Banner shows: task description, occurrence count, Save/Dismiss buttons
4. Save → REST call to `POST /skills`, banner replaced with confirmation
5. Dismiss → REST call to `POST /skills/dismiss`, banner removed

---

## G. Gmail Integration Architecture

> **Scope change (2026-04-23):** Slack integration removed from Phase 1. All chat goes through the Permagent Command Center desktop app. Slack deferred to Phase 2.

### G.1 Integration as MCP Extensions

Gmail registers as an MCP extension via the existing `ExtensionConfig::Stdio` or `ExtensionConfig::StreamableHttp` variant. It is NOT built into the Rust daemon — it runs as a separate process.

```
permagentd (daemon)
    │
    ├── ExtensionManager
    │   ├── gmail-mcp (Stdio extension)
    │   │   └── Python process implementing MCP protocol
    │   │       Tools: gmail__search, gmail__read, gmail__list_labels, gmail__list_threads
    │   │
    │   └── ... other extensions
    │
    └── Tauri desktop app
        └── Embedded webview for OAuth flows
```

**Extension config in `~/.permagent/config.yaml`:**
```yaml
extensions:
  gmail:
    type: stdio
    cmd: permagent-gmail-mcp
    args: []
    envs:
      GMAIL_TOKEN_PATH: ~/.permagent/secrets/gmail_token.json
    enabled: true
    timeout: 30

  # REMOVED: slack (deferred to Phase 2 — all chat goes through Command Center desktop app)
```

### G.2 OAuth Flow

**Gmail (Google OAuth 2.0 via embedded Tauri webview):**
1. User clicks "Connect Gmail" in Command Center Settings
2. Tauri app opens an embedded webview loading Google OAuth consent screen with scopes: `gmail.readonly`
3. User approves in the embedded webview
4. Callback URL (`permagent://oauth/callback`) is intercepted by Tauri
5. Tauri extracts the authorization code from the callback
6. Daemon exchanges code for access_token + refresh_token
7. Tokens written to `~/.permagent/secrets/gmail_token.json`
8. `IntegrationConnected` event emitted
9. Integration row created in Spectral `integrations` table
10. Integration status updates in real-time in the Command Center

**CLI fallback:** `permagent integrations connect gmail` opens a browser-based OAuth flow for headless/CLI-only environments. The primary flow is through the desktop app embedded webview.

~~**Slack (Slack OAuth 2.0):**~~ **REMOVED** — Slack integration deferred to Phase 2. All chat goes through the Permagent Command Center desktop app.

### G.3 Token Storage

**Tokens do NOT live in Spectral.** They live in a separate secrets store:

```
~/.permagent/secrets/
├── gmail_token.json     # {access_token, refresh_token, expiry, scopes}
└── .gitignore           # "*.json" — never commit
```

**File permissions:** `chmod 600` on all files in `secrets/`

**Refresh logic:** The Gmail MCP extension handles its own token refresh. If refresh fails, the extension emits `IntegrationError` and the Command Center shows reconnect prompt.

**System keyring (preferred):** If available, store tokens in macOS Keychain instead of filesystem. Service name: `"permagent"`, account: `"gmail"`. File-based fallback when keyring is unavailable (headless servers, CI).

### G.4 Gmail Tools (Phase 1 — Read Only)

| Tool | Description |
|------|-------------|
| `gmail__search` | Search emails by query (same syntax as Gmail search bar) |
| `gmail__read` | Read full email by message ID |
| `gmail__list_labels` | List all labels/folders |
| `gmail__list_threads` | List recent threads with pagination |

### ~~G.5 Slack Tools (Phase 1 — Write Capable)~~ REMOVED

> **REMOVED (2026-04-23):** Slack integration deferred to Phase 2. Slack tools (`slack__post_message`, `slack__list_channels`, `slack__search_messages`, `slack__set_reminder`) will be implemented when Slack is added back in scope.

---

## H. Lines of Code Estimate & Implementation Order

### H.1 LOC Estimates

| Component | New LOC | Modified LOC | Language |
|-----------|---------|-------------|----------|
| Spectral storage module | ~1,200 | — | Rust |
| Session manager replacement | ~400 | ~200 | Rust |
| Config system (`~/.permagent/`) | ~300 | ~500 | Rust |
| WebSocket event server | ~600 | — | Rust |
| Skills engine (detection + storage) | ~800 | — | Rust |
| Skills REST API routes | ~400 | — | Rust |
| Memory REST API routes | ~300 | — | Rust |
| CLI wizard (`permagent setup`) | ~500 | ~300 | Rust |
| CLI daemon management | ~200 | ~100 | Rust |
| Gmail MCP extension | ~600 | — | Python |
| ~~Slack MCP extension~~ | ~~~700~~ | — | ~~Python~~ REMOVED (deferred to Phase 2) |
| Tauri desktop app shell | ~400 | — | Rust + TypeScript |
| Tauri embedded OAuth webview | ~300 | — | Rust |
| **Daemon subtotal** | **~5,000** | **~1,100** | **Rust + Python** |
| Command Center: app shell + routing | ~400 | — | TypeScript/React |
| Command Center: WebSocket client | ~300 | — | TypeScript |
| Command Center: Chat pane | ~800 | — | TypeScript/React |
| Command Center: Skills library | ~600 | — | TypeScript/React |
| Command Center: Event log | ~300 | — | TypeScript/React |
| Command Center: shared components | ~200 | — | TypeScript/React |
| **Command Center subtotal** | **~2,600** | — | **TypeScript** |
| CLI: memory commands | ~150 | — | Rust |
| CLI: integrations commands | ~150 | — | Rust |
| **CLI additions subtotal** | **~300** | — | **Rust** |
| **Total** | **~7,900** | **~1,100** | |

*Memory Dashboard (~500), Settings (~400), and shared AddMemoryForm (~100) moved to Phase 1.5 (see D.5). ~300 LOC added back as CLI equivalents. Net reduction: ~700 LOC.*

### H.2 Implementation Order (4-6 Weeks)

**Week 1: Foundation**
1. Fork Goose → `permagent-runtime` repo, rename crates
2. Excise Electron UI (`ui/desktop/`)
3. Implement config system replacement (`~/.permagent/`)
4. Create Spectral schema + migration runner
5. Replace `SessionStorage` with Spectral-backed implementation

**Week 2: Daemon**
6. Add WebSocket event server to `permagentd`
7. Implement task logging (tasks table + API routes)
8. Build CLI wizard (`permagent setup`)
9. Create launchd plist generation + `permagent start/stop`
10. Build skills engine (CRUD + repetition detection)

**Week 3: Command Center + CLI**
11. Scaffold Next.js 14 app with App Router
12. Build WebSocket connection provider
13. Implement Chat pane (message list, input, streaming)
14. Implement Event Log (real-time feed)
15. Implement Skills Library (list, detail, save flow)
16. Build `permagent memory` CLI commands (search, list, add)
17. Build `permagent integrations` CLI commands (list, connect, disconnect)

**Week 4: Desktop App + Integrations + End-to-End**
18. Wrap Command Center in Tauri desktop app (macOS native)
19. Build embedded OAuth webview for integration connections
20. Build Gmail MCP extension (Python, OAuth via embedded webview)
21. Wire auto-skills detection end-to-end (task completion -> repetition check -> skill proposal -> save)

**Week 5-6: Testing + Hardening**
22. Test on 5+ non-developer machines
23. Tauri desktop app testing (Intel + Apple Silicon macOS)
24. Error handling, reconnect logic, edge cases
25. Performance: Spectral query optimization, FTS tuning
26. Documentation: README, setup guide

---

## I. Open Questions & Risks

### Open Questions

| # | Question | Owner | Impact |
|---|----------|-------|--------|
| 2 | Permagent app registration for Gmail OAuth only (Slack removed from Phase 1 scope) — do we register our own Google Cloud project, or use user-provided credentials? | Jesse | Blocks Week 4 |

**Caveat on #2:** Phase 1 can ship with user-provided credentials as a fallback — the user registers their own Google Cloud project, then provides client ID/secret via `permagent integrations connect gmail`. If a Permagent-owned OAuth app isn't ready by Week 4, this is the path. Slack integration deferred. All chat goes through the Permagent Command Center desktop app.

**Resolved questions (moved to spec):**
- ~~#1 (Goose fork version):~~ Decided: block/goose. See `specs/SPEC_OPUS_GAP_FIXES.md`.
- ~~#3 (Task description normalization):~~ Resolved by argument-shape matching (Section F). Description text is display-only; matching uses `(tool_used, argument_shape_hash)`.
- ~~#4 (Command Center dev server vs static export):~~ Decided: static export served by daemon at `localhost:3001/ui/`. See Section C.1.
- ~~#5 (Keep goose_mode?):~~ Decided: keep it, default to `supervised` in Phase 1. See Section C.1.

### Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Goose upstream breaks our fork | Medium | Minimal fork surface. Only replace session storage, config, and UI. Keep execution engine and MCP system untouched. Track upstream quarterly, cherry-pick security fixes only. |
| Spectral migration complexity | High | Build migration runner from Day 1. Schema version table. Test with real conversation data from existing Goose sessions. |
| Auto-skills detection is noisy | Medium | Conservative threshold (2x/7d). Dismissal cooldown (30d). Track accept/reject rates from Week 4 onward. |
| Gmail OAuth requires app verification | Medium | Gmail: Google OAuth for "testing" works for up to 100 users without verification. Sufficient for Phase 1. Slack removed from Phase 1 scope. |
| Tauri desktop app packaging | Medium | Tauri 2.x is stable on macOS. Test on Intel and Apple Silicon. Code signing deferred to post-Phase 1. |
| CLI wizard UX is confusing | Medium | Test with 3 non-technical users in Week 5. Iterate on prompt text. Add `--non-interactive` flag for CI/automation. |
| WebSocket reconnect reliability | Low | Exponential backoff (1s, 2s, 4s, 8s, max 30s). Event replay from last-seen event ID on reconnect. Daemon buffers last 1000 events. |
| Spectral DB corruption on crash | Medium | SQLite WAL mode. Write-ahead logging. Periodic VACUUM in maintenance window. Backup command in CLI. |
| goose-server route conflicts with new endpoints | Low | New routes use `/permagent/` prefix. Existing Goose routes kept at root for backward compatibility during transition. |

### Blocking Issues

None identified. The Goose audit confirms all surgery points are accessible:
- Session storage has a clear single-module boundary (no trait, but clean public API)
- MCP system is well-abstracted behind `McpClientTrait`
- Electron UI is fully decoupled from the Rust backend (communicates only via HTTP/WS)
- Provider system needs zero changes
- Config system is modular with clear path replacement points

The main risk is scope — ~7,900 new LOC in 4-6 weeks is achievable given that the Goose foundation handles execution, providers, and MCP tooling, and the Command Center scope is reduced to three core panels.

### I.3 Known Phase 2 Migrations

The following changes are anticipated for Phase 2 (Mesh/Chitin integration). **Do not build any of this in Phase 1.** This section exists so future schema migrations and CLI additions don't surprise anyone.

**Schema additions to `users` table:**
- `chitin_id TEXT UNIQUE` — Mesh identity (Chitin ID)
- `mesh_joined_at TEXT` — timestamp for Founding Executive tracking (first 5 users to join Mesh Forum with verified Chitin ID get permanent founding status)

**New tables (spec in Phase 2):**
- `mesh_reputation` — reputation scores from Mesh participation
- `forum_interactions` — Mesh Forum activity log
- `founding_executive` — permanent founding status registry

**Secrets store additions:**
- `~/.permagent/secrets/mesh_auth_token.json` alongside existing `gmail_token.json`
- `~/.permagent/secrets/slack_token.json` (when Slack integration is added in Phase 2)

**CLI additions:**
- `permagent mesh join` — join the Mesh network
- `permagent mesh status` — show Mesh connection and reputation
- `permagent chitin register` — register or link a Chitin ID
