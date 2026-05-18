# Permagent State Inventory

Every persistent artifact under `~/.permagent/` (or `$PERMAGENT_PATH_ROOT` override).

All paths are resolved through `crates/goose/src/config/paths.rs` via `Paths::base_dir()`.

---

## Brain (Spectral-owned)

### `~/.permagent/brain/memory.db`

| Field | Value |
|-------|-------|
| **Owner** | Spectral |
| **Purpose** | Primary memory store -- facts, observations, and activity entries. Includes FTS5 virtual table (`memories_fts`) for BM25-ranked recall. |
| **Schema** | SQLite. Tables: `memories` (id, key, content, description, source, device_id, confidence, visibility, wing, created_at, signal_score, compaction_tier), `memory_annotations`, `memories_fts`, `retrieval_events`, `co_retrieval_pairs`. Schema version managed internally by Spectral. |
| **Read-by** | `brain_ops::read_only_brain_conn()` (brain_ops.rs:167), `routes/brain.rs` (search, graph, memories endpoints), `routes/ollama.rs` (memory counts, consolidation scan, co-retrieval rebuild), `activity/cleanup.rs`, `librarian.rs` (describe batch) |
| **Written-by** | Spectral `Brain::remember_with()` (via state.rs Brain mount), `routes/ollama.rs` (direct `UPDATE` for `_pm_consolidated_into` -- see Schema Extensions below) |
| **Migration** | Spectral auto-migrates on `Brain::builder().open()`. Permagent-side `_pm_consolidated_into` column added via idempotent `ALTER TABLE ... ADD COLUMN` (.ok() swallows duplicate-column error). |
| **Backup** | Copy `.db`, `.db-wal`, and `.db-shm` together. Safe to delete if you accept full memory loss. Regenerated empty on next Brain init. |

### `~/.permagent/brain/graph.kz`

| Field | Value |
|-------|-------|
| **Owner** | Spectral |
| **Purpose** | Kuzu knowledge graph -- entity nodes and relationship edges for recall_cascade and entity-aware retrieval. |
| **Schema** | Binary (Kuzu embedded graph DB). Node/relationship tables defined by Spectral based on ontology.toml entity types. |
| **Read-by** | Spectral Brain API (recall, entity queries), `routes/brain.rs` (entity count, graph browse) |
| **Written-by** | Spectral `Brain::remember_with()` (entity extraction + graph insert) |
| **Migration** | Spectral-managed. Ontology changes may trigger graph re-indexing. |
| **Backup** | Copy `graph.kz` and `graph.kz.wal` together. Safe to delete if you accept graph loss; rebuilt on next Brain init but entities from existing memories are NOT re-extracted automatically. |

### `~/.permagent/brain/ontology.toml`

