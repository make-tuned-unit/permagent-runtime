# State of the Agentic Substrate

*Audit date: 2026-05-03. Baseline commit: ec0ca7278712a2f8805df52b7c3b3d29ef7235b1 (main).*

## Summary

Permagent inherits a **genuinely functional agentic runtime** from Goose. The core loop — provider call, tool dispatch, multi-turn reasoning, session persistence — is fully wired and running in production. The session/message system works, multiple providers are actively used (Anthropic, Moonshot confirmed in DB), the scheduler executes cron-based recipes, subagent delegation is operational, and the MCP client supports external tool servers. The permission system provides per-tool approval routing including an LLM-based read-only judge.

**Correction from initial assessment**: The TaskLogger IS wired into the agent's tool dispatch path (`agent.rs:575-640`). It logs every tool call as a task, computes argument shape hashes, and runs `check_repetition_candidates()` after each completion. The `tasks` table has 0 rows simply because no tool-calling sessions have been exercised yet (all existing sessions are simple chat exchanges). The auto-skills pipeline is complete end-to-end — it will activate the moment a session makes tool calls.

A **separate Spectral Brain** exists at `~/.permagent/brain/memory.db` with **997 memories** and **138,825 constellation fingerprints**. This is an actively working memory system distinct from the empty `memories` table in the Spectral DB. The brain search API queries both systems. The Spectral DB's `memories` and `knowledge_graph` tables (0 rows) appear to be a parallel schema that was designed but never connected to the working Brain system.

The highest-leverage substrate work is connecting the **Spectral DB memory tables to the working Spectral Brain** (or deprecating the duplicate schema), and then running a tool-using session to exercise the auto-skills pipeline.

---

## Status Matrix

| # | Subsystem | Schema | Population | Consumption | API | UI | Status |
|---|-----------|--------|------------|-------------|-----|----|----|
| 1 | Orchestrator / agent loop | Y | Y | Y | Y | Y | **WORKING** |
| 2 | Tools / tool execution | Y | Y | Y | Y | Y | **WORKING** |
| 3 | Skills system | Y | Y (wired) | Partial | Y | Y | **WORKING (unexercised)** |
| 4 | Schedules / cron | Y | Y | Y | Y | Y | **WORKING** |
| 5 | Extensions / MCP | Y | Y | Y | Y | Partial | **WORKING** |
| 6 | Sessions & messages | Y | Y | Y | Y | Y | **WORKING** |
| 7 | Tasks (tool logging) | Y | Y (wired) | Y (wired) | N | N | **WORKING (unexercised)** |
| 8 | Knowledge graph | Y | N | N | Partial | Partial | **SCHEMA ONLY** |
| 9 | Provider routing | Y | Y | Y | Partial | Y | **WORKING** |
| 10 | Credential storage | Y | Y | Y | Partial | Y | **WORKING** |
| 11 | Subagent dispatch | Y | Y | Y | Y | N | **WORKING** |
| 12 | Continuous awareness | Partial | Partial | Partial | N | N | **PARTIALLY WIRED** |
| 13 | Permissions / authorization | Y | Y | Y | Partial | Y | **WORKING** |
| 14 | Recipes / playbooks | Y | Y | Y | Y | Partial | **WORKING** |
| 15 | Observability | Y | Y | Partial | Partial | N | **PARTIALLY WIRED** |
| 16 | Persistence beyond sessions | Y | Y | Y | Y | Y | **WORKING** |

---

## Detailed Subsystem Reports

### 1. Orchestrator / Agent Loop

**Status:** WORKING

**Lives at:** `crates/goose/src/agents/agent.rs`, `crates/goose/src/execution/manager.rs`

**What works:**
- Full think-act-observe loop in `Agent::reply()` (line 1014) and `reply_internal()` (line 1220)
- The loop streams provider responses, dispatches tool calls, collects results, and loops until the model stops requesting tools or hits `max_turns` (default 1000)
- `AgentManager` (singleton via `OnceCell`) holds an LRU cache of up to 100 concurrent sessions, each with its own `Agent` instance
- Per-session cancellation tokens allow interrupting running agents
- Context compaction: automatic conversation summarization when approaching token limits
- `GooseMode` (auto/chat) controls whether tools execute or are presented as plans
- MOIM (Message of Immediate Mind) injection adds per-turn context via env vars
- Retry logic via `RetryManager` handles transient provider failures
- Session naming: auto-generates session names via LLM after initial messages

