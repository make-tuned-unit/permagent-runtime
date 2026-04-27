# Permagent Capability Inventory

Generated 2026-04-27 for Phase 2 planning.

---

## What the UI surfaces today

| Component | Feature |
|-----------|---------|
| Sidebar | Workspace switcher (Work/World/Build), Settings button, connection status indicator |
| ChatView | SSE-streaming chat with an agent session, message bubbles, tool call rendering, streaming indicator |
| SkillsPanel | List skills, detail view, editor, execution history, proposal banner (accept/dismiss) |
| TerminalManager | Multi-tab embedded terminal via portable-pty, xterm.js with WebGL renderer |
| Browser | Multi-tab embedded Chromium webview via Tauri child webview, URL bar, navigation |
| WorkspaceRenderer | Layout engine for split/panel tree with resizable panes via react-resizable-panels |
| WorldView | Placeholder panel (no real content yet) |
| ExecutionTrace | Placeholder panel for execution trace (no real content yet) |
| SettingsView | Gmail OAuth credential entry and connect/disconnect |
| EventLog | Event stream display with filtering (rendered in Trace workspace but fed from WebSocket) |

**API endpoints the UI actually calls (20 of ~90+ available):** getHealth, getSessions, getSession, deleteSession, createSession, sendReply, getConfig, upsertConfig, getSkills, createSkill, updateSkill, deleteSkill, getSkillExecutions, dismissSkillProposal, getWorkspaces, getWorkspace, getActiveWorkspace, setActiveWorkspace, updateWorkspaceLayout, getStateSnapshot.

---

## High-leverage gaps (top 10)

| # | Capability | Current state in code | Missing UI affordance | Effort | User gain |
|---|-----------|----------------------|----------------------|--------|-----------|
| 1 | **Scheduled jobs / cron** | Full scheduler (`tokio-cron-scheduler`) with cron expressions, pause/unpause, recipe scheduling. 14 REST endpoints under `/schedule/*`. | Zero UI. No way to create, view, or manage scheduled jobs. | M | Recurring automations: "check email every morning", "sync repo at 6pm" |
| 2 | **Memories (knowledge palace)** | Spectral tables: memories, memories_fts, knowledge_graph, knowledge_graph_fts. Spatial metaphor (wing/hall/room). Signal scoring, supersession, validity windows. Builtin MCP memory server with remember/retrieve/remove. | Zero UI. No memory browser, no way to view/edit/search what the agent remembers. | L | Inspect and correct agent's long-term memory; build trust in persistence |
| 3 | **Recipes** | Full recipe system: create, parse, encode/decode deeplinks, schedule, scan. Recipe parameters, YAML export. 10 REST endpoints under `/recipes/*`. | Zero UI. No recipe builder, library, or import/export flow. | M | Shareable agent workflows, one-click automations |
| 4 | **Session history browser** | Sessions list, search (FTS), export/import JSON, fork with timestamp. All wired in REST API. | UI calls getSessions but has no session picker, search, or history view. Cannot resume old conversations. | S | Browse past conversations, continue where you left off, search across history |
| 5 | **Dictation / speech-to-text** | Whisper inference via Candle (whisper.rs, providers.rs). Tokenizer data bundled. Audio decoding via `symphonia` (FLAC, MP3, WAV, OGG). Resampling via `rubato`. | Zero UI. No microphone button, no voice input anywhere. | M | Hands-free interaction; accessibility |
| 6 | **Local model inference** | Full download/manage pipeline for local LLMs via `llama-cpp-2` with Metal/CUDA. Model search, sync featured models, per-model settings. 7 REST endpoints under `/local-inference/*`. | Zero UI. No model manager, download progress, or local-model picker. | L | Run Permagent fully offline; no API key needed |
| 7 | **Extension manager** | Runtime add/remove extensions. Extension catalog. Tool inspection. REST endpoints: add_extension, remove_extension, list tools. Platform extension: extensionmanager with `search_available_extensions`, `manage_extensions`, `list_resources`, `read_resource`. | Zero UI. No extension marketplace or toggle panel. | M | User installs/removes capabilities without editing config files |
| 8 | **Visualizations** | Builtin MCP autovisualiser with 8 chart types: `render_sankey`, `render_radar`, `render_donut`, `render_treemap`, `render_chord`, `render_map`, `render_mermaid`, `show_chart`. | Agent can generate charts but results are not rendered in any UI panel. | S | Data visualization inline in chat or in a dedicated panel |
| 9 | **Orchestrator / sub-agents** | Platform extension orchestrator: `list_sessions`, `view_session`, `start_agent`, `send_message`, `interrupt_agent`. Multi-agent delegation. | Zero UI. No agent roster, no sub-agent status, no delegation visualization. | L | See what agents are running, inspect delegation chains |
| 10 | **Document processing** | computercontroller MCP: `xlsx_tool` (Excel read/write/search), `docx_tool` (Word extract/update/append), `pdf_tool` (PDF extract text/images). Backed by `docx-rs`, `lopdf`, `umya-spreadsheet` crates. | Agent can process documents but no file viewer, preview, or upload affordance. | M | Drag-and-drop document processing; preview results |

