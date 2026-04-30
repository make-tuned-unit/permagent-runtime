# Inheritance Audit: Goose vs Permagent Surfaces

**Date:** 2026-04-29
**Commit baseline:** `fd76019de` (main)
**Scope:** Descriptive inventory of inherited Goose architecture, Permagent-specific additions, and orphaned code.

## Executive Summary

Permagent is a fork of Goose that retains the full upstream provider, extension, and session architecture while adding three major surfaces: Spectral Brain (long-term memory via recall/remember), a global event bus (WebSocket), and a gateway manager (Slack/Discord). The crate has been renamed (`goose` package → `permagent`) and the system prompt rebranded, but directory names retain `goose-*` prefixes for upstream merge compatibility. The provider surface is unexpectedly large (30 distinct provider structs across 43+ files), which reflects upstream Goose growth rather than Permagent additions. Two files and one struct field are confirmed orphaned.

---

## Surface 1: Providers

**Location:** `crates/goose/src/providers/`
**Permagent modifications:** None. All providers are inherited from Goose.

### Provider Trait

Defined in `base.rs`. Key methods:

| Method | Purpose |
|--------|---------|
| `stream()` | Primary async streaming completion |
| `complete()` / `complete_fast()` | Non-streaming / fast-model completion |
| `get_model_config()` | Current model configuration |
| `fetch_supported_models()` / `fetch_recommended_models()` | Model discovery |
| `supports_embeddings()` / `create_embeddings()` | Embedding capability |
| `manages_own_context()` | True for CLI wrappers (Claude Code, Codex, etc.) |
| `supports_cache_control()` | Prompt caching support |
| `configure_oauth()` / `refresh_credentials()` | OAuth lifecycle |
| `permission_routing()` | Permission confirmation routing |

### Registry

`provider_registry.rs` — Async-lazy singleton `OnceCell<RwLock<ProviderRegistry>>`. Providers registered at init in `init.rs` with types: Preferred, Builtin, Declarative, Custom.

### Complete Provider Inventory (30 provider structs)

**API-based (direct implementation):**

| Provider | File | Default Model | Auth |
|----------|------|---------------|------|
| Anthropic | `anthropic.rs` | claude-sonnet-4-5 | API key |
| OpenAI | `openai.rs` | gpt-4o | API key |
| Google | `google.rs` | gemini-2.5-pro | API key |
| Databricks | `databricks.rs` | (dynamic) | OAuth/token |
| GCP Vertex AI | `gcpvertexai.rs` | (dynamic) | GCP credentials |
| Snowflake | `snowflake.rs` | claude-sonnet-4-5 | Cortex SQL |
| Venice | `venice.rs` | (dynamic) | API key |

**OpenAI-compatible wrappers (via `openai_compatible.rs`):**

| Provider | File | Default Model |
|----------|------|---------------|
| Azure OpenAI | `azure.rs` | gpt-4o |
| OpenRouter | `openrouter.rs` | (multi-provider) |
| Tetrate | `tetrate.rs` | (multi-provider) |
| LiteLLM | `litellm.rs` | gpt-4o-mini |
| NanoGPT | `nanogpt.rs` | anthropic/claude-sonnet-4.6 |
| Avian | `avian.rs` | deepseek/deepseek-v3.2 |
| XAI (Grok) | `xai.rs` | grok-code-fast-1 |

**OAuth-based:**

| Provider | File | Default Model |
|----------|------|---------------|
| Gemini OAuth | `gemini_oauth.rs` | (Gemini models) |
| Kimi Code | `kimicode.rs` | kimi-for-coding |
| ChatGPT Codex | `chatgpt_codex.rs` | gpt-5.3-codex |
| GitHub Copilot | `githubcopilot.rs` | gpt-4.1 |

**CLI wrappers (`manages_own_context() = true`):**

| Provider | File | Subprocess |
|----------|------|------------|
| Claude Code | `claude_code.rs` | `claude` |
| Codex | `codex.rs` | `codex exec` |
| Gemini CLI | `gemini_cli.rs` | `gemini` |
| Cursor Agent | `cursor_agent.rs` | `cursor-agent` |

**ACP (Agent Client Protocol) adapters:**