**What's stubbed or dormant:**
- No explicit state machine (thinking/speaking/idle) — state is implicit in whether `reply()` is streaming
- No autonomous inter-turn execution — each loop starts with a user message via `reply()`

**What's missing:**
- No background agent reasoning between user turns
- No "agent decides to act proactively" mechanism
- No explicit session state field (completed/paused/awaiting_input) — state is derived

**Honest assessment:**
The orchestrator is Goose's strongest subsystem. It's a production-quality agent loop with streaming, tool dispatch, retry, compaction, and cancellation. It handles the full lifecycle of a user interaction competently. The main limitation is that it's entirely reactive — the agent only acts when a user sends a message or a cron fires.

**Files of interest:**
- `crates/goose/src/agents/agent.rs` — Agent struct, reply(), reply_internal()
- `crates/goose/src/execution/manager.rs` — AgentManager singleton, session LRU cache
- `crates/goose/src/agents/retry.rs` — RetryManager
- `crates/goose/src/context_mgmt/` — Compaction logic
- `crates/goose/src/agents/moim.rs` — MOIM (Top of Mind) injection

---

### 2. Tools / Tool Execution

**Status:** WORKING

**Lives at:** `crates/goose/src/agents/tool_execution.rs`, `crates/goose/src/agents/platform_extensions/`, `crates/goose/src/agents/extension_manager.rs`

**What works:**
- Tool dispatch: when the model emits a `ToolRequest`, the agent routes it through `ToolConfirmationRouter` for permission checks, then executes via the extension that registered the tool
- 12 platform extensions (builtin tools):
  - **developer** — file read/write, shell execution
  - **todo** — task list management
  - **analyze** — tree-sitter code analysis
  - **apps** — HTML/CSS/JS sandbox app creation
  - **skills** — filesystem skill discovery and injection
  - **summon** — subagent delegation and knowledge loading
  - **orchestrator** — multi-session management (disabled by default)
  - **chatrecall** — past conversation search (disabled by default)
  - **summarize** — LLM summarization (disabled by default)
  - **code_execution** — code mode for token-saving execution (disabled by default)
  - **tom** — Top of Mind context injection
  - **ext_manager** — extension discovery/enable/disable
- Tool results flow back as `CallToolResult` and are appended to the conversation
- `RepetitionInspector` detects repeated identical tool calls and blocks infinite loops
- `SecurityInspector`, `AdversaryInspector`, `EgressInspector` inspect tool calls for security concerns
- `ToolInspectionManager` coordinates all inspectors in a pipeline

**What's stubbed or dormant:**
- 4 extensions disabled by default but fully implemented (orchestrator, chatrecall, summarize, code_execution)

**Honest assessment:**
The tool system is mature and well-architected. The MCP-compatible interface means external and internal tools use the same dispatch path. The security inspection pipeline is genuinely production-grade. All 12 platform extensions are fully implemented.

**Files of interest:**
- `crates/goose/src/agents/tool_execution.rs` — ToolCallContext, dispatch logic
- `crates/goose/src/agents/platform_extensions/` — All 12 builtin extensions
- `crates/goose/src/agents/extension_manager.rs` — Extension lifecycle management
- `crates/goose/src/tool_monitor.rs` — RepetitionInspector
- `crates/goose/src/security/` — Security inspectors

---

### 3. Skills System

**Status:** WORKING (unexercised)

**Lives at:** `crates/goose/src/skills.rs` (CRUD), `crates/goose-server/src/routes/skills.rs` (API), `crates/goose/src/agents/platform_extensions/skills.rs` (filesystem skills), `crates/goose/src/tasks/mod.rs` (detection)

**What works:**
- Full database schema: `skills`, `skill_executions`, `skill_triggers`, `skill_dismissals` tables
- CRUD API at `/permagent/skills` — create, list, get detail, delete, dismiss
- **TaskLogger IS wired** into tool dispatch (`agent.rs:575-640`): every tool call is logged as a task with `log_task_created`, `log_task_started`, `log_task_completed`
- **Auto-detection IS wired**: `check_repetition_candidates()` runs after every task completion, queries the `repetition_candidates` view, and emits `SkillProposed` events when patterns are detected
- `argument_shape_hash` computation: SHA-256 hash of sorted top-level parameter keys
- Config: `skills.auto_detect: true`, `skills.repetition_threshold: 2`, `skills.repetition_window_days: 7`
- The **skills platform extension** discovers `.md` skill files from filesystem directories
- Builtin skills exist in `crates/goose/src/agents/builtin_skills/skills/`
- Skill dismissal: 30-day window to suppress re-proposals