---

## Full inventory

### 1. Rust crate capabilities

**Document processing:**
- `docx-rs` 0.4.7 — Read/write Word .docx documents (paragraph, run, table structures). Used by computercontroller `docx_tool`. No UI affordance.
- `lopdf` 0.36.0 — Read/write PDF files. Used by computercontroller `pdf_tool`. No UI affordance.
- `umya-spreadsheet` 2.2.3 — Excel spreadsheet parsing and generation. Used by computercontroller `xlsx_tool`. No UI affordance.
- `pulldown-cmark` 0.13.0 — Markdown parsing. Used for rendering in chat/tutorials.
- `zip` 0.8 (deflate) — ZIP archive handling. Used for diagnostics export.

**Media / Audio:**
- `symphonia` 0.5 (all features, optional) — Audio decoding for FLAC, MP3, WAV, OGG, etc. Used by Whisper dictation pipeline. No UI affordance.
- `rubato` 0.16 (optional) — Audio resampling for Whisper input preprocessing. No UI affordance.
- `image` 0.24.9 — Image format support. Used for screenshot handling in computercontroller.

**AI/ML:**
- `candle-core`, `candle-nn`, `candle-transformers` 0.9 (optional, Metal/CUDA) — Hugging Face Candle framework for local neural inference (Whisper, embeddings). No UI affordance.
- `llama-cpp-2` 0.1.143 (optional, Metal/CUDA/sampler/mtmd) — Local LLM inference via llama.cpp bindings. Powers the `/local-inference/*` endpoints. No UI affordance.
- `tokenizers` 0.21.0 (optional) — Hugging Face tokenizers for NLP models. Used for token counting.
- `tiktoken-rs` 0.6.0 — OpenAI-compatible token counting.

**Code analysis:**
- `tree-sitter` 0.26 + 9 language grammars (Go, Java, JavaScript, Kotlin, Python, Ruby, Rust, Swift, TypeScript) — AST parsing for the `analyze` platform extension. No dedicated UI panel.
- `rayon` 1.10 — Parallel file analysis in the analyze extension.

**Networking/Security:**
- `reqwest` 0.13 (multipart, cookies, gzip, brotli, zstd, http2, stream) — HTTP client for all provider APIs, OAuth, MCP.
- `rustls` 0.23 / `openssl` 0.10 — TLS backends for optional HTTPS.
- `rcgen` 0.13, `pem` 3.0.6 — Self-signed certificate generation.
- `keyring` 3.6.2 — System keychain for credential storage (API keys, OAuth tokens).
- `jsonwebtoken` 10.3.0 — JWT creation/validation for auth flows.
- `sigstore-verify` 0.6 — Sigstore signature verification for extension integrity.
- `sha2` 0.10, `blake3` 1.5 — Hashing for content addressing and integrity.
- `oauth2` 5.0 — OAuth 2.0 client library.

**AWS integration:**
- `aws-sdk-bedrockruntime` 1.120.0 — Amazon Bedrock LLM inference.
- `aws-sdk-sagemakerruntime` 1.62.0 — SageMaker model invocation.
- `aws-config` 1.8.12 — AWS credential and region management.

