# Phase 2 Foundation Audit: Projects & Orchestrator

**Date:** 2026-05-15
**Scope:** Epics #69 (Projects as Workspaces) and #59 (Permagent as Orchestrator)
**Author:** Claude (audit only, no code changes)

---

## 1. Executive Summary

**What exists today.** Projects are a config-based, filesystem-backed concept living in `~/.goose/projects/<slug>/project.json`. Each project stores UI preferences (name, color, prompt, working directories, preferred provider) and is managed entirely through Tauri commands — there is no database table. The orchestrator is a real, working platform extension that can spawn sub-agent sessions, send them messages, and interrupt them. It operates on sessions, not goals. There is no Goals primitive, no Worker Registry, no lifecycle handoff protocol, and no Kanban UI anywhere in the codebase.

**What's missing.** The entire Phase 2 data layer is absent: no `projects` SQL table, no `goals` table, no `project_id` foreign key on sessions/memories/entities/tasks/skills in the Spectral schema, no worker capability declarations, no state machine for goal lifecycle, and no Kanban or board UI. The existing project IDs are hash-based strings generated at creation time (not UUIDs, not database-assigned). Thread metadata carries an optional `project_id` string but it's opaque — it's set via activity events when a user starts a chat from a project, and has no foreign-key integrity. The activity system tracks an `active_project_id` in its `LiveState` and renders it in ambient context, but this is purely informational (injected into the system prompt as `PROJECT: <slug>`).

**Recommended critical path.** The smallest increment that unblocks a social-posting worker: (1) Add a `projects` table to Spectral with stable UUIDs, migrate existing filesystem projects, and add `project_id` FK to `sessions` and `tasks`; (2) Add a `goals` table with status enum and `project_id` FK; (3) Extend the orchestrator extension with a minimal worker registry (static capability declarations in code, queryable via a new `list_workers` tool); (4) Add goal lifecycle state transitions inside the orchestrator. This is ~4 focused issues. Kanban UI is not required for the social worker to function and can follow.

---

## 2. Section A: Projects Audit

### Where do projects physically live?

**Filesystem.** Each project is a directory under `~/.goose/projects/<slug>/` containing a `project.json` file.

```rust
// ui/goose2/src-tauri/src/commands/projects.rs:5-8
fn projects_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    Ok(home.join(".goose").join("projects"))
}
```

The Tauri command `list_projects()` (line 197) scans this directory, reads each `project.json`, deserializes into `StoredProjectInfo`, and returns them sorted by `order`.

**No SQLite table.** There is no `CREATE TABLE projects` anywhere in `crates/goose/src/session/spectral_schema.rs`. The `spectral_schema.rs` file defines tables for: users, sessions, messages, threads, thread_messages, memories, knowledge_graph, tasks, skills, skill_executions, skill_triggers, skill_dismissals, integrations, provider_inventory_entries, provider_inventory_models, workspaces, attachments. None of these is "projects."

**localStorage cache.** The frontend caches projects in `window.localStorage` under key `"goose:projects"` for instant rendering before the Tauri IPC round-trip completes (`projectStore.ts:11-35`).

### Shape of a Project

**Rust (stored on disk):**
```rust
// ui/goose2/src-tauri/src/commands/projects.rs:127-151
struct StoredProjectInfo {
    pub id: String,              // hash-based, NOT UUID
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub icon: String,
    pub color: String,
    pub preferred_provider: Option<String>,
    pub preferred_model: Option<String>,
    pub working_dirs: Vec<String>,
    pub use_worktrees: bool,
    pub order: i32,
    pub archived_at: Option<String>,
    pub created_at: String,      // milliseconds-since-epoch as string
    pub updated_at: String,
}
```

**Rust (returned to frontend) adds `artifacts_dir`:**
```rust
// ui/goose2/src-tauri/src/commands/projects.rs:153-174
pub struct ProjectInfo {
    // ... all StoredProjectInfo fields ...
    pub artifacts_dir: String,  // computed: <project_dir>/artifacts
}
```

**TypeScript:**
```typescript
// ui/goose2/src/features/projects/api/projects.ts:3-19
export interface ProjectInfo {
  id: string;
  name: string;
  description: string;
  prompt: string;
  icon: string;
  color: string;
  preferredProvider: string | null;
  preferredModel: string | null;
  workingDirs: string[];
  useWorktrees: boolean;
  order: number;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  artifactsDir: string;
}
```

**ID generation** uses `DefaultHasher` on nanoseconds + PID, formatted as a pseudo-UUID string (`projects.rs:10-34`). These are NOT proper UUIDs and have no collision guarantees beyond the hash.

### How is the "active project" tracked?

**Frontend (zustand store):** `useProjectStore` has `activeProjectId: string | null` and `setActiveProject(id)` (`projectStore.ts:40,69,179`). This is in-memory only — not persisted across app restarts.