**Why it appears empty:**
- The `tasks` table has 0 rows because all 25 existing sessions are simple chat exchanges ("hello", "say hi") that never triggered tool calls
- The pipeline is complete: tool call → task log → shape hash → repetition check → skill proposal → CRUD
- It will activate the moment a session makes tool calls (e.g., developer tools, file operations)

**What's stubbed:**
- `skill_executions` table exists but no code writes execution records when a skill is used
- Skills are loaded as passive context (instructions injected into system prompt), not actively triggered

**Honest assessment:**
This was initially assessed as "schema only" but is actually fully wired end-to-end. The entire pipeline from tool invocation → task logging → repetition detection → skill proposal is in place. It just hasn't been exercised because no tool-using sessions have run. A single session with file edits or shell commands would populate tasks and potentially trigger skill proposals.

**Files of interest:**
- `crates/goose/src/agents/agent.rs` (lines 575-640) — TaskLogger integration in tool dispatch
- `crates/goose/src/tasks/mod.rs` — TaskLogger, check_repetition_candidates, SkillsConfig
- `crates/goose/src/skills.rs` — CRUD operations
- `crates/goose-server/src/routes/skills.rs` — API routes
- `ui/command-center/src/components/skills/` — SkillsPanel, SkillCard, SkillEditor

---

### 4. Schedules / Cron / Background Work

**Status:** WORKING

**Lives at:** `crates/goose/src/scheduler.rs`, `crates/goose-server/src/routes/schedule.rs`

**What works:**
- Full scheduler implementation using `tokio-cron-scheduler` for cron-based job execution
- Jobs defined as recipe + cron expression, persisted to `~/.permagent/schedule.json`
- When a cron fires, creates a new session (type: `scheduled`), instantiates an Agent, runs the recipe prompt
- Worker persona support: scheduled jobs can specify a `worker_persona` key
- API: create, list, delete, update cron, run_now, pause, unpause, kill, inspect, list sessions per schedule
- Evidence of actual use: 5 scheduled sessions in DB with recipes like "Persona Test", "Worker Test"
- The agent can create/manage schedules through the `schedule_tool` platform tool
- Concurrent job support with per-job cancellation tokens

**What's missing:**
- Event-driven triggers (only cron/time-based)
- Schedule history/audit log beyond session records

**Honest assessment:**
The scheduler is genuinely production-ready with proper lifecycle management, persistence, cancellation, persona injection, and full API coverage. Confirmed working with real scheduled sessions in the database.

**Files of interest:**
- `crates/goose/src/scheduler.rs` — Scheduler struct, cron job execution
- `crates/goose-server/src/routes/schedule.rs` — Full REST API (11 endpoints)
- `crates/goose/src/agents/schedule_tool.rs` — Agent-facing schedule management
- `ui/command-center/src/components/automate/AutomateView.tsx` — Schedule UI

---

### 5. Extensions / MCP Servers

**Status:** WORKING

**Lives at:** `crates/goose/src/agents/mcp_client.rs`, `crates/goose/src/agents/extension_manager.rs`

**What works:**
- Full MCP client implementation via `rmcp` crate — list_tools, call_tool, list_resources, read_resource, list_prompts, get_prompt
- Extension types: `platform` (builtin), `sse`, `stdio` (subprocess), `streamable_http`
- 12 platform extensions currently configured in `config.yaml`
- Extension Manager allows the agent to discover, enable, and disable extensions at runtime
- Env variable injection with secret resolution from keyring
- Malware check for external extensions
- Hot-reload: detects config changes and restarts affected extensions (including secret rotation detection)
- OAuth flow support for auth-required MCP servers

**What's stubbed:**
- No external MCP servers currently configured (all are platform extensions)
- MCP resource/prompt reading is implemented but not integrated into the agent loop's automatic context loading

**Honest assessment:**
The MCP subsystem is one of the most complete. It implements the full MCP protocol, handles multiple transport types, and manages extension lifecycles including security checks. The external MCP path hasn't been heavily exercised but the code appears correct.

**Files of interest:**
- `crates/goose/src/agents/mcp_client.rs` — MCP client implementation
- `crates/goose/src/agents/extension_manager.rs` — Extension lifecycle
- `crates/goose/src/agents/extension.rs` — ExtensionConfig types
- `crates/goose/src/agents/validate_extensions.rs` — Validation
- `crates/goose/src/agents/extension_malware_check.rs` — Security

---

### 6. Sessions, Messages, Tool Calls

**Status:** WORKING

