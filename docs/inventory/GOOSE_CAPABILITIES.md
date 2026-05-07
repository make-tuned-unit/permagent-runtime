# Goose Capability Inventory

## Summary
- **Total capabilities identified:** 78
- **In use:** 24
- **Available but unused:** 38
- **Internal only:** 10
- **Already replaced:** 6
- **Inventory date:** 2026-05-07
- **Codebase scope:** crates/goose/, crates/goose-server/, crates/goose-cli/, ui/goose2/src-tauri/

---

## Runtime Core

### 1. Agent Execution Loop
- **Location:** `crates/goose/src/agents/agent.rs` (lines 138-400+)
- **Description:** The main `Agent` struct implements the agentic loop for autonomous execution. Manages tool requests, permission checks, conversation state, and retry logic. Supports interactive, background, and subtask execution modes.
- **Public surface:** `Agent::new()`, `Agent::with_config(AgentRunnerConfig)`, `SessionExecutionMode` enum, `AgentEvent` enum
- **Permagent usage:** In use — core of daemon reply handling in `crates/goose-server/src/routes/reply.rs` and `session_events.rs`
- **Notes:** Central orchestrator. Every chat turn flows through this.

### 2. Prompt Management & Templating
- **Location:** `crates/goose/src/agents/prompt_manager.rs`, `prompt_template.rs`
- **Description:** Manages system prompts with Jinja2-style templating. Supports user customization via config directory. Maintains templates for system.md, compaction.md, subagent_system.md, recipe.md.
- **Public surface:** `PromptManager`, `render_template(name, context)`, user prompt customization
- **Permagent usage:** In use — system prompt generation, compaction
- **Notes:** User-facing customization via `~/.config/permagent/prompts/` — not exposed in Permagent UX yet.

### 3. Tool Execution Pipeline
- **Location:** `crates/goose/src/agents/tool_execution.rs`, `tool_confirmation_router.rs`, `platform_tools.rs`
- **Description:** Executes tool calls from agent responses, routes through confirmation if needed, handles results and streaming. Integrates with the tool inspection pipeline.
- **Public surface:** `ToolCallContext`, tool execution with confirmation routing, platform tools
- **Permagent usage:** In use — every tool call flows through this
- **Notes:** Tightly integrated with security and permission systems.

### 4. Subagent & Container Execution
- **Location:** `crates/goose/src/agents/subagent_handler.rs`, `subagent_execution_tool.rs`, `container.rs`
- **Description:** Spawns child agents to handle delegated tasks. Isolates execution with own sessions, tools, and conversation state. Supports recursive task decomposition.
- **Public surface:** `SubagentHandler`, `Container`, task delegation via `SUBAGENT_TOOL_REQUEST_TYPE`
- **Permagent usage:** In use — via summon/delegate extension
- **Notes:** Powerful for multi-agent workflows. Not directly exposed in Permagent UX.

### 5. Tool Inspection Pipeline
- **Location:** `crates/goose/src/tool_inspection.rs`
- **Description:** Coordinated multi-inspector system. Runs: SecurityInspector, EgressInspector, AdversaryInspector, PermissionInspector, RepetitionInspector. Aggregates results.
- **Public surface:** `ToolInspectionManager`, `ToolInspector` trait, `InspectionResult`
- **Permagent usage:** In use — gates all tool execution
- **Notes:** Smart Approve mode driven by `apply_tool_annotations()`.

### 6. Repetition Detection
- **Location:** `crates/goose/src/tool_monitor.rs`
- **Description:** Detects when tools are called repeatedly with same parameters, indicating loops or stuck states. Configurable max repetitions.
- **Public surface:** `RepetitionInspector`, configurable thresholds
- **Permagent usage:** In use — part of tool inspection pipeline
- **Notes:** Prevents runaway agent loops.

### 7. Reply Parts & Streaming
- **Location:** `crates/goose/src/agents/reply_parts.rs`
- **Description:** Handles streaming reply assembly, partial message construction, and tool call result formatting for SSE delivery to frontends.
- **Public surface:** Reply part types, streaming assembly
- **Permagent usage:** In use — chat streaming
- **Notes:** Internal plumbing for SSE event generation.

### 8. Slash Commands
- **Location:** `crates/goose/src/slash_commands.rs`
- **Description:** Command-style interface for special agent operations (e.g., `/ask`, `/schedule`). Parsed from user input.
- **Public surface:** Slash command parsing and routing
- **Permagent usage:** Available but unused — not exposed in desktop UX
- **Notes:** CLI uses these heavily. Could enable power-user features in chat.

---

## Session Management

### 9. Session Persistence (SQLite)
- **Location:** `crates/goose/src/session/session_manager.rs`, `session/mod.rs`
- **Description:** Creates, stores, and retrieves chat sessions with full conversation history. SQLite-backed with metadata (created_at, updated_at, session_type). Supports search and recovery.
- **Public surface:** `SessionManager` singleton, `Session` struct, `SessionType` enum
- **Permagent usage:** In use — daemon session routes
- **Notes:** Core infrastructure. All sessions persisted here.