**MCP protocol:**
- `rmcp` 1.5.0 (client, server, transport) — Model Context Protocol implementation.
- `agent-client-protocol-schema` 0.11 (unstable) — ACP schema definitions.
- `sacp` 11.0.0 — Symposium ACP implementation.

**Terminal/Desktop:**
- `portable-pty` 0.8 — PTY spawning for the embedded terminal. UI surfaces this.
- `tauri` 2.x (unstable) — Desktop app framework with child webview support. UI surfaces this.

**Storage:**
- `sqlx` 0.8 (sqlite, chrono, json, migrate) — Spectral database (SQLite with WAL).
- `tar` 0.4.45, `bzip2` 0.5 — Archive handling.

**CLI:**
- `clap` 4 (derive), `clap_mangen`, `clap_complete` — CLI with man pages and shell completions.
- `dialoguer` 0.11 (fuzzy-select), `cliclack` 0.3.5 — Interactive prompts.
- `bat` 0.26.1 — Syntax-highlighted output.
- `rustyline` 15.0 — Readline for REPL mode.

**Observability:**
- `opentelemetry` 0.31 + `opentelemetry-otlp` + `tracing-opentelemetry` — Full OpenTelemetry pipeline. No UI affordance.

**Scheduling:**
- `tokio-cron-scheduler` 0.14.0 — Cron job scheduling. Powers `/schedule/*` endpoints. Zero UI.

**Other notable:**
- `arboard` 3 — Clipboard access.
- `minijinja` 2.12.0 — Template engine for recipe rendering.
- `dashmap` 6.1, `lru` 0.16 — Concurrent data structures and caching.

### 2. Platform extensions (16 total, ~70 tools)

#### Default-enabled (9 extensions):

| Extension | Tools | What it does | UI surfaces it? |
|-----------|-------|-------------|-----------------|
| **developer** | `write`, `edit`, `shell`, `tree` | File CRUD, shell execution, directory tree listing. The primary coding toolkit. | Agent-only. Chat shows tool call results as raw cards. |
| **analyze** | `analyze` | Tree-sitter AST analysis: directory overview, file semantic detail, symbol call graphs with configurable depth. | Agent-only. No code analysis panel. |
| **todo** | `todo_write` | Persists a todo/task list across turns. WARNING: replaces entire content, not append-only. | Agent-only. No todo panel. |
| **apps** | `create_app_content`, `iterate_app`, `delete_app`, `list_apps`, `create_app`, `update_app_content` | Creates and manages sandboxed HTML/CSS/JS mini-apps in separate windows. | Hidden. No app gallery in UI. |
| **extensionmanager** | `search_available_extensions`, `manage_extensions`, `list_resources`, `read_resource` | Runtime extension discovery, enable/disable, resource access. | Hidden. No extension panel. |
| **summon** | `load`, `delegate` | Loads knowledge sources into context. Delegates tasks to sub-agents (supports async with task tracking). | Agent-only. |
| **skills** | `load_skill` | Discovers and loads skill instructions from filesystem/builtins. | Partially surfaced. SkillsPanel shows skills list; `load_skill` is agent-initiated. |
| **tom** | _(no tools)_ | "Top Of Mind" — injects custom context via GOOSE_MOIM_MESSAGE_TEXT and GOOSE_MOIM_MESSAGE_FILE env vars. | Hidden. No UI to set MOIM context. |

#### Opt-in (4 extensions):

| Extension | Tools | What it does | UI surfaces it? |
|-----------|-------|-------------|-----------------|
| **chatrecall** | `chatrecall` | FTS search across past conversations. Can load session summaries (first/last 3 messages). | Agent-only. No search UI. |
| **summarize** | `summarize` | Loads files/directories and generates LLM summary in single call (more efficient than sub-agent). Respects .gitignore, filter by extension. | Agent-only. |
| **orchestrator** | `list_sessions`, `view_session`, `start_agent`, `send_message`, `interrupt_agent` | Multi-agent orchestration: spawn, manage, and communicate with sub-agents. | Hidden. No agent roster or delegation UI. |
| **code_execution** | `list_functions`, `get_function_details`, `execute_typescript`, `execute_bash` | Sandboxed code execution (saves tokens vs. LLM-mediated tool calls). Feature-gated behind "code-mode". | Hidden. No execution panel. |