| Provider | File | Binary |
|----------|------|--------|
| Claude ACP | `claude_acp.rs` | `claude-agent-acp` |
| Codex ACP | `codex_acp.rs` | `@zed-industries/codex-acp` |
| Copilot ACP | `copilot_acp.rs` | `@github/copilot` |
| Pi ACP | `pi_acp.rs` | (Pi binary) |
| Amp ACP | `amp_acp.rs` | `amp-acp` |

**Local/on-device:**

| Provider | File | Backend |
|----------|------|---------|
| Ollama | `ollama.rs` | Ollama daemon (localhost:11434) |
| Local Inference | `local_inference.rs` | llama-cpp-rs (GGUF, Metal/CUDA) |

**Feature-gated (AWS):**

| Provider | File | Feature Flag |
|----------|------|-------------|
| Bedrock | `bedrock.rs` | `aws-providers` |
| SageMaker TGI | `sagemaker_tgi.rs` | `aws-providers` |

**Infrastructure files (not provider structs):**

| File | Purpose |
|------|---------|
| `openai_compatible.rs` | Shared OpenAI-compatible implementation |
| `acp_tooling.rs` | ACP tool marshalling |
| `api_client.rs` | Shared HTTP client |
| `embedding.rs` | Embedding trait + models |
| `errors.rs` | `ProviderError` types |
| `retry.rs` | Retry configuration |
| `utils.rs` | Image handling, request logging |
| `oauth.rs` / `oauth_device_flow.rs` | OAuth helpers |
| `cli_common.rs` | CLI provider utilities |
| `toolshim.rs` | Tool shim for non-native models |
| `usage_estimator.rs` | Token estimation |
| `catalog.rs` | Provider catalog/metadata |
| `inventory.rs` | Model inventory system |
| `testprovider.rs` / `provider_test.rs` | Test harnesses |
| `canonical/` | Canonical model registry (4 files) |
| `formats/` | Request/response formatters (10 files) |

---

## Surface 2: MCP Extensions

**Location:** `crates/goose/src/agents/platform_extensions/`, `crates/goose-mcp/src/`, `crates/goose/src/agents/extension*.rs`

### Extension Types

Defined in `extension.rs`:

| Type | Description |
|------|-------------|
| Builtin | MCP servers bundled in goose-mcp crate |
| Platform | In-process extensions with direct agent access |
| Stdio | Child process via stdin/stdout |
| StreamableHttp | HTTP-based MCP clients |
| Frontend | Tools provided by UI |
| InlinePython | Python code via uvx |
| Sse | Deprecated (kept for config compat) |

### Platform Extensions (12 total)

| Name | File | Default Enabled | Unprefixed |
|------|------|-----------------|------------|
| analyze | `platform_extensions/analyze/mod.rs` | yes | yes |
| developer | `platform_extensions/developer/mod.rs` | yes | yes |
| summon | `platform_extensions/summon.rs` | yes | yes |
| skills | `platform_extensions/skills.rs` | yes | yes |
| todo | `platform_extensions/todo.rs` | yes | no |
| apps | `platform_extensions/apps.rs` | yes | no |
| extensionmanager | `platform_extensions/ext_manager.rs` | yes | no |
| tom (Top of Mind) | `platform_extensions/tom.rs` | yes | no |
| chatrecall | `platform_extensions/chatrecall.rs` | no | no |
| summarize | `platform_extensions/summarize.rs` | no | no |
| code_execution | `platform_extensions/code_execution.rs` | no | yes |
| orchestrator | `platform_extensions/orchestrator.rs` | no (hidden) | no |

### Builtin MCP Servers (4 total)

| Name | File | Purpose |
|------|------|---------|
| autovisualiser | `goose-mcp/src/autovisualiser/` | Charts, maps, diagrams (d3, leaflet, mermaid) |
| computercontroller | `goose-mcp/src/computercontroller/` | Document processing (PDF, DOCX, XLSX) |
| memory | `goose-mcp/src/memory/` | Memory storage |
| tutorial | `goose-mcp/src/tutorial/` | Interactive tutorials |

### User-Configurable Extensions (1 shipped)

| Name | Location | Type |
|------|----------|------|
| gmail_mcp | `extensions/gmail_mcp/` | Stdio (Python) |

### Extension Manager