### 10. Thread Management (Conversation Branching)
- **Location:** `crates/goose/src/session/thread_manager.rs`
- **Description:** Manages branching conversation threads within a session. Allows exploring alternative conversation paths.
- **Public surface:** `ThreadManager`, `Thread`, `ThreadMetadata`
- **Permagent usage:** Available but unused — not exposed in UX
- **Notes:** Powerful for "what if" exploration. No UI surface exists.

### 11. Session Forking
- **Location:** `crates/goose-server/src/routes/session.rs` (`POST /api/sessions/{id}/fork`)
- **Description:** Clone a session optionally truncating to a point in time. Supports both copy and truncate modes.
- **Public surface:** `POST /api/sessions/{id}/fork` with `timestamp`, `truncate`, `copy` params
- **Permagent usage:** Available but unused — endpoint exists, no frontend calls it
- **Notes:** Enables branching workflows and session recovery.

### 12. Session Export/Import
- **Location:** `crates/goose-server/src/routes/session.rs`, `crates/goose-cli/src/commands/session.rs`
- **Description:** Export sessions in markdown/JSON/YAML formats. Import from exported JSON. CLI supports diagnostics bundle export.
- **Public surface:** `POST /api/sessions/import`, CLI `session export` command
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Useful for debugging and session sharing.

### 13. Extension State Persistence
- **Location:** `crates/goose/src/session/extension_data.rs`
- **Description:** Stores per-session extension state, enabled extensions list, and todo items in session metadata.
- **Public surface:** `EnabledExtensionsState`, `ExtensionState`, `TodoState`
- **Permagent usage:** In use — session initialization
- **Notes:** Internal infrastructure.

### 14. Workspace Management
- **Location:** `crates/goose/src/workspaces.rs`
- **Description:** CRUD for Command Center workspaces. Preset seeding (Home, Automate, World, Build, Brain). Layout JSON persistence.
- **Public surface:** `Workspace` struct, `seed_presets_if_empty()`
- **Permagent usage:** In use — workspace switching in Command Center
- **Notes:** Permagent-specific addition, not original goose.

---

## Provider Abstraction

### 15. Provider Registry & Factory
- **Location:** `crates/goose/src/providers/mod.rs`, `init.rs`, `provider_registry.rs`, `catalog.rs`
- **Description:** Central registry of 40+ AI model providers. Factory functions create instances with configuration. Manages discovery, instantiation, and lifecycle.
- **Public surface:** `create()`, `create_with_default_model()`, `create_with_named_model()`, `get_from_registry()`
- **Permagent usage:** In use — agent provider setup
- **Notes:** Massive provider surface. Permagent likely only uses Anthropic + maybe OpenAI.

### 16. Provider Base Interface & Streaming
- **Location:** `crates/goose/src/providers/base.rs`
- **Description:** Abstract `Provider` trait all providers implement. Handles message streaming, token counting, usage tracking, context limits. Supports streaming and non-streaming modes.
- **Public surface:** `Provider` trait, `ProviderUsage`, `MessageStream`
- **Permagent usage:** In use — core interface
- **Notes:** Foundational trait.

### 17. Anthropic Provider
- **Location:** `crates/goose/src/providers/anthropic.rs`
- **Description:** Claude-specific implementation with extended thinking, streaming, tool use. Primary provider for Permagent.
- **Public surface:** Anthropic provider struct
- **Permagent usage:** In use — primary provider
- **Notes:** Most important provider for Permagent.

### 18. OpenAI Provider
- **Location:** `crates/goose/src/providers/openai.rs`
- **Description:** OpenAI API implementation with GPT-4, streaming, function calling.
- **Public surface:** OpenAI provider struct
- **Permagent usage:** Available but unused — registered but Permagent defaults to Anthropic
- **Notes:** Available for user configuration.

### 19. 30+ Additional Providers
- **Location:** `crates/goose/src/providers/` (azure.rs, ollama.rs, bedrock.rs, databricks.rs, gemini_cli.rs, etc.)
- **Description:** Implementations for Azure, Ollama, Bedrock, Databricks, Google, Codex, Cursor, Copilot, and more.
- **Public surface:** Provider structs for each service
- **Permagent usage:** Available but unused — most are dormant
- **Notes:** Significant code surface. Consider which providers Permagent actually needs.

### 20. Custom Provider Registry
- **Location:** `crates/goose-server/src/routes/config_management.rs`
- **Description:** User-defined providers (OpenAI-compatible endpoints). CRUD via HTTP routes.
- **Public surface:** `GET|POST|PUT|DELETE /config/custom-providers/{name}`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Power-user feature for custom endpoints.

### 21. Token Counting & Estimation
- **Location:** `crates/goose/src/token_counter.rs`, `providers/usage_estimator.rs`
- **Description:** Accurate token counting using Tiktoken with DashMap caching. Estimates tokens for tools and function calls.
- **Public surface:** `TokenCounter`, `count_tokens()`, `count_chat_tokens()`
- **Permagent usage:** In use — context management, compaction decisions
- **Notes:** Performance-critical with caching.

### 22. Multi-Provider OAuth
- **Location:** `crates/goose/src/providers/oauth.rs`, `config/signup_*.rs`
- **Description:** OAuth flows for OpenRouter, Nanogpt, Tetrate. Credential storage and refresh.
- **Public surface:** `oauth_flow()`, setup routes `POST /handle_openrouter`, `POST /handle_tetrate`, `POST /handle_nanogpt`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Enables provider marketplace.