| Field | Value |
|-------|-------|
| **Owner** | Spectral (template shipped in `crates/goose/assets/ontology.toml`) |
| **Purpose** | Entity taxonomy -- defines canonical entity types, aliases, and visibility flags for the knowledge graph. |
| **Schema** | TOML. `version = 1`. Sections per entity type with `canonical_name`, `aliases`, `visibility`. No pre-declared instances (enforced by test in `spectral_smoke.rs`). |
| **Read-by** | Spectral `Brain::builder()` at init, `state.rs:72` (existence check at daemon startup) |
| **Written-by** | Copied from `assets/ontology.toml` at first Brain init. Edited manually or via ontology cleanup PRs (e.g., PR #139). |
| **Migration** | Manual edits. `version` field is format version, not instance version. |
| **Backup** | Safe to copy. If deleted, Brain fails to initialize (long-term memory disabled). Can be restored from `assets/ontology.toml`. |

### `~/.permagent/brain/brain.id`

| Field | Value |
|-------|-------|
| **Owner** | Spectral |
| **Purpose** | Device identifier for distributed multi-device support. 64-byte hex string. |
| **Read-by** | Spectral Brain constructor |
| **Written-by** | Spectral Brain constructor (generated on first init) |
| **Migration** | None. Stable across Spectral pin bumps. |
| **Backup** | Safe to copy. If deleted, Spectral generates a new one -- device identity changes but memories remain. |

### `~/.permagent/brain/brain.key`

| Field | Value |
|-------|-------|
| **Owner** | Spectral |
| **Purpose** | Private key for memory encryption/signing. 32-byte binary. |
| **Read-by** | Spectral Brain (internal crypto operations) |
| **Written-by** | Spectral Brain constructor (generated on first init) |
| **Migration** | None. |
| **Backup** | **Critical.** If lost, encrypted memories become unrecoverable. Must be copied alongside `brain.pub`. Never commit to version control. |

### `~/.permagent/brain/brain.pub`

| Field | Value |
|-------|-------|
| **Owner** | Spectral |
| **Purpose** | Public key paired with `brain.key`. 32-byte binary. |
| **Read-by** | Spectral Brain (internal crypto operations) |
| **Written-by** | Spectral Brain constructor (generated on first init) |
| **Migration** | None. |
| **Backup** | Must be copied alongside `brain.key`. |

---

## Permagent

### `~/.permagent/spectral/permagent.db`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Session/message/task/skill database -- all UI conversation history and metadata. |
| **Schema** | SQLite, `SPECTRAL_SCHEMA_VERSION = 6` (tracked in `schema_version` table). WAL mode. Tables: `users`, `sessions`, `messages`, `threads`, `thread_messages`, `memories`, `knowledge_graph`, `tasks`, `skills`, `skill_executions`, `skill_triggers`, `integrations`, plus FTS5 virtual tables, triggers, and views. |
| **Read-by** | `session/session_manager.rs` (SessionManager pool), `routes/henry_status.rs`, `commands/memory.rs`, `commands/info.rs` |
| **Written-by** | `session/spectral_schema.rs:init_spectral_db()` (creates schema), SessionManager (messages, sessions), annotation backfill |
| **Migration** | Versioned. Migrations in `spectral_schema.rs` (e.g., `migrate_v4_to_v5()`). Auto-applied on pool init. |
| **Backup** | Copy `.db`, `.db-wal`, `.db-shm` together. If deleted, all session history lost. Regenerated empty on next daemon startup. |

### `~/.permagent/data/librarian_state.json`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Persisted Librarian scheduler timestamps -- tracks when warm-load, co-retrieval rebuild, and consolidation last ran. |
| **Schema** | JSON: `{ "last_warmed_date": "2026-05-17", "last_co_retrieval_rebuild": "2026-05-17T14:32:00.000Z", "last_consolidated": "2026-05-17T14:32:00.000Z" }`. All fields nullable. |
| **Read-by** | `routes/ollama.rs` -- `load_scheduler_state()`, `already_warmed_today()`, `co_retrieval_rebuild_due()`, `consolidation_due()` |
| **Written-by** | `routes/ollama.rs` -- `save_scheduler_state()`. Atomic write-then-rename (tmp -> final). |
| **Migration** | Additive (new fields default to null). |
| **Backup** | Safe to delete. Librarian re-warms on next scheduled window (wasteful but not harmful). |

### `~/.permagent/data/librarian_schedule.json`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Librarian scheduling configuration -- when to run, which model, duration, pruning toggle. |
| **Schema** | JSON: `{ "enabled": true, "start_time": "02:00", "duration_minutes": 240, "model": "qwen2.5:7b", "run_if_launched_in_window": true, "pruning_enabled": false }` |
| **Read-by** | `routes/ollama.rs` (GET /api/librarian/schedule, scheduler loop), `librarian.rs:resolve_model()`, `librarian.rs:load_schedule_summary()` |
| **Written-by** | `routes/ollama.rs` (PUT /api/librarian/schedule) |
| **Migration** | Additive. Falls back to compiled-in defaults if missing or if a field is absent. |
| **Backup** | Safe to delete. Uses defaults (02:00 UTC, 240min, qwen2.5:7b, enabled). |

### `~/.permagent/config.yaml`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Primary application configuration -- daemon host/port, provider credentials, integration settings. |
| **Schema** | YAML. Sections: `daemon` (host, port, tls), `integrations` (per-provider API keys/models), flags (`tunnel_auto_start`, `disable_keyring`). |
| **Read-by** | `config/base.rs:139` (Config loading), `configuration.rs:46` (PERMAGENT_CONFIG env override), `routes/integrations.rs` |
| **Written-by** | `commands/setup.rs` (setup wizard), `routes/integrations.rs` (provider config updates) |
| **Migration** | Additive. Unknown keys ignored. Precedence: env vars > config.yaml > bundled defaults. |
| **Backup** | Safe to copy. If deleted, daemon uses hardcoded defaults; many features disabled until re-configured. Does not contain secrets (those go in keyring or secrets.yaml). |

### `~/.permagent/agent.yaml`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Agent persona configuration -- primary agent identity and worker personas. |
| **Schema** | YAML. `primary` (first_name, last_name, nickname, traits, tone, opening_greeting, voice_id). `workers` map of named worker personas with same fields plus `role`. |
| **Read-by** | `config/agent_identity.rs:149` (`load_agent_config`), `routes/agents.rs:23` (persona endpoint) |
| **Written-by** | `config/agent_identity.rs:161` (`save_agent_config`) |
| **Migration** | Additive. Falls back to defaults (primary: "Aria", no workers). |
| **Backup** | Safe to delete. Uses built-in defaults. |

### `~/.permagent/secrets/daemon_token.json`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Bearer token for daemon API authentication (activity/emit, findings endpoints). |
| **Schema** | JSON: `{ "token": "0a1b2c3d..." }` (32-byte random hex string). |
| **Read-by** | `state.rs:479` (`load_or_create_daemon_token`), auth middleware in `routes/activity.rs:37`, `routes/findings.rs:116` |
| **Written-by** | `state.rs:479` (generated on startup if missing). File mode 0600. |
| **Migration** | None. Regenerated if missing. |
| **Backup** | If deleted, daemon generates a new token on next startup. Clients using the old token will get 401 until they read the new one. |

### `~/.permagent/automation/findings/*.json`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Automation recipe results -- cleanup findings, storage analysis, actionable recommendations. One file per run. |
| **Schema** | JSON: `{ "run_id": "...", "findings": [{ "id", "type", "path", "size_bytes", "age_days", "recommendation", "action_taken", "actioned_at", "size_recovered_bytes" }] }` |
| **Read-by** | `routes/findings.rs:63` (`load_findings`) |
| **Written-by** | `routes/findings.rs` (`save_findings`), `scheduler.rs` (recipe runner). Atomic writes. Sensitive paths rejected (/.ssh/, /.aws/, .env, etc.). |
| **Migration** | None. Each file is self-contained. |
| **Backup** | Safe to delete. Historical findings lost but no functional impact. |

### `~/.permagent/tunnel.lock`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Exclusive file lock preventing multiple daemon instances from running the tunnel concurrently. Contains holder PID. |
| **Schema** | Plain text (PID). Lock semantics via `fs2::FileExt::try_lock_exclusive`. |
| **Read-by** | `tunnel/mod.rs:18` (`is_tunnel_locked_by_another`) |
| **Written-by** | `tunnel/mod.rs:18` (`try_acquire_tunnel_lock`). Lock held for tunnel lifetime, dropped on TunnelManager drop. |
| **Migration** | None. |
| **Backup** | Safe to delete when daemon is stopped. Stale lock file from a crashed daemon may block tunnel startup (lock is process-scoped, so OS releases on exit). |

### `~/.permagent/instance_id`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Stable UUID v4 identifying this Permagent installation. Survives restarts. |
| **Schema** | Plain text UUID (e.g., `550e8400-e29b-41d4-a716-446655440000`). |
| **Read-by** | `instance_id.rs:34` (`get_instance_id()`) -- LazyLock, read once per process |
| **Written-by** | `instance_id.rs:12` (`load_or_create()`) -- generated on first use if missing |
| **Migration** | None. |
| **Backup** | Safe to delete. A new UUID is generated, changing the installation identity. |

### `~/.permagent/logs/`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Application logs organized by component and date. |
| **Schema** | Directory structure: `logs/{component}/{YYYY-MM-DD}/{timestamp}-{name}.log`. JSON format (tracing subscriber). Components: cli, server, activity. |
| **Read-by** | External tooling / manual inspection |
| **Written-by** | `logging.rs:14` (`prepare_log_directory`), tracing_appender::rolling |
| **Migration** | None. No automated rotation. Files accumulate by date. |
| **Backup** | Safe to delete entirely. No functional impact. |

### `~/.permagent/data/models/`

| Field | Value |
|-------|-------|
| **Owner** | Permagent |
| **Purpose** | Downloaded LLM model files for local inference (GGUF format). |
| **Schema** | Binary files: `.gguf` (quantized LLMs), `.gguf.mmproj` (multimodal projectors). Organized by model ID. |
| **Read-by** | `routes/local_inference.rs` (inference backend) |
| **Written-by** | `routes/local_inference.rs:412` (download endpoint). Path traversal validation enforced. |
| **Migration** | None. Re-downloadable. |
| **Backup** | Safe to delete. Models re-downloaded on demand. Can be large (multi-GB). |

---

## Investigated: `~/.permagent/db.sqlite`

| Field | Value |
|-------|-------|
| **Owner** | Unknown / legacy |
| **Purpose** | **Dead.** File exists on disk (0 bytes) but no code in the codebase references `db.sqlite`. |
| **Read-by** | Nothing |
| **Written-by** | Nothing |
| **Proposal** | Safe to delete. Likely a leftover from pre-Spectral Goose-inherited code before the migration to `spectral/permagent.db`. |

---

## Schema Extensions

### The `_pm_` column convention

Permagent extends Spectral-owned tables with columns prefixed `_pm_` to signal they are Permagent-side additions that Spectral does not manage.

**Current extensions:**

| Table | Column | Type | Purpose | Code |
|-------|--------|------|---------|------|
| `memories` (in `brain/memory.db`) | `_pm_consolidated_into` | `TEXT DEFAULT NULL` | Points to the keeper memory ID when this memory has been consolidated (deduplicated). `NULL` = unconsolidated. | `routes/ollama.rs:671,777` |

**Invariants:**

- Added via idempotent `ALTER TABLE ... ADD COLUMN` (`.ok()` swallows duplicate-column error)
- **Never assume Spectral preserves `_pm_` columns across pin bumps.** A Spectral migration that recreates the `memories` table will drop them.
- Consolidation queries filter on `_pm_consolidated_into IS NULL` to exclude already-merged memories from candidate sets
- The migration path to remove this convention is tracked in [Spectral #131](https://github.com/make-tuned-unit/spectral/issues/131) (Brain.consolidate_into() API)

**Consolidation strategies using this column:**

1. **Exact duplicates** (`find_exact_duplicate_clusters`, ollama.rs:596) -- groups by identical content, keeps oldest, marks rest
2. **Browser navigation domains** (`find_domain_clusters`, ollama.rs:616) -- groups "Navigated to..." entries by domain (3+ threshold), creates summary via `brain.remember_with()` with key `consolidated:browser:{domain}`, marks originals

---

## Path resolution reference

All paths derive from `Paths::base_dir()` in `crates/goose/src/config/paths.rs`:

```
~/.permagent/                    # base_dir() -- or $PERMAGENT_PATH_ROOT
  brain/                         # brain_dir()
    memory.db
    graph.kz
    ontology.toml                # brain_ontology()
    brain.id
    brain.key
    brain.pub
  spectral/                      # spectral_dir()
    permagent.db                 # spectral_db()
  data/
    librarian_state.json
    librarian_schedule.json
    models/
  secrets/
    daemon_token.json
    secrets.yaml                 # fallback when keyring disabled
    gmail_token.json             # OAuth token (post-flow)
  automation/
    findings/*.json
  logs/                          # logs_dir()
    {component}/{date}/*.log
  config.yaml
  agent.yaml
  tunnel.lock
  instance_id
  db.sqlite                     # DEAD -- safe to delete
```