### 3. Builtin MCP servers (crates/goose-mcp)

| Server | Tools | Capability | UI surfaces it? |
|--------|-------|-----------|-----------------|
| **autovisualiser** | `render_sankey`, `render_radar`, `render_donut`, `render_treemap`, `render_chord`, `render_map`, `render_mermaid`, `show_chart` | 8 interactive chart/diagram types: flow diagrams, spider charts, pie charts, treemaps, chord diagrams, maps with markers, Mermaid syntax, and line/scatter/bar charts. | Agent can generate but no rendering panel in UI. |
| **computercontroller** | `web_scrape`, `automation_script`, `computer_control`, `xlsx_tool`, `docx_tool`, `pdf_tool`, `cache` | Web scraping, OS automation (AppleScript/PowerShell/shell), UI control (Peekaboo on macOS), Excel/Word/PDF processing, file cache management. | Agent-only. No document viewer or automation panel. |
| **memory** | `remember_memory`, `retrieve_memories`, `remove_memory_category`, `remove_specific_memory` | Categorized persistent memory with tags. Global and local scope. | Agent-only. No memory browser. |
| **tutorial** | `load_tutorial` | Loads interactive tutorial markdown by name. | Agent-only. No tutorial UI. |

**MCP transport types supported:** stdio (builtin extensions), HTTP/SSE (streaming tool results), arbitrary external MCP server URIs via config. Any MCP-compatible server can be added.

**Currently wired integrations:** Gmail only (OAuth in SettingsView).

### 4. Spectral schema and runtime state

| Table | What it stores | UI reads? | UI writes? |
|-------|---------------|-----------|------------|
| `users` | Single default user (Phase 1), active_workspace_id | Indirectly | Via workspace switch |
| `sessions` | Chat sessions with metadata, tokens, mode, provider, schedule_id, recipe | Yes (list, get) | Yes (create, delete, name) |
| `messages` | Per-session message history with role, content, metadata | Yes (via session get) | Yes (via reply) |
| `threads` | Thread grouping for ACP sessions | No | No |
| `thread_messages` | Messages within threads | No | No |
| `memories` | Long-term memories with wing/hall/room taxonomy, embeddings, signal scores, validity windows, supersession | **No** | **No** |
| `memories_fts` | Full-text search index for memories (key + content) | **No** | **No** |
| `knowledge_graph` | Subject-predicate-object triples with validity windows, confidence scores | **No** | **No** |
| `knowledge_graph_fts` | Full-text search for knowledge graph (subject + predicate + object) | **No** | **No** |
| `tasks` | Agent task log (tool, argument shape hash, steps, status, timing) | Indirectly (snapshot) | Via agent execution |
| `skills` | Learned skills with triggers, definitions, version, source task | Yes (list, CRUD) | Yes |
| `skill_executions` | Execution history per skill (status, input, output, timing) | Yes | No |
| `skill_triggers` | Trigger configs for auto-skills (type, config, last triggered) | No | No |
| `skill_dismissals` | Dismissed skill proposals (suppresses for 30 days) | Yes (dismiss) | Yes |
| `integrations` | OAuth integration status per provider (scopes, last sync, errors) | Via Tauri commands | Via Tauri commands |
| `workspaces` | Layout JSON definitions, sort order, icons, is_default | Yes | Yes |
| `provider_inventory_entries` | Provider catalog metadata (family, refresh timestamps) | No | No |
| `provider_inventory_models` | Available models per provider (context window, capabilities, preferred) | No | No |
| `schema_version` | Schema migration tracking | No | No |

**SQL views:**
- `current_memories` — Active memories (valid_until IS NULL)
- `current_knowledge` — Active knowledge triples (valid_until IS NULL)
- `recent_tasks` — Last 100 completed tasks
- `repetition_candidates` — Tasks with ≥2 occurrences of same (user, tool, arg shape) in 7 days — powers skill auto-detection