### 23. Local Model Support (Feature-Gated)
- **Location:** `crates/goose/src/providers/local_inference.rs`, `providers/ollama.rs`
- **Description:** Local GGUF model management. Search, download, list, delete from HuggingFace. Ollama integration.
- **Public surface:** CLI `local-models` commands, `GET /local-models` endpoint
- **Permagent usage:** Available but unused — feature-gated behind `local-inference`
- **Notes:** Enables offline/private deployment.

---

## MCP Integration

### 24. MCP Client Infrastructure
- **Location:** `crates/goose/src/agents/mcp_client.rs` (600+ lines)
- **Description:** Core MCP client interface. `McpClientTrait` defines list_tools, call_tool, list_resources, read_resource, get_prompt. Platform-aware capabilities (Desktop has `mcpui: true`).
- **Public surface:** `McpClientTrait`, `GooseClient`, `GooseMcpClientCapabilities`
- **Permagent usage:** In use — extension tool execution
- **Notes:** Foundational for all extensions.

### 25. Extension Manager
- **Location:** `crates/goose/src/agents/extension_manager.rs` (400+ lines)
- **Description:** Creates, initializes, and manages MCP clients. In-memory caching with versioning. Resource handling, tool aggregation, process management, auth integration, malware scanning.
- **Public surface:** `ExtensionManager`, `Extension`, `ResourceItem`, `ExtensionError`
- **Permagent usage:** In use — extension lifecycle
- **Notes:** Centralized extension coordinator.

### 26. Extension Configuration
- **Location:** `crates/goose/src/config/extensions.rs`
- **Description:** Registry of platform extensions. Enable/disable state persistence. Default extension setup. Stored in `~/.config/permagent/config.yaml`.
- **Public surface:** `get_all_extensions()`, `get_enabled_extensions()`, `set_extension()`, extension CRUD routes
- **Permagent usage:** In use — extension setup
- **Notes:** goose2 has Tauri commands for this; desktop does not.

### 27. Extension Validation & Security
- **Location:** `crates/goose/src/agents/validate_extensions.rs`, `builtin_extension.rs`
- **Description:** Validates extension configs at load time. Malware scanning for subprocess-based extensions.
- **Public surface:** Extension validation pipeline
- **Permagent usage:** In use — load-time validation
- **Notes:** Security boundary for third-party extensions.

---

## Platform Extensions (In-Process MCP)

### 28. Developer Extension
- **Location:** `crates/goose/src/agents/platform_extensions/developer/mod.rs`
- **Description:** First-class file/shell access: write, edit, shell execute, directory tree. Unprefixed tools. Platform-specific instructions.
- **Public surface:** `write`, `edit`, `shell`, `tree` tools
- **Permagent usage:** In use — core agent file/shell operations
- **Notes:** Default enabled. Most-used extension.

### 29. Analyze Extension
- **Location:** `crates/goose/src/agents/platform_extensions/analyze/mod.rs`
- **Description:** Tree-sitter AST parsing for code structure analysis. Directory overview, semantic file details, symbol call graphs.
- **Public surface:** `analyze(path, focus?, max_depth?, follow_depth?)` tool
- **Permagent usage:** In use — code understanding
- **Notes:** Default enabled.

### 30. Todo Extension
- **Location:** `crates/goose/src/agents/platform_extensions/todo.rs`
- **Description:** Persistent todo list tracking across turns and compaction. Configurable max chars via `GOOSE_TODO_MAX_CHARS`.
- **Public surface:** `todo_write` tool
- **Permagent usage:** In use — agent task tracking
- **Notes:** Default enabled.

### 31. Apps Extension (MCP Apps)
- **Location:** `crates/goose/src/agents/platform_extensions/apps.rs`
- **Description:** Create and manage HTML/CSS/JS apps in sandboxed windows. Create from PRD, iterate with feedback. Stored in `~/.local/share/permagent/apps/`.
- **Public surface:** `create_app`, `iterate_app`, `delete_app`, `list_apps` tools
- **Permagent usage:** In use — dynamic UI generation
- **Notes:** Default enabled. Powerful for agent-generated UIs.

### 32. Summon Extension (Recipe/Subagent Delegation)
- **Location:** `crates/goose/src/agents/platform_extensions/summon.rs` (400+ lines)
- **Description:** Load recipes/subrecipes/agents into context. Delegate tasks to subagents with background execution. Discovers from filesystem YAML frontmatter. Supports async tasks with cancellation.
- **Public surface:** `load`, `delegate` tools
- **Permagent usage:** In use — knowledge composition, delegation
- **Notes:** Default enabled. Core of multi-agent orchestration.

### 33. Orchestrator Extension
- **Location:** `crates/goose/src/agents/platform_extensions/orchestrator.rs`
- **Description:** Manage agent sessions: list, view, start, send messages, interrupt, stop. Session filtering by type. Worker persona support.
- **Public surface:** `list_sessions`, `view_session`, `start_agent`, `send_message`, `interrupt_agent`, `stop_agent` tools
- **Permagent usage:** In use — multi-session management
- **Notes:** Default enabled. Enables agent-managed sessions.