**Lives at:** `crates/goose/src/session/session_manager.rs`, `crates/goose-server/src/routes/session.rs`

**What works:**
- Full session lifecycle: create (date-based ID), add messages, update tokens, fork
- 7 session types: `User`, `Scheduled`, `SubAgent`, `Hidden`, `Terminal`, `Gateway`, `Acp`
- Message storage: role, content_json (text, tool requests/results, thinking blocks, action required), metadata, tokens
- Token tracking: per-message and accumulated session totals
- Chat history search via FTS on message content
- Auto-generated session names via LLM
- Thread system: `threads` and `thread_messages` tables exist for conversation threading
- SSE event streaming with sequence numbering and replay buffer (max 512 events)
- DB evidence: 25 sessions, 93 messages — actively populated and used

**What's stubbed:**
- `threads` table has 0 rows — schema exists but not used
- No explicit session state field — completion/pausing is implicit

**Honest assessment:**
Sessions and messages are the workhorse. They work correctly, persist reliably, and handle the full range of content types. The Spectral database is well-structured with proper indexes and FTS.

**Files of interest:**
- `crates/goose/src/session/session_manager.rs` — SessionManager, all DB operations
- `crates/goose/src/session/spectral_schema.rs` — Database schema/migrations
- `crates/goose/src/session/chat_history_search.rs` — FTS search
- `crates/goose-server/src/routes/session.rs` — Session API
- `crates/goose-server/src/routes/session_events.rs` — SSE streaming

---

### 7. Tasks (Tool Invocation Logging)

**Status:** WORKING (unexercised)

**Lives at:** `crates/goose/src/tasks/mod.rs`, `crates/goose/src/agents/agent.rs` (lines 575-640)

**What works:**
- `TaskLogger` struct with full lifecycle: `log_task_created`, `log_task_started`, `log_task_completed`, `log_task_failed`
- **IS wired into tool dispatch** at `agent.rs:575-640` — every tool call goes through:
  1. `log_task_created()` before execution
  2. `log_task_started()` with session_id
  3. `log_task_completed()` after execution with duration
  4. `check_repetition_candidates()` for auto-skills detection
- `argument_shape_hash`: SHA-256 of sorted top-level parameter keys for pattern matching
- `SkillsConfig` loaded from config for threshold/window settings
- `repetition_candidates` view: aggregates tool+shape_hash pairs occurring 2+ times in 7 days
- Events emitted on all lifecycle transitions
- Initialized at server startup (`goose-server/src/state.rs:49-54`)

**Why 0 rows:**
- All 25 existing sessions are simple chat exchanges (no tool calls)
- The wiring is correct; it just hasn't been triggered

**Honest assessment:**
Initially appeared to be "schema only" but is actually fully integrated. The code at `agent.rs:575-640` is clear and correct — it wraps every `dispatch_tool_call()` with TaskLogger calls and runs repetition detection afterward. The moment a user session makes tool calls (developer tools, file ops, shell commands), tasks will be logged and the auto-skills pipeline will activate.

**Files of interest:**
- `crates/goose/src/tasks/mod.rs` — TaskLogger implementation
- `crates/goose/src/agents/agent.rs` (lines 575-640) — Integration point
- `crates/goose-server/src/state.rs` (lines 49-54) — Initialization

---

### 8. Knowledge Graph

**Status:** SCHEMA ONLY (Spectral DB tables); WORKING (Spectral Brain)

**Lives at:** Schema in `crates/goose/src/session/spectral_schema.rs`; Working brain at `~/.permagent/brain/`

**Two distinct systems exist:**

**Spectral DB (`~/.permagent/spectral/permagent.db`):**
- `knowledge_graph` table: subject-predicate-object triples with confidence, validity windows — **0 rows, nothing writes to it**
- `memories` table: key/content pairs with wing/hall/room taxonomy, embeddings, confidence — **0 rows, nothing writes to it**
- FTS5 indexes on both, views for current state
- These appear to be a designed-but-unconnected schema

**Spectral Brain (`~/.permagent/brain/memory.db`):**
- **997 memories** with key, content, category, wing, signal_score, visibility, confidence
- **138,825 constellation fingerprints** — vector/semantic index for similarity search
- Active FTS triggers maintain search indexes
- Brain search API (`/api/brain/search`) confirmed working — returns results from both chat FTS and Spectral Brain semantic recall

**What's missing:**
- Connection between the two systems — the Spectral DB tables duplicate concepts that the Spectral Brain already implements
- Entity/relationship extraction from conversations to populate the knowledge_graph triples
- Memory consolidation / decay logic