**Activity system:** When a user starts a chat from a project, `AppShell.tsx:339-344` emits a `project_selected` activity event with `project_id: project.id`. The activity ingestion layer (`ingestion.rs:227-263`) updates an `ActiveProject` struct in an `RwLock`. The context builder (`context_builder.rs:56-59,203,277`) propagates this into `LiveState.active_project_id` and renders it into the ambient context system prompt.

**Thread metadata:** `ThreadMetadata` has `project_id: Option<String>` (`thread_manager.rs:28`). This is stored in `threads.metadata_json` column. Set when a thread is created within a project context.

### When the user switches projects, what side effects happen?

1. `setActiveProject(id)` updates zustand in-memory state
2. A `project_selected` activity event is emitted via Tauri IPC
3. The activity ingestion layer updates `self.active_project` (the wing slug is derived)
4. The next Henry turn's ambient context will include `PROJECT: <slug>`
5. A new session/tab is created with the project's system prompt (`chatProjectContext.ts:77-133` builds `<project-settings>`, `<project-file-policy>`, `<project-instructions>` blocks)

No database rows are updated. No memories are re-tagged. No Brain filtering changes.

### Is project_id propagated to other entities?

| Entity | Propagated? | Citation |
|--------|-------------|----------|
| **Sessions** | PARTIAL | Thread metadata has `project_id: Option<String>` (`thread_manager.rs:28`) but the `sessions` SQL table has no `project_id` column (`spectral_schema.rs:63-90`) |
| **Threads** | PARTIAL | Via `metadata_json` blob, not a first-class column (`thread_manager.rs:26-28`) |
| **Memories** | NO | `memories` table has `wing` and `hall` but no `project_id` (`spectral_schema.rs:173-210`) |
| **Entities/Knowledge Graph** | NO | `knowledge_graph` table has no `project_id` (`spectral_schema.rs:259-287`) |
| **Tasks** | NO | `tasks` table has `user_id` and `session_id` but no `project_id` (`spectral_schema.rs:302-335`) |
| **Skills** | NO | `skills` table has no `project_id` (`spectral_schema.rs:342-366`) |

### Database migrations related to projects

None. The `spectral_schema.rs` file contains the full schema and migration functions (`run_migrations_v2` through `run_migrations_v14`). None of these add a projects table or project_id columns.

### CLI Project Tracker (separate system)

The CLI has a completely separate project tracking mechanism:
- `crates/goose-cli/src/project_tracker.rs` — `ProjectTracker` stores a `HashMap<String, ProjectInfo>` in `~/.permagent/data/projects.json`
- Its `ProjectInfo` is different: `{ path, last_accessed, last_instruction, last_session_id }` (line 11-19)
- This is keyed by filesystem path, not by project ID
- It has NO relationship to the Tauri projects system

---

## 3. Section B: Orchestrator Audit

### What does the orchestrator extension actually do today?

The orchestrator (`crates/goose/src/agents/platform_extensions/orchestrator.rs`) is a platform extension that provides **session management tools** to the primary agent (Henry). It exposes 5 tools:

1. **`list_sessions`** (line 598-602) — Lists agent sessions with status (loaded/busy/idle), filterable by session type, returns most recent N
2. **`view_session`** (line 603-608) — Views a session's conversation (first+last message, or LLM summary)
3. **`start_agent`** (line 609-614) — Spawns a new agent session with its own working directory. Can optionally set a `worker_persona` from `agent.yaml` workers section (line 427-439)
4. **`send_message`** (line 615-619) — Sends a message to an existing agent session and streams the response back. Handles busy detection and parent cancellation
5. **`interrupt_agent`** (line 620-627) — Cancels a busy agent's current operation

**Key architectural facts:**
- It's registered as a default-enabled platform extension (`mod.rs:228-239`)
- It's deliberately excluded from `AGENT_EXTENSIONS` — "it manages sessions but isn't a character" (`mod.rs:91`)
- It uses `AgentManager` for session lifecycle and `SessionManager` for storage
- The `start_agent` tool creates `SessionType::User` sessions (line 402) — not a dedicated orchestrated type
- Worker persona support exists but is config-based (`agent.yaml` workers section), not a capability registry
- No goal concept exists — the orchestrator operates purely on sessions and messages

### Is there a Goals primitive?

**NO.** Grep for `struct Goal`, `enum Goal`, `trait Goal` across all Rust code returns zero matches. The existing `tasks` table (`spectral_schema.rs:302-317`) is Spectral's task tracking for tool executions (with `tool_used`, `argument_shape_hash`, `steps_json`). It is NOT an orchestrator goals system — it tracks what tools the agent called, not user-facing intents.

### Is there a Worker Registry or capability declaration?