### 34. Extension Manager Extension
- **Location:** `crates/goose/src/agents/platform_extensions/ext_manager.rs`
- **Description:** Discover, enable/disable extensions and review extension resources at runtime.
- **Public surface:** `search_available_extensions`, `manage_extensions`, `list_resources`, `read_resource` tools
- **Permagent usage:** In use — runtime extension control
- **Notes:** Default enabled.

### 35. Skills Extension (Filesystem-Based)
- **Location:** `crates/goose/src/agents/platform_extensions/skills.rs` (507 lines)
- **Description:** Discover and load skill instructions from filesystem and builtins. SKILL.md frontmatter parsing. Multiple search paths. Built-in skills compiled in.
- **Public surface:** `load_skill` tool
- **Permagent usage:** In use — knowledge composition
- **Notes:** Default enabled.

### 36. Top Of Mind (Tom) Extension
- **Location:** `crates/goose/src/agents/platform_extensions/tom.rs`
- **Description:** Inject custom context into every turn via environment variables (`GOOSE_MOIM_MESSAGE_TEXT`, `GOOSE_MOIM_MESSAGE_FILE`).
- **Public surface:** `get_moim()` — no tools, context injection only
- **Permagent usage:** In use — ambient context injection
- **Notes:** Default enabled.

### 37. Chat Recall Extension
- **Location:** `crates/goose/src/agents/platform_extensions/chatrecall.rs`
- **Description:** Search past conversations and load session summaries. Two modes: search (query + date filters) and load (session_id retrieval).
- **Public surface:** `chatrecall` tool
- **Permagent usage:** Available but unused — default disabled
- **Notes:** Powerful for cross-session memory. Not enabled by default.

### 38. Summarize Extension
- **Location:** `crates/goose/src/agents/platform_extensions/summarize.rs`
- **Description:** Load files/directories deterministically and get LLM summary. Max 100KB/file, 1MB total.
- **Public surface:** `summarize` tool
- **Permagent usage:** Available but unused — default disabled
- **Notes:** Efficient bulk analysis.

### 39. Code Execution Extension (Feature-Gated)
- **Location:** `crates/goose/src/agents/platform_extensions/code_execution.rs`
- **Description:** Tool calls via code execution for token efficiency. Tool graph DAG visualization. Feature-gated behind `code-mode`.
- **Public surface:** Code execution tools, `ExecuteWithToolGraph`
- **Permagent usage:** Available but unused — requires feature flag
- **Notes:** Experimental. Significant token savings potential.

---

## Skills & Automation

### 40. Database Skills System
- **Location:** `crates/goose/src/skills.rs` (208 lines)
- **Description:** Auto-detection pipeline learns from tool patterns. Skills stored in SQLite with triggers, argument shape hashing, 30-day dismissal window.
- **Public surface:** `create_skill()`, `list_skills()`, `get_skill()`, `delete_skill()`, `dismiss_skill()`
- **Permagent usage:** In use — skill CRUD routes called from frontend
- **Notes:** Agent-driven skill learning.

### 41. Skills HTTP API
- **Location:** `crates/goose-server/src/routes/skills.rs`
- **Description:** CRUD endpoints: create, list, get detail, update, delete, get executions, dismiss.
- **Public surface:** `POST|GET|PUT|DELETE /permagent/skills`, `POST /permagent/skills/dismiss`
- **Permagent usage:** In use — frontend calls these
- **Notes:** Active in Permagent UX.

### 42. Scheduler (Cron-Based Automation)
- **Location:** `crates/goose/src/scheduler.rs` (400+ lines), `scheduler_trait.rs`
- **Description:** Cron-based job scheduler. Persists to JSON. Manages running tasks with cancellation tokens. Worker persona support.
- **Public surface:** `Scheduler`, `ScheduledJob`, cron expression parsing, HTTP routes at `/schedule/*`
- **Permagent usage:** Available but unused — backend exists, no frontend surface
- **Notes:** High-value harvest candidate. Enables recurring agent tasks.

### 43. Recipe System (Workflow Templates)
- **Location:** `crates/goose/src/recipe/` (14 modules)
- **Description:** YAML/JSON workflow definitions with instructions, extensions, parameters, retry config, response schemas, sub-recipes. Full validation, template rendering, deeplink support.
- **Public surface:** `Recipe` struct, HTTP routes at `/recipe/*`, CLI commands
- **Permagent usage:** In use — recipes loaded during agent sessions, but recipe CRUD not exposed in UX
- **Notes:** Recipe creation/sharing UX could be powerful for Automate tab.

### 44. Recipe Deeplinks
- **Location:** `crates/goose/src/recipe/recipe_deeplink.rs`
- **Description:** Compressed URI scheme for recipe invocation with parameters. Encode/decode endpoints.
- **Public surface:** `POST /recipe/encode`, `POST /recipe/decode`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Enables sharing recipes via URL.

---

## Memory & Context

### 45. Context Window Management & Compaction
- **Location:** `crates/goose/src/context_mgmt/mod.rs`
- **Description:** Manages context window size. Triggers automatic compaction when threshold exceeded. Summarizes conversation history to save tokens.
- **Public surface:** `compact_messages()`, `check_if_compaction_needed()`, `compute_tool_call_cutoff()`
- **Permagent usage:** In use — automatic compaction during long sessions
- **Notes:** Critical for long conversations.