**Tables the UI never touches:** memories, memories_fts, knowledge_graph, knowledge_graph_fts, threads, thread_messages, skill_triggers, provider_inventory_entries, provider_inventory_models.

### 5. Background and meta capabilities

**Scheduler/Cron (14 endpoints, zero UI):** Full cron-based job scheduler via `tokio-cron-scheduler`. `SchedulerTrait` with add/remove/pause/unpause/list/update/run_now/kill/inspect. Recipes can be scheduled. Sessions track `schedule_id`.

**Session forking (API exists, zero UI):** `POST /sessions/{id}/fork` with `{ timestamp?, truncate, copy }`. Creates branched conversation at a point in time. Useful for "what if" exploration.

**Session search (API exists, zero UI):** `GET /sessions/search` with FTS query, date range, session type filters. Powered by SQLite FTS.

**Session export/import (API exists, zero UI):** `GET /sessions/{id}/export` returns full JSON. `POST /sessions/import` restores.

**Recipes (10 endpoints, zero UI):** Create, parse, encode/decode deeplinks, schedule, scan directory for recipes, YAML export, slash commands. Supports parameterized templates via minijinja.

**Sampling (MCP bridge, zero UI):** `POST /sessions/{id}/sampling/message` — direct LLM completion without agent loop. No temperature/top-p controls in UI.

**Tunnel (3 endpoints, zero UI):** Remote access via Lapstone proxy. Start/stop/status. Allows controlling agent from outside LAN.

**Gateway (5 endpoints, zero UI):** Pair with Telegram (formatting module exists), start/stop/status/remove. Allows controlling agent from messaging apps.

**Provider management (15+ endpoints, minimal UI):** Full provider catalog, OAuth flows, model listing, custom providers, permissions, canonical model info. UI only calls getConfig/upsertConfig.

**Action required (2 endpoints, zero UI):** `GET /action_required` + `POST /action_required/skip` — tool confirmation flow. Agent can request human approval before executing tools. No UI affordance.

**Features endpoint:** `/features` returns feature flags. Not used by UI.

**Skills auto-detection:** `repetition_candidates` view → `skill_proposed` event → `SkillPromptBanner` in chat UI. This IS surfaced, but threshold/window cannot be configured from UI.

**Panel types:** Store defines `ToolType = 'chat' | 'skills' | 'trace' | 'world' | 'terminal' | 'browser'`. All six are in the three presets, but `world` and `trace` are placeholder implementations.

---

## Recommended Phase 2 UI priorities

1. **Session history browser.** Effort: S. The API is already complete (list, search, fork, export/import). Adding a session picker sidebar and search bar to the chat view gives users the ability to resume conversations, which is fundamental to the "remembers context across sessions" promise. This is the highest-ROI change because the backend is done and the UI gap is the only thing between the user and the feature.

2. **Memory inspector.** Effort: M-L. The Spectral memory tables (memories, knowledge_graph) with FTS indexes and the MCP memory server (remember/retrieve/remove) are fully functional. A read-only memory browser panel — searchable, filterable by wing/hall/room — would be the single most differentiating feature vs. every other AI assistant. Users need to see what the agent remembers, correct it, and build trust in persistence. This is what "World" view should become.

3. **Scheduled jobs UI.** Effort: M. The scheduler REST API is complete with 14 endpoints. A "Schedules" panel showing active/paused jobs with create/edit/delete would unlock the "acts on your behalf" value prop. Cron expression builder, last-run status, next-run time. Combine with recipes for maximum leverage.

4. **Provider and model settings.** Effort: S. 15+ config endpoints exist. The Settings view only has Gmail OAuth. Adding provider selection, model picker, API key management, and sampling parameter controls would make the entire setup process possible within the UI. Currently requires the CLI wizard.

5. **Visualization rendering.** Effort: S. The autovisualiser MCP server already generates 8 chart types. Rendering chart output inline in chat (or in a dedicated panel) instead of raw JSON would make data analysis conversations dramatically more useful. The chart data is already structured — just needs a rendering layer.