**Honest assessment:**
There are two memory systems that don't talk to each other. The Spectral Brain at `~/.permagent/brain/` is actually working with nearly 1,000 memories and 138K fingerprints. The Spectral DB's `memories` and `knowledge_graph` tables are an unused parallel schema. The brain search correctly queries the working system. The question is whether to wire the unused schema to the Brain or deprecate it.

**Files of interest:**
- `~/.permagent/brain/memory.db` — Working Spectral Brain (997 memories)
- `~/.permagent/brain/graph.kz` — Knowledge graph (7.7MB)
- `crates/goose/src/session/spectral_schema.rs` — Unused duplicate schema
- `crates/goose-server/src/routes/brain.rs` — Brain search API (queries both systems)
- `ui/command-center/src/components/brain/` — BrainPanel, BrainView, useBrainData

---

### 9. Provider Routing / Model Selection

**Status:** WORKING

**Lives at:** `crates/goose/src/providers/`

**What works:**
- 44+ provider implementations: Anthropic, OpenAI, Azure, Bedrock, Google/Gemini, Ollama, OpenRouter, LiteLLM, Moonshot/Kimicode, Databricks, Snowflake, GitHub Copilot, ChatGPT Codex, Claude Code, Cursor Agent, Venice, xAI, NanoGPT, Avian, Tetrate, Pi, SageMaker, GCP Vertex AI, plus declarative providers and ACP variants
- Per-session provider selection: each session records `provider_name` and `model_config`
- Confirmed in DB: sessions use both `anthropic` and `moonshot` providers
- Provider registry (`provider_registry.rs`) for discovering available providers
- Retry logic with exponential backoff (initial 1s, 2x multiplier, 30s cap), transient-error-aware
- Auth-aware retry: single credential refresh on auth failure before giving up
- Toolshim for providers lacking native tool support
- Usage estimator for token counting with cost fields (`input_token_cost`, `output_token_cost`)

**What's stubbed:**
- `provider_inventory_entries` table exists but 0 rows — model discovery/caching not exercised
- Cost data available in `ModelInfo` but not used for routing decisions

**What's missing:**
- Cross-provider fallback (if one provider fails, try another)
- Cost-based routing (cheap model for simple tasks)
- Per-task model routing (different model per task type)

**Honest assessment:**
The provider system is impressively broad with 44+ integrations. Per-session provider selection works with real usage data confirming multi-provider operation. The retry logic is production-grade. What's missing is the intelligence layer: automatic fallback, cost optimization, and task-appropriate routing.

**Files of interest:**
- `crates/goose/src/providers/mod.rs` — All provider modules
- `crates/goose/src/providers/init.rs` — Provider creation factory
- `crates/goose/src/providers/retry.rs` — Retry logic
- `crates/goose/src/providers/toolshim.rs` — Tool shim
- `crates/goose/src/providers/usage_estimator.rs` — Token/cost tracking

---

### 10. Credential Storage / Vault

**Status:** WORKING

**Lives at:** `crates/goose/src/config/base.rs`, `crates/goose/src/oauth/persist.rs`