### 46. Conversation Validation & Repair
- **Location:** `crates/goose/src/conversation/mod.rs`
- **Description:** Validates and repairs conversation state for LLM compatibility. Fixes role alternation, removes orphaned tool calls, merges consecutive messages.
- **Public surface:** `Conversation` struct, `fix_conversation()`
- **Permagent usage:** In use — message preparation before LLM calls
- **Notes:** Internal plumbing.

### 47. Activity Context Builder
- **Location:** `crates/goose/src/activity/context_builder.rs`
- **Description:** Maintains live activity state (browser URL, terminal commands, project, active session). Produces digests for model context. Integrates with Brain for memory probing.
- **Public surface:** `ContextBuilder`, `handle_event()`, `current_digest()`, `LiveState`, `render_ambient_context()`
- **Permagent usage:** In use — Phase 3.5 awareness layer
- **Notes:** Permagent-specific. Active development.

### 48. Activity Ingestion to Brain
- **Location:** `crates/goose/src/activity/ingestion.rs`
- **Description:** Captures activity events and ingests to Spectral Brain for semantic memory. Tier-based ingestion (Always persisted, Aggregated rolled up, Ephemeral live-only).
- **Public surface:** Ingestion pipeline, `ActivityEvent` processing
- **Permagent usage:** In use — Brain memory writes
- **Notes:** Permagent-specific.

### 49. Activity Event Taxonomy
- **Location:** `crates/goose/src/events/activity.rs`
- **Description:** Defines all activity event types, source surfaces, tiers. Ring buffer for recent events. Convenience constructors.
- **Public surface:** `ActivityEvent`, `ActivityEventType` enum, `SourceSurface`, `EventTier`, `emit_activity()`, `recent_activity()`
- **Permagent usage:** In use — activity bus
- **Notes:** Permagent-specific.

### 50. Hints System (Project Context)
- **Location:** `crates/goose/src/hints/mod.rs`, `hints/load_hints.rs`
- **Description:** Loads project hints and .gitignore patterns. Provides context about which files are relevant.
- **Public surface:** `load_hint_files()`, `build_gitignore()`, `SubdirectoryHintTracker`
- **Permagent usage:** In use — file discovery context
- **Notes:** Standard goose feature.

---

## Security & Permissions

### 51. Permission Management
- **Location:** `crates/goose/src/permission/mod.rs`, `permission_inspector.rs`, `permission_judge.rs`
- **Description:** Evaluates tool requests against policies. GooseMode (Smart Approve, Manual, etc.) for user intent.
- **Public surface:** `PermissionManager`, `PermissionInspector`, `PermissionCheckResult`
- **Permagent usage:** In use — tool execution pipeline
- **Notes:** Active.

### 52. Prompt Injection Detection
- **Location:** `crates/goose/src/security/mod.rs`, `scanner.rs`, `classification_client.rs`, `patterns.rs`
- **Description:** Pattern matching and optional ML-based detection. Configurable thresholds, confidence scoring.
- **Public surface:** `SecurityManager`, `PromptInjectionScanner`, `SecurityResult`
- **Permagent usage:** In use — tool inspection
- **Notes:** Security boundary.

### 53. Adversary Inspection (LLM-Based Security)
- **Location:** `crates/goose/src/security/adversary_inspector.rs`
- **Description:** LLM-based review of suspicious tool calls. Generates human-readable threat explanations.
- **Public surface:** `AdversaryInspector` trait
- **Permagent usage:** Internal only — part of inspection pipeline
- **Notes:** Deferred analysis for complex threats.

### 54. Egress Inspector
- **Location:** `crates/goose/src/security/egress_inspector.rs`
- **Description:** Analyzes tool outputs for data exfiltration, sensitive data leaks.
- **Public surface:** `EgressInspector`
- **Permagent usage:** Internal only — part of inspection pipeline
- **Notes:** Output security.

### 55. Tool Permission Configuration
- **Location:** `crates/goose-server/src/routes/config_management.rs`
- **Description:** CRUD for per-tool permission levels (AllowAlways, AskOnce, AskAlways).
- **Public surface:** `GET|POST /config/permissions`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Enables fine-grained tool control.

---

## Observability

### 56. Global Event Bus
- **Location:** `crates/goose/src/events/mod.rs`
- **Description:** Centralized broadcast with 1000-event replay buffer. All runtime events published here. WebSocket subscribers receive live stream.
- **Public surface:** `PermagentEvent`, `emit()`, `subscribe()`, `buffered_events()`
- **Permagent usage:** In use — WebSocket `/events` endpoint, activity bus
- **Notes:** Foundation for all event-driven features.

### 57. WebSocket Event Stream
- **Location:** `crates/goose-server/src/routes/events.rs`
- **Description:** WebSocket at `/events`. Supports `resume_from` for replay from specific event. Full event history.
- **Public surface:** `GET /events` (WebSocket upgrade)
- **Permagent usage:** In use — EventLogView in Command Center
- **Notes:** Active.