6. **Recipes library.** Effort: M. The recipe system supports create, parse, encode deeplinks, schedule, and YAML export. A recipe gallery with import/share/schedule-from-recipe would make Permagent's automation capabilities accessible to non-technical users via shareable one-click workflows.

7. **Extension marketplace.** Effort: M. The extensionmanager platform extension and REST endpoints for add/remove/list extensions already work. A toggle panel showing installed extensions, available ones, and an install button would let users customize their agent without editing config.yaml.

8. **Document processing UI.** Effort: M. The computercontroller MCP has xlsx_tool, docx_tool, pdf_tool backed by dedicated Rust crates. A file drop zone + preview panel would let users drag documents in and see the agent process them, rather than the current invisible tool-call flow.

9. **Local model manager.** Effort: L. The local-inference subsystem can download, manage, and run models via llama.cpp with Metal acceleration. A model browser with download progress, size info, and a selector in Settings would fulfill the "runs locally on your Mac" promise for users without API keys.

10. **Dictation / voice input.** Effort: M. Whisper inference via Candle is fully implemented with tokenizer data. Audio decoding via symphonia supports all major formats. Adding a microphone button to the chat input that captures audio and transcribes locally would be a strong differentiator for hands-free use.

---

## Goose UI references for Phase 2 gaps

Upstream: `block/goose` fetched as `upstream/main`. Goose uses Electron + React + react-router-dom + shadcn/ui components + react-intl i18n. Permagent uses Tauri + React (no router, no i18n). Below: what Goose built, what ports, and what doesn't.

### Gap 1: Session history browser

**Goose source files:**
- `ui/desktop/src/components/sessions/SessionsView.tsx` — Top-level view with list/detail toggle
- `ui/desktop/src/components/sessions/SessionListView.tsx` — Session list with search, grouped by date (Today/Yesterday/This Week/Older), edit name, delete, export JSON, import, fork (duplicate), inline rename. Uses `SearchView` component for FTS. Pagination via scroll.
- `ui/desktop/src/components/sessions/SessionHistoryView.tsx` — Single-session detail view: full conversation replay with `ProgressiveMessageList`, search within session, resume button, share link, metadata header (date, working dir, tokens, extensions used).
- `ui/desktop/src/components/sessions/SessionItem.tsx` — Individual session card: name, message count, date, working dir, extensions, actions dropdown (edit/delete/export/duplicate/open in new window).
- `ui/desktop/src/components/sessions/SessionsInsights.tsx` — Token usage summary (total sessions, total tokens).

**Data flow:** `listSessions()` → REST GET `/sessions` (already called by Permagent UI). `searchSessions(query)` → GET `/sessions/search`. `exportSession(id)` → GET `/sessions/{id}/export`. `forkSession(id)` → POST `/sessions/{id}/fork`. `importSession(json)` → POST `/sessions/import`. All of these REST endpoints exist in permagentd.

**Portable?** Yes. The React components use shadcn/ui (Card, Button, ScrollArea, Dialog) and lucide icons — close matches to Permagent's Tailwind stack. The main differences: (1) react-intl i18n wrappers — strip these, use plain strings. (2) react-router-dom `useLocation`/`useNavigation` — replace with Zustand store `activePanel` state. (3) Electron `window.electron` calls — not present in session components.

**Effort:** S. The API is done. The UI is 5 files totaling ~800 lines. Strip i18n, replace routing with store navigation, restyle to dark theme. The date grouping util (`groupSessionsByDate`) and search highlighting (`SearchHighlighter`) can be copied directly.

### Gap 2: Memory inspector

**Goose source files:** None found. Goose has no memory UI. The memory MCP server (`remember_memory`, `retrieve_memories`, `remove_memory_category`, `remove_specific_memory`) operates agent-side only. The Spectral tables (memories, knowledge_graph, FTS indexes) are a Permagent addition.

**Portable?** N/A — this is greenfield.