**NO.** Grep for `WorkerRegistry`, `worker_registry`, `register_worker` returns zero matches. The closest concept is:
- `agent.yaml` workers section (read by `load_agent_config()` at `orchestrator.rs:428-429`) — but this is persona configuration (system prompts, display names), not capability declarations
- `PLATFORM_EXTENSIONS` static map (`mod.rs:93-285`) — lists all extensions but has no capability/skill declaration fields
- `AGENT_EXTENSIONS` (`mod.rs:91`) — currently just `["librarian"]`, a hard-coded list of extensions that appear in World View

### Is there a lifecycle handoff protocol?

**NO.** The orchestrator's `send_message` (line 453-566) is a synchronous request-response: send text, collect all `AgentEvent::Message` responses, return concatenated text. There is no concept of "done", "needs review", "review passed", "review failed" message formats. No state transitions. No timeout/SLA detection. No ping-pong loop detection.

### Is there any Kanban or board UI?

**NO.** The only "kanban" reference in the UI is `FolderKanban` — a Lucide icon used in `ProjectsView.tsx:9,258` (empty state illustration) and `SettingsModal.tsx:27,57` (settings nav icon). There is no board, column, card, or drag-between-columns UI anywhere.

### Orchestrator in builtin_personas.rs

The `builtin_personas.rs` file contains one reference: `"One orchestrator, many subagents. Subagents do not spawn their own subagents. You are the only coordinator."` (line 42). This is a system prompt instruction, not a data model.

---

## 4. Section C: Gap Analysis

| Sub-issue | Status | What Exists | What's Missing |
|-----------|--------|-------------|----------------|
| **#60** Goals as first-class primitive | NOT_STARTED | Spectral `tasks` table exists but tracks tool executions, not user goals. No `Goal` struct/enum/trait anywhere. | Need `goals` table (id, prompt, status, assigned_worker, project_id, parent_goal_id, history_json), status enum (triage/ready/in_progress/review/complete), audit log per transition |
| **#61** Worker registry + capability protocol | NOT_STARTED | `agent.yaml` workers has persona configs; `PLATFORM_EXTENSIONS` static map lists extensions. `/api/agents` returns 404. | Need registry struct with declared capabilities per worker, goal-type-to-capability matching, availability tracking, `/api/agents` endpoint returning real data |
| **#62** Lifecycle handoff protocol | NOT_STARTED | Orchestrator has `send_message` (fire-and-wait) and `interrupt_agent`. No structured response format. | Need standard message format (done/needs_review/review_passed/review_failed), automatic state advancement, timeout detection, ping-pong loop detection, fallback routing |
| **#63** Kanban-style goals view | NOT_STARTED | `FolderKanban` icon used decoratively. No board/column/card components. | Full Kanban UI: columns by goal state, cards per goal, drag-and-drop reassign, filter by worker/project/time |
| **#67** Goal-to-tasks decomposition | NOT_STARTED | Orchestrator can spawn sessions and send messages. No decomposition logic. | Henry's decomposition engine: break goal into tasks, dispatch to workers, observe results, decide next step. Support single-agent, parallel, hierarchical patterns |
| **#70** Schema + project_id propagation | NOT_STARTED | Projects live in filesystem JSON. Thread metadata has optional project_id string (no FK). Activity system tracks active_project_id in memory. | Need `projects` table in Spectral DB, `project_id` FK on sessions/tasks/memories/entities/skills, migration for existing filesystem projects, default-scope handling |
| **#71** Workspace UI | NOT_STARTED | ProjectsView shows a flat list of project cards with CRUD. Sidebar shows projects with nested chat sessions. | Need workspace shell: project selector dropdown, left panel = Kanban filtered by project, right panel = project details, card-select rebinds right panel to goal context |
| **#72** Details: services/credentials | NOT_STARTED | Project has `prompt` and `workingDirs` fields only. No services/credentials concept. | Need per-project service entries (type, account_email, URL, environment, credential_reference). NEVER store credentials. |
| **#73** Details: resources/people/activity | NOT_STARTED | No resources, people, or activity feed in project model. | Need repo links, doc links, collaborators, recent activity feed, free-form notes |
| **#74** Brain integration | NOT_STARTED | Memories have `wing` field (can hold project slug). Activity system derives wing from project_id. No UI filter chip for project. | Need `project_id` on memories, Brain view filter chip, automatic project tagging on memory creation, entity-to-project relationships, memory retagging for backfill |
| **#75** Project-scoped Henry | NOT_STARTED | Henry sees `PROJECT: <slug>` in ambient context. No credential lookup, no project-scoped worker matching. | Need Henry to look up project services/credentials when acting, Librarian project tags, project-scoped worker capability matching |
| **#76** Cross-project flows | NOT_STARTED | No cross-project goal concept. No memory promotion. Default-scope memories have no explicit handling. | Need multi-project goals, memory promotion (personal -> project-tagged), default-scope handling for untagged memories |