### 58. Logging & Tracing
- **Location:** `crates/goose/src/logging.rs`, `tracing/mod.rs`
- **Description:** Structured logging with tracing layers. Langfuse and PostHog integration. Rate limiting.
- **Public surface:** Logging initialization, tracing layers
- **Permagent usage:** In use — observability
- **Notes:** Standard infrastructure.

### 59. Telemetry
- **Location:** `crates/goose-server/src/routes/telemetry.rs`
- **Description:** Fire-and-forget telemetry events to PostHog.
- **Public surface:** `POST /telemetry/event`
- **Permagent usage:** Available but unused — endpoint exists but no frontend calls it
- **Notes:** Analytics infrastructure.

### 60. Diagnostics Bundle
- **Location:** `crates/goose-server/src/routes/status.rs`
- **Description:** Generates zip with logs, session state, system info.
- **Public surface:** `GET /diagnostics/{session_id}`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Debugging aid.

### 61. Dashboard API
- **Location:** `crates/goose-server/src/routes/dashboard.rs`
- **Description:** Summary stats, active sessions, in-flight/recent session info.
- **Public surface:** `GET /api/dashboard`
- **Permagent usage:** In use — dashboard hooks in Command Center
- **Notes:** Active.

---

## Identity & Configuration

### 62. Agent Identity (Persona)
- **Location:** `crates/goose/src/config/agent_identity.rs`, `identity/canonical.rs`
- **Description:** Worker personas with custom instructions. Identity resolution. HTTP routes for get/put.
- **Public surface:** `GET|PUT /agent/identity`, worker persona CRUD at `/agent/workers/*`
- **Permagent usage:** In use — identity loaded at startup, displayed in chat
- **Notes:** goose2 has full persona CRUD Tauri commands; desktop does not.

### 63. Configuration Management
- **Location:** `crates/goose/src/config/base.rs`, server routes
- **Description:** Global config from `~/.config/permagent/config.yaml` and env vars. HTTP routes for read/upsert.
- **Public surface:** `Config::global()`, `GET|POST /config`, `POST /config/read`, `POST /config/upsert`
- **Permagent usage:** In use — provider setup
- **Notes:** Active.

### 64. System Prompt Customization
- **Location:** `crates/goose-server/src/routes/prompts.rs`
- **Description:** List, get, update, reset system prompt templates.
- **Public surface:** `GET|PUT|DELETE /config/prompts/{name}`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Power-user feature. Enables deep agent behavior customization.

---

## Server (Daemon)

### 65. Session Reply & SSE Streaming
- **Location:** `crates/goose-server/src/routes/reply.rs`, `session_events.rs`
- **Description:** Modern per-session reply with request_id routing. SSE stream at `/sessions/{id}/events`. Cancel support.
- **Public surface:** `POST /sessions/{id}/reply`, `GET /sessions/{id}/events`, `POST /sessions/{id}/cancel`
- **Permagent usage:** In use — core chat functionality
- **Notes:** Primary message pathway.

### 66. File Attachments
- **Location:** `crates/goose-server/src/routes/attachments.rs`
- **Description:** Upload files (multipart, max 50MB/file, 500MB total), download, delete.
- **Public surface:** `POST /api/sessions/{id}/upload`, `GET|DELETE /api/sessions/{id}/attachments/{id}`
- **Permagent usage:** In use — file uploads in chat
- **Notes:** Active.

### 67. Brain Search API
- **Location:** `crates/goose-server/src/routes/brain.rs`
- **Description:** Hybrid search combining FTS (chat history) and Spectral (semantic similarity). Filtering by date range and source.
- **Public surface:** `GET /api/brain/search`, `GET /api/brain/graph`
- **Permagent usage:** In use — Brain view in Command Center
- **Notes:** Permagent-specific. Active.

### 68. Action Required (Permission Confirmations)
- **Location:** `crates/goose-server/src/routes/action_required.rs`
- **Description:** Handle tool permission confirmations from frontend.
- **Public surface:** `POST /action-required/tool-confirmation`
- **Permagent usage:** Internal only — permission system
- **Notes:** Bridges frontend approval UX with backend permission gate.

### 69. MCP Sampling
- **Location:** `crates/goose-server/src/routes/sampling.rs`
- **Description:** Direct model inference without agent state, for MCP sampling protocol.
- **Public surface:** `POST /sessions/{id}/sampling/message`
- **Permagent usage:** Available but unused — MCP protocol compatibility
- **Notes:** Needed if Permagent hosts MCP servers for external clients.

---

## Gateway & Integrations

### 70. Gateway System (Telegram, Chat Platforms)
- **Location:** `crates/goose/src/gateway/`, server routes at `/gateway/*`
- **Description:** Multi-platform chat gateway. Telegram implementation. Start/stop/restart gateways. Pairing codes.
- **Public surface:** `POST /gateway/start|stop|restart`, `GET /gateway/status`, CLI `gateway` command
- **Permagent usage:** Available but unused — backend fully implemented, no UX
- **Notes:** High-value candidate. Enables Permagent as multi-platform agent.

### 71. Gmail Integration
- **Location:** `crates/goose-server/src/routes/integrations.rs`
- **Description:** OAuth flow for Gmail. Connect/disconnect/status endpoints.
- **Public surface:** `POST /integrations/gmail/connect`, `GET /integrations/gmail/callback`, `DELETE /integrations/gmail`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Enables email-aware agent.