**What works:**
- System keyring integration via `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- Secrets stored as single JSON object in keyring under service "permagent"
- Fallback: `~/.permagent/secrets.yaml` with mode 0o600 if keyring disabled (`PERMAGENT_DISABLE_KEYRING`)
- `get_secret()` checks env vars first, then keyring
- Extension env_keys: MCP extensions reference secrets by key, resolved from keyring at extension start
- OAuth token persistence for provider auth flows (`GooseCredentialStore`)
- Per-MCP credential scoping via `oauth_creds_{name}` pattern
- Auth-error-triggered credential refresh (single retry)

**What's missing:**
- No automatic credential rotation or TTL-based refresh
- No encrypted-at-rest for database content (SQLite is plaintext)
- No per-session credential scoping

**Honest assessment:**
Functional and reasonably secure for a single-user desktop agent. OS keyring is the right default. The file fallback with restrictive permissions is pragmatic. For multi-user or cloud deployment, it would need hardening.

**Files of interest:**
- `crates/goose/src/config/base.rs` — Config struct with keyring operations
- `crates/goose/src/oauth/persist.rs` — OAuth token persistence

---

### 11. Subagent Dispatch

**Status:** WORKING

**Lives at:** `crates/goose/src/agents/subagent_handler.rs`, `crates/goose/src/agents/platform_extensions/summon.rs`

**What works:**
- `run_subagent_task()` creates a fresh Agent instance with independent conversation context
- **summon** extension provides `delegate` tool: configurable provider, model, temperature, max_turns, extensions
- Async mode: `delegate(async: true)` runs tasks in background with progress tracking
- `BackgroundTask` tracking with JoinHandles, turn counts, activity timestamps
- Max concurrency: `GOOSE_MAX_BACKGROUND_TASKS` (default 5)
- Worker persona support: subagents assume personas from `agent.yaml`
- Task result loading: `load(source: "task_id")` waits for completion, `cancel: true` aborts
- Completed task caching and automatic cleanup
- **orchestrator** extension (disabled by default): session-level management of other agents
- Session type `SubAgent` for tracking

**What's missing:**
- No automatic task decomposition (model must explicitly call delegate)
- No subagent-to-subagent communication
- No shared context between parent and child agents

**Honest assessment:**
Subagent dispatch is genuinely functional with async execution, background tracking, concurrency limits, and persona injection. Combined with the scheduler, this enables multi-agent workflows. The main gap is that delegation is explicit — the model must decide to delegate.

**Files of interest:**
- `crates/goose/src/agents/subagent_handler.rs` — Core subagent execution
- `crates/goose/src/agents/platform_extensions/summon.rs` — Summon extension
- `crates/goose/src/agents/platform_extensions/orchestrator.rs` — Orchestrator extension
- `crates/goose/src/agents/subagent_task_config.rs` — TaskConfig

---

### 12. Continuous Awareness / Probe

**Status:** PARTIALLY WIRED

**Lives at:** `crates/goose/src/hints/`, `crates/goose/src/scheduler.rs`

**What works:**
- **Hint loading** (`hints/load_hints.rs`): `SubdirectoryHintTracker` observes tool arguments (file paths from shell commands), proactively loads `.goosehints` and `AGENTS.md` from accessed subdirectories, extends context as agent navigates filesystem
- **Scheduled execution** can serve as periodic awareness (cron-based agent runs with recipes)
- **MOIM** (Top of Mind) injects static context via env vars into every turn

**What's stubbed:**
- `SessionType::Hidden` exists but not used for background probes
- Hint loading is reactive (triggered by tool calls), not continuously proactive

**What's missing:**
- Background reasoning loop between user interactions
- Proactive memory surfacing ("I noticed something relevant")
- Event-driven agent activation (file changes, incoming messages)
- Context-aware nudges
- Ambient monitoring / watch patterns

**Honest assessment:**
Not fully MISSING as initially assessed. The hint system provides per-turn context enrichment based on filesystem access patterns, and the scheduler could theoretically run periodic "awareness" recipes. But there's no mechanism for the agent to decide on its own that it should act, surface information, or reflect between interactions. The gap between "chat agent" and "persistent agent" remains.

**Files of interest:**
- `crates/goose/src/hints/load_hints.rs` — Subdirectory hint discovery
- `crates/goose/src/agents/moim.rs` — MOIM static context injection
- `crates/goose/src/scheduler.rs` — Could be extended for awareness jobs

---

### 13. Permissions / Authorization

**Status:** WORKING

**Lives at:** `crates/goose/src/permission/`, `crates/goose/src/config/permission.rs`

**What works:**
- `PermissionManager` manages tool-level authorization policies from `~/.permagent/permission.yaml`
- Three levels: `AlwaysAllow`, `AskBefore`, `NeverAllow`
- `PermissionJudge` uses LLM to classify tool calls as read-only vs. write when policy requires approval
- `ToolPermissionStore` persists per-tool decisions with optional expiry (blake3 argument hashing)
- `ToolConfirmationRouter` routes through the permission pipeline before execution
- `GooseMode` provides high-level control: `auto` vs. `chat`
- Tool annotations: `read_only`, `destructive`, `idempotent`, `open_world`
- Single-user model with `DEFAULT_USER_ID = "default"` — appropriate for desktop

**What's missing:**
- No runtime API to update permissions (config file only)
- No multi-user support — hardcoded single user
- No audit log of permission decisions (stored but not queried)
- No per-extension approval (only per-tool)

**Honest assessment:**
Well-designed for a single-user desktop agent. The LLM-based read-only judge is clever. For multi-user deployment, this would need significant extension.

**Files of interest:**
- `crates/goose/src/permission/permission_judge.rs` — LLM-based classification
- `crates/goose/src/permission/permission_store.rs` — Persistent decisions
- `crates/goose/src/agents/tool_confirmation_router.rs` — Routing pipeline
- `crates/goose/src/config/permission.rs` — PermissionManager

---

### 14. Recipes / Playbooks

**Status:** WORKING

**Lives at:** `crates/goose/src/recipe/`, `crates/goose-server/src/routes/recipe.rs`

**What works:**
- `Recipe` struct: version, title, description, prompt, settings (provider, model, temperature), extensions, parameters, sub_recipes
- Recipe loading from YAML/JSON files with template rendering
- Discovery from multiple paths: current dir, env var, `~/.permagent/recipes/`, `.goose/recipes/`, `.agents/recipes/`
- Recipe deeplinks for sharing via URLs
- Scheduled recipes: attached to cron jobs, stored in `~/.permagent/scheduled_recipes/`
- Full API: create, encode, decode, list, schedule, save, delete, parse, recipe_to_yaml
- Sessions record their recipe (confirmed in DB)
- Slash commands can reference recipes

**What's missing:**
- Recipe composition (calling one recipe from another beyond sub_recipes)
- Dry-run / validation before execution

**Honest assessment:**
Recipes are the backbone of automated workflows. Combined with the scheduler and subagent dispatch, they enable sophisticated multi-step agent operations. This is solid Goose infrastructure.

**Files of interest:**
- `crates/goose/src/recipe/mod.rs` — Recipe struct and types
- `crates/goose/src/recipe/build_recipe/` — Recipe construction
- `crates/goose-server/src/routes/recipe.rs` — API routes (9 endpoints)

---

### 15. Observability

**Status:** PARTIALLY WIRED

**Lives at:** `crates/goose/src/tracing/`, `crates/goose/src/otel/`, `crates/goose/src/logging.rs`

**What works:**
- Structured logging via `tracing` crate — file appender with JSON formatting (DEBUG level), console (INFO level)
- Log files at `~/.permagent/logs/`: daemon.log, daemon.err, plus `llm_request.{0-9}.jsonl` rotating files for full LLM request/response capture
- Auto-cleanup: removes logs older than 14 days
- Langfuse integration (`langfuse_layer.rs`) for LLM observability with batch sending (every 5 seconds)
- OpenTelemetry/OTLP export (`otel/otlp.rs`) — traces, metrics, logs with resource detection
- Rate-limited telemetry sender
- Token counting: sessions track input/output/total per message and accumulated
- PostHog analytics (feature-gated)

**What's stubbed:**
- Langfuse requires external instance configuration
- OTLP requires external collector
- PostHog behind `telemetry` feature flag

**What's missing:**
- Cost tracking (tokens → dollars)
- Latency metrics per tool call
- Agent-level trace visualization
- Observability dashboard

**Honest assessment:**
The observability infrastructure is surprisingly complete. Structured logging, LLM request journaling, Langfuse, and OTLP cover the major bases. The `llm_request.*.jsonl` files are particularly valuable for debugging. What's missing is aggregation and visualization.

**Files of interest:**
- `crates/goose/src/tracing/langfuse_layer.rs` — Langfuse integration
- `crates/goose/src/otel/otlp.rs` — OTLP export
- `crates/goose/src/logging.rs` — Log configuration
- `~/.permagent/logs/` — Log output

---

### 16. Persistence Beyond Sessions

**Status:** WORKING

**Lives at:** `crates/goose/src/config/agent_identity.rs`, `~/.permagent/agent.yaml`, `~/.permagent/brain/`

**What works:**
- **Agent persona**: `PrimaryPersona` (first_name, last_name, nickname, traits, tone, opening_greeting, voice_id) stored in `~/.permagent/agent.yaml`
- Persona injected into system prompt: "You are Henry. You are a Permagent..."
- Confirmed: agent is "Henry" with traits ["precise", "direct", "concise"]
- **Worker personas**: HashMap of named workers (e.g., "archivist" for overnight memory consolidation)
- API: GET/PUT `/api/agent/identity`, CRUD `/api/agent/workers/{key}`
- **Workspaces**: 5 configured (Home, Automate, World, Build, Brain) with layout persistence
- **Spectral Brain**: `~/.permagent/brain/memory.db` — **997 memories**, **138,825 constellation fingerprints**
  - Categories: core memories with wing assignments (permagent, jesse)
  - Sample: `henry_sells_business_model`, `workspace_context`, `henry_sells_product_strategy`
  - Brain keys: `brain.id`, `brain.key`, `brain.pub` for identity/encryption
  - Knowledge graph: `graph.kz` (7.7MB) — separate from unused Spectral DB knowledge_graph table
  - Ontology: `ontology.toml` defining the memory taxonomy
- **Config**: `config.yaml` stores provider, model, extension, and skills settings
- **Config backup**: `config.yaml.bak` with restore-from-backup on corruption

**What's stubbed:**
- Spectral DB `memories` table (0 rows) — duplicate/unused schema alongside working Brain
- No explicit export/import API for persona profiles
- No encryption at rest for database content

**Honest assessment:**
The persistence story is richer than initially apparent. The Spectral Brain at `~/.permagent/brain/` is actually working with nearly 1,000 memories and semantic fingerprints. The persona system with worker personas for scheduled jobs adds genuine personality. The brain search API correctly queries the working system. The open question is why the Spectral DB has a parallel, empty `memories` schema.

**Files of interest:**
- `~/.permagent/agent.yaml` — Persona configuration
- `~/.permagent/brain/memory.db` — Working Spectral Brain (997 memories)
- `~/.permagent/brain/graph.kz` — Knowledge graph data
- `~/.permagent/brain/ontology.toml` — Memory taxonomy
- `crates/goose/src/config/agent_identity.rs` — PrimaryPersona, WorkerPersona
- `crates/goose-server/src/routes/identity.rs` — Identity API
- `crates/goose-server/src/routes/workers.rs` — Workers API

---

## Substrate Roadmap Implications

Based on the corrected audit findings:

### Tier 1: Quick Wins (days)
1. **Run a tool-using session** — Simply having a session that triggers developer tools (file edits, shell commands) will exercise the TaskLogger, populate the tasks table, and potentially trigger the first auto-skill proposal. This is a test, not a code change.
2. **Reconcile the dual memory systems** — Understand the relationship between Spectral Brain (`~/.permagent/brain/`) and Spectral DB (`~/.permagent/spectral/`) memory tables. Either wire the DB tables to the Brain or deprecate them.
3. **Enable orchestrator extension by default** — It's fully implemented but disabled. Enabling it gives the agent multi-session self-management.

### Tier 2: Intelligence Layer (weeks)
4. **Wire knowledge_graph population** — The Spectral Brain has `graph.kz` (7.7MB) but the Spectral DB `knowledge_graph` table has 0 rows. Connect entity extraction to whichever system should own structured knowledge.
5. **Activate skill execution recording** — Write `skill_executions` rows when skills are used, enabling skill performance tracking.
6. **Cost-aware provider routing** — Use the existing `ModelInfo` cost fields to route simple tasks to cheap models.

### Tier 3: Persistent Intelligence (months)
7. **Continuous awareness / probe system** — Build the "agent acts between user turns" mechanism. Start with scheduled "reflection" recipes.
8. **Event-driven triggers** — Extend scheduler beyond cron to support file-change, webhook, and message-based triggers.
9. **Cross-provider fallback** — Automatic failover when a provider errors.

---

## Concerns

1. **Dual memory systems**: The Spectral Brain (`~/.permagent/brain/memory.db`, 997 memories) and Spectral DB (`~/.permagent/spectral/permagent.db`, `memories` table, 0 rows) represent two parallel approaches to the same problem. The Brain is working; the DB tables are empty. This should be reconciled before building on either.

2. **System prompt claims vs. Spectral DB reality**: The persona prompt says "continuity across sessions through Spectral memory" — this IS true via the Brain, but the empty DB tables create confusion about which system is the source of truth.

3. **Plaintext database**: Both SQLite databases store content in plaintext. No encryption at rest. The Brain has `brain.key` and `brain.pub` files but it's unclear if they provide encryption or just identity.

4. **Single-user hardcoded**: `DEFAULT_USER_ID = "default"` throughout. Multi-user would require significant refactoring.

---

## Open Questions

1. **What populates the Spectral Brain?** The Brain at `~/.permagent/brain/` has 997 memories. What code writes to `memory.db`? Is it a separate process, or integrated into the agent session flow?

2. **Why do duplicate memory schemas exist?** The Spectral DB `memories` table and the Spectral Brain `memories` table have similar but not identical schemas. Was one intended to replace the other?

3. **Brain keys purpose**: `~/.permagent/brain/brain.key` and `brain.pub` — are these for encryption, identity verification, or something else?

4. **Thread system purpose**: The `threads` and `thread_messages` tables exist with 0 rows. Intended for conversation branching?

5. **Provider inventory**: `provider_inventory_entries` and `provider_inventory_models` tables exist but are empty. Is there a provider discovery mechanism that should populate these?

6. **Skill execution tracking**: The `skill_executions` table exists but nothing writes to it. Skills are injected as context, not tracked as executions. Is execution tracking planned?