`extension_manager.rs` (~2000 lines) handles lifecycle: discovery, loading, tool caching, invalidation, dispatch. Includes malware checking via OSV for Stdio/InlinePython extensions (`extension_malware_check.rs`).

### Permagent-Specific Extensions

- **chatrecall** — Platform extension that searches past conversations via Spectral session storage. Two modes: keyword search across sessions, or load a specific session's messages.
- The **memory** builtin MCP server exists but appears to be Goose-inherited, separate from Spectral Brain.

---

## Surface 3: Session Handlers & Routes

**Location:** `crates/goose-server/src/routes/` (27 files), `crates/goose/src/session/` (8 files)

### Route Inventory

| File | Routes | Origin |
|------|--------|--------|
| **session.rs** | GET/POST/DELETE/PUT `/api/sessions/*` (11 endpoints: list, create, get, export, import, delete, rename, fork, extensions, insights, search) | Goose |
| **session_events.rs** | GET `/sessions/{id}/events`, POST `/sessions/{id}/reply`, POST `/sessions/{id}/cancel` | Goose + **Permagent brain recall/remember** |
| **reply.rs** | POST `/reply` (legacy endpoint) | Goose + **Permagent brain recall/remember** |
| **agent.rs** | 17 endpoints under `/agent/*` (start, resume, restart, tools, provider, extensions, apps, stop, etc.) | Goose |
| **action_required.rs** | POST `/action-required/tool-confirmation` | Goose |
| **config_management.rs** | CRUD for `/config/*`, `/config/extensions/*`, `/config/providers/*` | Goose |
| **recipe.rs** | CRUD for `/recipe/*` (create, encode, decode, scan, save, delete, list, get, validate) | Goose |
| **schedule.rs** | CRUD for `/schedule/*` (create, list, update, delete, run, kill, inspect) | Goose |
| **gateway.rs** | `/gateway/*` (start, stop, restart, status, pairing, remove) | **Permagent** |
| **events.rs** | GET `/events` (WebSocket) | **Permagent** |
| **attachments.rs** | `/api/sessions/{id}/upload`, `/api/sessions/{id}/attachments/{id}` | **Permagent** |
| **workspaces.rs** | Workspace CRUD | **Permagent** |
| **status.rs** | GET `/status`, `/system_info`, `/diagnostics/{id}` | Goose |
| **setup.rs** | Setup wizard routes | Goose |
| **prompts.rs** | Prompt management | Goose |
| **skills.rs** | Skill discovery | Goose |
| **integrations.rs** | Integration management | Goose |
| **features.rs** | Feature flags | Goose |
| **sampling.rs** | MCP sampling messages | Goose |
| **telemetry.rs** | Telemetry submission | Goose |
| **tunnel.rs** | SSH tunnel management | Goose |
| **local_inference.rs** | Local model management (feature-gated) | Goose |
| **recipe_utils.rs** | Recipe helper functions | Goose |
| **utils.rs** | Shared route utilities | Goose |
| **errors.rs** | Error response types | Goose |
| **mod.rs** | Router configuration + UI serving | Goose + Permagent UI serving |

### Session Management

**`session_manager.rs`** — CRUD for sessions, backed by Spectral SQLite DB.

| Method | Purpose |
|--------|---------|
| `create_session()` | Create with working_dir, name, type, mode |
| `get_session()` | Retrieve metadata ± conversation |
| `list_sessions()` / `list_sessions_by_types()` | Query sessions |
| `delete_session()` | Remove from DB |
| `fork_session()` / `truncate_conversation()` | History manipulation |
| `search_chat_history()` | FTS search (via `chat_history_search.rs`) |
| `import_session()` / `export_session()` | JSON serialization |

**Session types:** User, Scheduled, SubAgent, Hidden, Terminal, Gateway, Acp.

### Agent Execution

**`execution/manager.rs`** — `AgentManager` with LRU cache (100 sessions max). One `Agent` per session_id. Agents restored from DB state (provider, extensions, mode, working_dir).

### Event Bus (per-session SSE)

**`session_event_bus.rs`** — Per-session broadcast channel with 512-event circular replay buffer. Monotonic sequence IDs for Last-Event-ID reconnection. 500ms heartbeat pings.

### AppState