### 72. Tunnel (External Access)
- **Location:** `crates/goose-server/src/routes/tunnel.rs`
- **Description:** Lapstone tunnel for external access to daemon.
- **Public surface:** `POST /tunnel/start|stop`, `GET /tunnel/status`
- **Permagent usage:** Available but unused — no frontend surface
- **Notes:** Enables remote access to local daemon.

---

## CLI

### 73. Terminal-Integrated Sessions
- **Location:** `crates/goose-cli/src/commands/term.rs`
- **Description:** Persistent goose session per terminal via `$AGENT_SESSION_ID`. Shell integration (bash/zsh/fish/powershell) with preexec hooks, `@goose` alias, prompt info display.
- **Public surface:** `permagent term init|log|run|info`
- **Permagent usage:** Available but unused — CLI-only
- **Notes:** Shell integration pattern could inform Permagent's terminal awareness.

### 74. Headless Recipe Execution
- **Location:** `crates/goose-cli/src/commands/session.rs`, `cli.rs`
- **Description:** Execute recipes/instructions without interactive prompts. Text, JSON, stream-JSON output formats. Stdin support.
- **Public surface:** `permagent run --instructions|--text|--recipe`
- **Permagent usage:** Available but unused — CLI-only
- **Notes:** Enables scripting and CI/CD integration.

### 75. Memory CLI
- **Location:** `crates/goose-cli/src/commands/memory.rs`
- **Description:** Direct spectral DB access. Search, list with filtering, add memories with categorization.
- **Public surface:** `permagent memory search|list|add`
- **Permagent usage:** Available but unused — CLI-only
- **Notes:** No equivalent in Permagent UX.

### 76. Daemon Lifecycle (launchd)
- **Location:** `crates/goose-cli/src/commands/daemon.rs`
- **Description:** Manage daemon via launchctl on macOS. Start/stop/restart/status/logs.
- **Public surface:** `permagent start|stop|restart|status|logs`
- **Permagent usage:** Already replaced — desktop shell manages daemon via sidecar
- **Notes:** CLI approach vs desktop sidecar approach.

### 77. Doctor Health Checks
- **Location:** `crates/goose-cli/src/commands/doctor.rs`
- **Description:** Health checks with interactive fixes. Environment, dependencies, credentials, provider status.
- **Public surface:** `permagent doctor`
- **Permagent usage:** Available but unused — goose2 has Tauri commands, desktop does not
- **Notes:** Useful for Settings view.

---

## Tauri Shell (goose2)

### 78. goose2 Divergent Capabilities
- **Location:** `ui/goose2/src-tauri/src/commands/` (agents.rs, projects.rs, git.rs, extensions.rs, credentials.rs, doctor.rs, system.rs, agent_setup.rs)
- **Description:** goose2 has 30+ Tauri commands not present in desktop shell: persona CRUD, project management, git operations (branch switching, worktrees, stash), extension management, credential management, health checks, agent provider installation, file system utilities.
- **Public surface:** See detailed comparison table below
- **Permagent usage:** Already replaced — desktop shell has different architecture (daemon-centric vs client-side)
- **Notes:** Critical comparison. Many goose2 features should be evaluated for desktop shell adoption.

**goose2 vs desktop divergence table:**

| Capability | goose2 | desktop |
|---|---|---|
| Persona/Agent CRUD | 10 commands | 0 |
| Project CRUD | 9 commands | 0 |
| Git operations | 8 commands | 0 |
| Extension management | 4 commands | 0 |
| Credential management | 5 commands | 0 |
| Doctor health checks | 2 commands | 0 |
| Agent provider setup | 4 commands | 0 |
| System utilities | 8 commands | 0 |
| Activity emission | 1 command | 1 command |
| PTY hosting | 0 | 4 commands |
| Browser overlay | 0 | 6 commands |
| Daemon token | 0 | 1 command |

---

## Cross-Cutting Observations

The goose codebase is a **substantial, well-architected runtime** with significantly more capability than Permagent currently exposes. The core agent loop, provider abstraction, and MCP integration are production-quality and form a solid foundation. The security pipeline (multi-stage tool inspection) is notably thorough with pattern-based, ML-based, and LLM-based security layers.

**Architecture tension:** The codebase serves two masters. It was designed as a general-purpose agent framework (the original Block/Goose vision: provider-agnostic, extension-driven, recipe-based workflows) and is now being specialized as Permagent's runtime (opinionated, Anthropic-focused, with proprietary features like Spectral Brain and activity awareness). This creates dead weight — 30+ provider implementations for services Permagent may never use, and a recipe/skill system designed for a different UX than what Permagent is building.

**Stale areas:** The gateway system (Telegram), tunnel support, and several OAuth integrations (Tetrate, Nanogpt) appear minimally maintained. The goose2 Tauri shell has diverged significantly from the desktop shell and represents a parallel codebase that's drifting. The ACP (Anthropic Custom Protocol) integration spans multiple files but seems tightly coupled to Block's internal infrastructure.

**Active areas:** The activity awareness layer (Phase 3.5), Brain integration, and session management are under active development with recent commits. The core agent loop and provider interface are stable and well-tested. The platform extensions system is the most important architectural feature — it provides the extensibility that Permagent can build on.