---

## 5. Section D: Critical Path for Social-Posting Worker

A social-posting worker needs to register itself, receive goals, report status, and have those goals tied to a project. Here is the **minimum** Phase 2 increment:

### D1. Projects table with stable IDs (#70 minimum)

**Scope:** Add `CREATE TABLE projects` to `spectral_schema.rs` with: `id TEXT PRIMARY KEY` (UUID v4), `slug TEXT UNIQUE`, `name TEXT`, `description TEXT`, `status TEXT DEFAULT 'active'`, `created_at`, `updated_at`. Add `project_id TEXT REFERENCES projects(id)` to the `sessions` table. Write a migration that reads existing `~/.goose/projects/*/project.json` files and inserts rows. Keep the filesystem as the source of truth for UI settings (color, icon, prompt) for now — the DB table is for relational integrity.

**Cut from scope:** Don't add `project_id` to memories/entities/skills yet. Don't build the workspace UI. Don't migrate away from filesystem for project CRUD.

**Estimated files touched:** `spectral_schema.rs` (new table + migration), `session_manager.rs` (write project_id on session creation)

### D2. Goals table (#60 minimum)

**Scope:** Add `CREATE TABLE goals` with: `id TEXT PRIMARY KEY` (UUID v4), `project_id TEXT REFERENCES projects(id)`, `prompt TEXT`, `status TEXT NOT NULL DEFAULT 'triage'`, `assigned_worker TEXT`, `parent_goal_id TEXT REFERENCES goals(id)`, `created_at`, `updated_at`. Add `CREATE TABLE goal_transitions` for audit log: `id, goal_id, from_status, to_status, actor, reason, created_at`. Status enum: `triage`, `ready`, `in_progress`, `review`, `complete`, `blocked`.

**Cut from scope:** No decomposition logic. No hierarchical goal trees. Goals are flat for now — one goal, one worker.

**Estimated files touched:** `spectral_schema.rs` (new tables), new `crates/goose/src/session/goal_manager.rs` (CRUD + state transitions)

### D3. Worker registry (#61 minimum)

**Scope:** Add a `WorkerCapability` struct with: `worker_key: String`, `display_name: String`, `capabilities: Vec<String>` (e.g., `["social_post", "content_create"]`), `available: bool`. Implement as a static registry in the orchestrator extension (not a DB table yet). Add a `list_workers` tool to the orchestrator. The social worker registers itself by adding an entry to the registry at startup.

**Cut from scope:** No dynamic registration API. No project-scoped workers. No availability polling.

**Estimated files touched:** `orchestrator.rs` (new tool + registry struct), new worker definition file

### D4. Goal lifecycle in orchestrator (#62 minimum)

**Scope:** Add `create_goal`, `update_goal_status`, `list_goals` tools to the orchestrator extension. `create_goal` inserts into the goals table. `update_goal_status` validates transitions (triage->ready->in_progress->review->complete) and writes to `goal_transitions`. Workers call `update_goal_status` to report progress. The orchestrator can query goals by project and status.

**Cut from scope:** No automatic state advancement. No timeout detection. No ping-pong detection. No structured message format — workers just call `update_goal_status` explicitly.

**Estimated files touched:** `orchestrator.rs` (3 new tools), `goal_manager.rs` (status validation)

### D5. (Optional for v1) Kanban UI (#63)

Not required for the social worker to function. The social worker operates entirely through the orchestrator tools. A Kanban can follow once goals are flowing.

---

## 6. Section E: Recommended Phase 2 Build Order

1. **#70 (Schema)** — Projects table + project_id on sessions. Foundation for everything else.
2. **#60 (Goals)** — Goals table + transitions table. Depends on #70 for project_id FK.
3. **#61 (Worker registry)** — Static registry in orchestrator. Can be done in parallel with #60.
4. **#62 (Lifecycle)** — Goal CRUD tools in orchestrator. Depends on #60.
5. **Social worker integration** — Register in worker registry, create goals, update status. Depends on #61 + #62.
6. **#63 (Kanban UI)** — Read goals by project + status, render as columns + cards. Depends on #60.
7. **#71 (Workspace UI)** — Project selector + Kanban + details panel shell. Depends on #63.
8. **#67 (Decomposition)** — Henry's intelligence layer. Depends on #62.
9. **#72-#73 (Details panels)** — Services, resources, people. Independent of orchestrator.
10. **#74 (Brain integration)** — project_id on memories, filter chips. Depends on #70.
11. **#75 (Project-scoped Henry)** — Credential lookup, scoped workers. Depends on #72 + #61.
12. **#76 (Cross-project)** — Multi-project goals, memory promotion. Last.

**Critical path for social worker: #70 -> #60 -> #62 -> social worker (4 PRs)**
**Worker registry (#61) can be done in parallel with #60.**