```
AppState {
    agent_manager: Arc<AgentManager>,          // Goose
    tunnel_manager: Arc<TunnelManager>,        // Goose
    gateway_manager: Arc<GatewayManager>,      // Permagent
    brain: Option<Arc<spectral::Brain>>,       // Permagent
    session_buses: HashMap<String, SessionEventBus>,  // Goose
    recipe_file_hash_map, extension_loading_tasks,     // Goose
}
```

---

## Surface 4: Settings & Configuration

### Frontend Settings UI

**Goose2 (primary Tauri desktop UI):**
`ui/goose2/src/features/settings/ui/SettingsModal.tsx` — 9 sections:

| Section | Components | Storage |
|---------|-----------|---------|
| Appearance | `AppearanceSettings.tsx` (theme, accent, density) | localStorage |
| Providers | `ProvidersSettings.tsx`, `ModelProviderRow.tsx`, `ModelProviderPanels.tsx`, `AgentProviderCard.tsx` | config.yaml + keyring |
| Extensions | (via config API) | config.yaml |
| Voice | `VoiceInputSettings.tsx`, `LocalWhisperModels.tsx` | config.yaml |
| General | Language preference | goose store |
| Projects | Archive/restore projects | React stores |
| Chats | Archive/restore sessions | React stores |
| Doctor | `DoctorSettings.tsx`, `DoctorCheckRow.tsx` | read-only |
| About | App info | read-only |

**Command Center (secondary web UI):**
`ui/command-center/src/components/settings/SettingsView.tsx` — Tabs: Providers, Integrations, About.

### Backend Configuration Routes

`config_management.rs`:

| Endpoint | Purpose |
|----------|---------|
| POST `/config/upsert` | Save key-value pairs |
| POST `/config/remove` | Delete config keys |
| POST `/config/read` | Read single value (secrets masked) |
| GET `/config` | Get all config |
| GET/POST/DELETE `/config/extensions/*` | Extension CRUD |
| GET `/config/providers` | List providers with status |
| GET `/config/providers/{name}/models` | Fetch provider models |

### Rust Configuration Layer

`crates/goose/src/config/`:

| Module | Purpose |
|--------|---------|
| `base.rs` | `Config` struct — YAML file + keyring + env var overrides |
| `extensions.rs` | Extension config CRUD |
| `permission.rs` | `PermissionManager` — tool permission levels (AlwaysAllow/AskBefore/NeverAllow) in `permission.yaml` |
| `paths.rs` | Permagent path resolution (`~/.permagent/`) |
| `goose_mode.rs` | Operating mode config |
| `experiments.rs` | Feature flags |
| `declarative_providers.rs` | Custom provider definitions |
| `signup_*.rs` | OAuth flow handlers |

### File System Layout

| Path | Purpose |
|------|---------|
| `~/.permagent/config.yaml` | Main configuration |
| `~/.permagent/secrets.yaml` | Secrets fallback (when keyring unavailable) |
| `~/.permagent/permission.yaml` | Tool permissions |
| `~/.permagent/spectral/permagent.db` | Spectral SQLite DB |
| `~/.permagent/brain/` | Spectral Brain (memory.db, graph.kz, ontology.toml) |
| `~/.permagent/logs/` | Daemon logs |
| `~/.permagent/uploads/` | Session attachments |
| System keyring (service="permagent") | Primary secret storage |

### Environment Variables (Permagent-specific)

| Variable | Purpose |
|----------|---------|
| `PERMAGENT_CONFIG` | Config file path override |
| `PERMAGENT_SPECTRAL_DB` | Spectral DB path override |
| `PERMAGENT_DISABLE_KEYRING` | Force file-based secrets |
| `PERMAGENT_UI_DIR` | UI dist directory override |
| `PERMAGENT_PATH_ROOT` | Base directory override (testing) |

---

## Surface 5: Permagent-Specific Additions

### 5.1 Spectral Brain (Long-term Memory)

**Dependency:** `spectral = { git = "https://github.com/make-tuned-unit/spectral", rev = "66cb19a" }`