**Conventions:** Consistent use of async/await with Tokio. Config stored in `~/.config/permagent/`. Secrets in `~/.permagent/secrets/`. Events use a typed enum taxonomy with replay buffers. The codebase favors composition (extensions as MCP clients) over inheritance. Error handling uses anyhow for application errors, thiserror for library errors.

**Dependencies of note:** Spectral (external git dep for Brain/memory), rmcp (MCP protocol client), tiktoken-rs (token counting), portable-pty (terminal), reqwest (HTTP), axum (server), SQLx (database). Heavy dependency on chrono, serde, and uuid throughout.

---

## Open Questions

1. **Provider surface area:** Permagent ships 40+ provider implementations. Does Permagent need all of them, or should it ship with Anthropic + OpenAI + Ollama and let users add custom providers? The provider code is ~15,000 lines.

2. **Recipe vs Automate:** The recipe system is designed for shareable, parameterized workflows. Permagent's Automate tab could build on recipes or could use a different abstraction. Should recipes become the backing format for the Automate tab?

3. **goose2 vs desktop:** goose2 has 50+ Tauri commands that desktop lacks (persona management, project CRUD, git integration, extension management). Should these be ported to desktop, accessed via daemon HTTP routes instead, or abandoned?

4. **Gateway strategy:** The Telegram gateway works but has no UX. Should Permagent expose gateway management (bringing the agent to Slack/Discord/Telegram), or is this out of scope?

5. **Chat Recall vs Brain:** The `chatrecall` extension searches past sessions, while Brain (Spectral) provides semantic memory. Are these complementary or redundant? Should chatrecall feed into Brain?

6. **Local inference:** The local model support (Ollama, GGUF downloads) is feature-gated. Is local/offline inference on Permagent's roadmap?

7. **Prompt customization:** System prompts are customizable via `/config/prompts/` endpoints but have no UX. Should power users be able to modify system prompts through the Settings view?

8. **Session forking:** Fork endpoint exists but is uncalled. Could this power a "branch conversation" UX in chat?

9. **Thread management:** Conversation threading infrastructure exists but is unused. Could this enable multi-thread conversations in the chat UX?

10. **Scheduler ownership:** The cron scheduler is fully implemented. Should scheduled tasks be visible in the Automate tab or as a standalone feature?

---

## Top Harvest Candidates

### 1. Scheduler (Cron-Based Automation)
The scheduler is **production-ready** with cron parsing, job persistence, cancellation, and worker personas. It could power the Automate tab's recurring task feature immediately — users could schedule "summarize my email every morning" or "check deployment status every hour." The daemon routes exist; only a frontend surface is missing.

### 2. Gateway System (Multi-Platform Chat)
The Telegram gateway implementation demonstrates the pattern for bringing Permagent to external platforms. With the Hub System on the roadmap, gateways could let the agent receive and respond on Slack, Discord, or Telegram — turning Permagent into a persistent agent reachable from anywhere. Backend is complete; needs UX for configuration and monitoring.

### 3. Recipe System (Workflow Templates)
Recipes are a complete workflow definition format with parameters, sub-recipes, retry config, and deeplink sharing. For the Automate tab, recipes could be the underlying format — users build recipes visually, share them via deeplinks, and schedule them. The encode/decode/validate/scan endpoints are already there.

### 4. Session Forking & Thread Management
Session forking (`POST /sessions/{id}/fork`) and thread management (`ThreadManager`) together could enable a "branch conversation" UX — try an alternative approach without losing the original thread, then merge back or discard. This is a differentiated feature for power users exploring complex problems.

### 5. Chat Recall Extension
The `chatrecall` extension searches past conversations with date filtering. Combined with Brain, this gives the agent cross-session memory without manual context loading. Enabling it by default (or auto-enabling when Brain has sufficient history) would make the agent noticeably smarter about the user's past work.

### 6. System Prompt Customization
The prompt CRUD endpoints let users modify system.md, compaction.md, and other prompts. Exposing this in Settings (with a "reset to default" button) gives power users deep control over agent behavior — a differentiator for technical users who want to tune the agent's personality and capabilities.

### 7. Git Integration (from goose2)
goose2's git commands (branch switching, worktree management, stash, fetch/pull) could feed into the Build tab's terminal experience. The agent could see git state, suggest branches, and help manage worktrees — especially powerful when combined with the activity awareness layer that already tracks terminal CWD.

### 8. Tool Permission Configuration
Per-tool permission levels (AllowAlways, AskOnce, AskAlways) exist as HTTP endpoints. Exposing these in Settings would let users fine-tune which operations the agent can perform autonomously vs. needing approval — important for trust and safety in production use.

### 9. Apps Extension (Agent-Generated UIs)
The Apps extension lets the agent create HTML/CSS/JS applications from natural language descriptions. This could power a "canvas" or "artifact" feature in the chat — the agent generates interactive visualizations, dashboards, or tools that the user can interact with directly in the app.

### 10. Local Model Support
For privacy-sensitive users or offline scenarios, local model support (Ollama, GGUF) enables running the agent without cloud API calls. This is increasingly important for enterprise adoption and could be a Settings toggle for "run locally."