**Effort:** L. Need to: (1) add REST endpoints for reading/searching memories and knowledge_graph (none exist in the server routes today — the agent writes to these tables but there's no HTTP API to read them), (2) build a panel UI with search, category/wing/hall/room filters, memory cards, and an edit/delete flow. The `WorldView` placeholder is the natural home for this.

### Gap 3: Scheduled jobs

**Goose source files:**
- `ui/desktop/src/components/schedule/SchedulesView.tsx` — Full scheduler UI: list of ScheduleCards showing job ID, human-readable cron (via `cronstrue` library), running/paused status badges, last run timestamp. Actions: create, edit cron, pause/unpause, kill running, inspect, delete. Uses `ScheduleModal` for create/edit.
- `ui/desktop/src/components/schedule/ScheduleModal.tsx` — Create/edit dialog with recipe selector and `CronPicker`.
- `ui/desktop/src/components/schedule/CronPicker.tsx` — Visual cron expression builder: period dropdown (minute/hour/day/week/month/year), conditional inputs for day-of-week, hour, minute, etc. Converts human selections to cron string. Validates with `cronstrue`.
- `ui/desktop/src/components/schedule/ScheduleDetailView.tsx` — Single schedule detail: past sessions list with tokens/duration, recipe source.
- `ui/desktop/src/schedule.ts` — API client wrapping the 10 schedule REST endpoints.

**Data flow:** `listSchedules()` → GET `/schedule/list`. `createSchedule({id, recipe, cron})` → POST `/schedule/create`. `pauseSchedule(id)` → POST `/schedule/{id}/pause`. `killRunningJob(id)` → POST `/schedule/{id}/kill`. All endpoints exist in permagentd.

**Portable?** Yes, with caveats. The ScheduleModal depends on the Recipes system (selecting a recipe to schedule), so it pulls in recipe components. The CronPicker is self-contained and directly portable. The SchedulesView uses shadcn Card/Button/ScrollArea — easy restyle.

**Effort:** M. The CronPicker (~200 lines) ports cleanly. The SchedulesView (~300 lines) needs i18n stripping and routing replacement. The ScheduleModal requires the recipe selector, which pulls in the recipe system (Gap 4 from the inventory). For a minimal version: port SchedulesView + CronPicker + a text-input-for-recipe-ID stub, then upgrade to full recipe selector later. Add `cronstrue` npm dependency.

### Gap 4: Provider/model settings

**Goose source files:**
- `ui/desktop/src/components/settings/models/ModelsSection.tsx` — Current model display card with provider name, model name, and "Switch Model" / "Reset Provider" buttons.
- `ui/desktop/src/components/settings/models/subcomponents/SwitchModelModal.tsx` — Modal dialog: provider dropdown (fetched from `/config/providers`), model dropdown (fetched from `/config/providers/{name}/models`), thinking-level/effort selector for Claude/OpenAI reasoning models, custom model ID text input, API key entry for new providers. Full model switching flow.
- `ui/desktop/src/components/settings/models/modelInterface.ts` — Model type definitions, provider metadata (display names, icons), model fetching utilities.
- `ui/desktop/src/components/settings/models/predefinedModelsUtils.ts` — Predefined model list from environment, display logic.
- `ui/desktop/src/components/onboarding/ProviderSelector.tsx` — First-run provider picker: "Use Free/Local" vs "Connect to Provider" cards, provider dropdown, custom provider dialog. Calls `/config/providers` and `/config/set_provider`.
- `ui/desktop/src/components/onboarding/ProviderConfigForm.tsx` — API key entry + validation per provider.
- `ui/desktop/src/components/settings/extensions/ExtensionsSection.tsx` — Extension toggle list with add/edit/remove modals. Uses `ConfigContext` for extension CRUD.

**Data flow:** `providers()` → GET `/config/providers`. `setProvider(name)` → POST `/config/set_provider`. Model list → GET `/config/providers/{name}/models`. API key → POST `/config/upsert`. All endpoints exist.

**Portable?** Partially. The model switching modal is ~300 lines and uses `ConfigContext` and `ModelAndProviderContext` — Goose-specific React contexts that wrap config read/write. These would need to be replaced with direct `api.getConfig()` / `api.upsertConfig()` calls. The ProviderSelector onboarding flow is ~200 lines and mostly self-contained. The ExtensionsSection depends on a complex `extension-manager.ts` layer.

**Effort:** S-M. For a minimal "switch model" panel: port SwitchModelModal (~300 lines), replace context calls with direct API, add to SettingsView. For full extension management: M-L, as the extension modal system is ~800 lines across 6 files. Recommended: start with model switching only (S), add extensions later (M).

### Gap 5: Visualization rendering

**Goose source files:**
- `ui/desktop/src/components/McpApps/McpAppRenderer.tsx` — The core rendering component. Uses `@mcp-ui/client` SDK's `AppRenderer` class to render MCP app content (including autovisualiser chart output) inside sandboxed iframes. Handles resource fetching, sandbox proxy, CSP, display modes (inline/fullscreen/pip), tool calling from within the app, and bidirectional message passing.
- `ui/desktop/src/components/MCPUIResourceRenderer.tsx` — Renders `EmbeddedResource` content returned by MCP tool calls. Uses `@mcp-ui/client`'s `UIResourceRenderer` for rendering resources with action handling (links, notifications, prompts, tool calls).
- `ui/desktop/src/components/McpApps/toolsCache.ts` — Caches tool definitions for MCP apps.
- `ui/desktop/src/components/McpApps/useDisplayMode.ts` — Display mode state management (inline → fullscreen → PiP transitions).

**How autovisualiser charts render:** The autovisualiser MCP server returns HTML/CSS/JS content as an `EmbeddedResource` in the tool result. `MCPUIResourceRenderer` detects the resource type and creates a sandboxed iframe via `@mcp-ui/client`'s `UIResourceRenderer`. The chart library (embedded in the HTML) renders client-side inside the iframe. There's no Vega-Lite dependency on the host side — the chart is self-contained HTML.

**Data flow:** Agent calls `show_chart` → MCP server returns `CallToolResult` with embedded resource → conversation renderer detects `resource` type in content → `MCPUIResourceRenderer` renders sandboxed iframe → chart displays inline in chat.

**Portable?** Partially. The `@mcp-ui/client` SDK (v6.1.0) is framework-agnostic — it works with any DOM environment. The `McpAppRenderer` uses Electron-specific APIs for standalone mode but the inline/fullscreen modes are pure React+DOM. The `MCPUIResourceRenderer` is simpler and more portable. The main adaptation: Goose's conversation renderer checks for `resource` content type in tool results and delegates to `MCPUIResourceRenderer` — Permagent's `ToolCallCard` would need the same check.

**Effort:** S-M. Minimal path: (1) add `@mcp-ui/client` dependency, (2) create a `ResourceRenderer` component that wraps `UIResourceRenderer`, (3) modify `ToolCallCard.tsx` to detect `resource` content in tool results and render via `ResourceRenderer` instead of raw JSON. This gets charts rendering inline in chat. Full MCP app support (display modes, tool calling from apps, PiP) is M-L.

### Portability summary

| Gap | Goose files | Lines (approx) | Key deps to add | Electron-specific? | Port effort |
|-----|------------|----------------|-----------------|-------------------|-------------|
| 1. Sessions | 5 files | ~800 | none | No | S |
| 2. Memory | 0 files | greenfield | none | N/A | L |
| 3. Schedules | 4 files + API client | ~700 | `cronstrue` | No | M |
| 4. Settings | 6 files (models only) | ~600 | none | No (uses contexts) | S-M |
| 5. Visualization | 2 key files | ~400 (core) | `@mcp-ui/client` | Partially (strip standalone mode) | S-M |

**Common porting work across all gaps:**
- Strip `react-intl` `defineMessages`/`useIntl`/`intl.formatMessage` → plain strings
- Replace `react-router-dom` navigation → Zustand store `activePanel` / `setActivePanel`
- Replace shadcn `import { Card } from '../ui/card'` → Tailwind utility classes (Permagent doesn't use shadcn)
- Replace `useConfig()` context → direct `api.getConfig()` / `api.upsertConfig()` calls
- Replace Electron `window.electron.*` → Tauri `invoke()` where needed (rare in these components)