| Component | File | Purpose |
|-----------|------|---------|
| Brain mount | `state.rs:56-125` | `spawn_blocking` to build Brain at daemon startup |
| Recall (Phase 3) | `session_events.rs`, `reply.rs` | Query brain before model invocation; top 3 hits with `signal_score >= 0.7` injected into system prompt |
| Remember (Phase 4) | `session_events.rs`, `reply.rs` | Store user+assistant text after turn completion; `tokio::spawn` (non-blocking) |
| Schema | `spectral_schema.rs` (799 lines) | 17 tables, FTS, migrations v2→v6 |
| Ontology | `assets/ontology.toml` (81 lines) | v1.0 entity types (person, project, chat_session, skill, topic) + predicates |
| Smoke tests | `tests/spectral_smoke.rs` (159 lines) | 3 tests: round-trip, schema-only, live brain |
| Paths | `config/paths.rs` | `brain_dir()`, `brain_ontology()`, `spectral_dir()`, `spectral_db()` |

### 5.2 System Prompt Identity

`prompts/system.md` line 1: `"You are Permagent, a persistent AI agent with spectral memory."`

Fallback in `prompt_manager.rs:164` if template render fails: same string.

No separate persona configuration system. No "Hank" identity found in codebase. The `ThreadMetadata.persona_id` field exists as a placeholder (never set or read).

### 5.3 Global Event Bus

| Component | File |
|-----------|------|
| Event types | `events/mod.rs` — `PermagentEvent` with types: DaemonStarted, DaemonStopped, TaskCreated, TaskStarted, TaskCompleted, MemoryAdded |
| WebSocket route | `routes/events.rs` — GET `/events` with replay support |
| Emitters | `tasks/mod.rs`, `state.rs` |

### 5.4 Gateway Manager

`routes/gateway.rs` — Slack/Discord multi-gateway integration with pairing codes.

### 5.5 Attachments

`routes/attachments.rs` + `attachments.rs` — File upload/download for session messages. Stored in `~/.permagent/uploads/{session_id}/`.

### 5.6 Workspaces

`workspaces.rs` — 4 seeded presets: Chat, Skills, Trace, Brain. Brain workspace added in schema migration v4→v5.

### 5.7 Task Logger

`tasks/mod.rs` — Logs tool invocations to Spectral `tasks` table with `argument_shape_hash` for repetition detection. Emits `PermagentEvent`s.

### 5.8 Thread Manager

`session/thread_manager.rs` — Multi-threaded conversation support within sessions. Contains `persona_id` and `project_id` placeholder fields.

### 5.9 Brain UI (Placeholder)

`ui/command-center/src/components/brain/BrainPanel.tsx` (19 lines) — Shows "Memory inspector and knowledge graph live here. Coming in Phase 2 Track 7."

### 5.10 Crate Rename

Package `goose` renamed to `permagent` in `crates/goose/Cargo.toml` (v1.31.0). Directory names retain `goose-*` prefixes. Daemon binary: `permagentd`. CLI binary: `permagent-cli`.

---

## Orphaned Code

### 1. `crates/goose-server/src/routes/agents_registry.rs` (41 lines)

**Status:** Completely unreachable. File exists but is NOT declared as a module in `routes/mod.rs`. Never compiled into the binary.

**Contents:** Defines `GET /api/agents` returning an empty `Vec<AgentRegistryEntry>`. Struct includes sprite/visual fields (`home_x`, `home_y`, `color`, `accent`, `hair`, `visor`, `outfit`) suggesting a planned "world view" agent visualization feature.

**Comment in file:** "Phase 1: empty registry — agents will be populated by the setup wizard"

### 2. `crates/goose/src/session/legacy.rs` (142 lines)

**Status:** Declared as `mod legacy` (private) in `session/mod.rs` but never exported or used outside the module. Contains JSONL session file loading functions (`load_session`, `list_sessions`) for backward compatibility with pre-Spectral file-based session storage.

**Functions are `pub` but module is private** — only the module's own `#[cfg(test)]` block exercises the code. No other module calls into `legacy::*`.

### 3. `ThreadMetadata.persona_id` (1 field)

**Status:** Defined in `session/thread_manager.rs:28` as `pub persona_id: Option<String>`. Never set or read anywhere in the codebase. Placeholder for a planned persona system that does not yet exist.

---

## Verification Checklist

- [x] Output file exists at `docs/architecture/INHERITANCE_AUDIT.md`
- [x] All five surfaces have content
- [x] Orphaned code section has 3 entries
- [x] Executive summary is 3-5 sentences
- [x] No code was modified during the audit
