# Orchestrator Capabilities Expansion — Goal Lifecycle, Worker Dispatch, Kanban Awareness, Project Setup, Roadmap Decomposition

**Date:** 2026-05-27
**Author:** Jesse Sharratt + Claude Opus 4.6
**Epic:** #192
**Branch:** `feat/orchestrator-kanban-awareness`
**Status:** Design review complete — decisions locked. PR 2A in progress.

---

## 1. Recon Findings

### Existing Infrastructure

| Component | Exists | Missing for This Work |
|-----------|--------|-----------------------|
| Orchestrator session mgmt | 5 tools in `orchestrator.rs` (list/view/start/send/interrupt) | Kanban awareness, worker selection logic |
| Summon (delegate) | Full subagent dispatch in `summon.rs` | Goal lifecycle tracking |
| Worker personas | Identity-only in `agent_identity.rs` (agent.yaml) | Capabilities, availability, registry |
| Projects CRUD | Complete in `projects.rs` | Nothing — sufficient |
| Cards schema | `card_type='goal'` declared, `column_kind='state'` + `state_binding` in schema | Goal lifecycle columns, goal-specific MCP tools |
| Project Manager MCP | 10 tools in `project_manager.rs` (project+card+column CRUD) | `board_summary`, `goal_advance`, `goal_review` |
| System prompt injection | `add_system_prompt_extra()` in `prompt_manager.rs:253` | Not wired to Kanban state |
| Capability detection | Nothing | Codex/CC/Qwen probing entirely missing |
| UI Kanban | Drag-drop board in `ProjectsView.tsx`, goal badge rendering | Goal lifecycle UI, worker assignment display |

### Key File Paths

- **Orchestrator extension:** `crates/goose/src/agents/platform_extensions/orchestrator.rs` (685 lines)
- **Summon extension:** `crates/goose/src/agents/platform_extensions/summon.rs` (2274 lines)
- **Worker personas:** `crates/goose/src/config/agent_identity.rs` (183 lines)
- **Projects CRUD:** `crates/goose/src/projects.rs` (742 lines)
- **Cards CRUD:** `crates/goose/src/cards.rs` (958 lines)
- **Cards routes:** `crates/goose-server/src/routes/cards.rs` (396 lines)
- **Project Manager MCP:** `crates/goose/src/agents/platform_extensions/project_manager.rs` (800 lines)
- **Prompt manager:** `crates/goose/src/agents/prompt_manager.rs` (line 253: `add_system_prompt_extra`)
- **Extension registry:** `crates/goose/src/agents/platform_extensions/mod.rs` (line 247: orchestrator registration)
- **Spectral schema:** `crates/goose/src/session/spectral_schema.rs` (cards table at line 642, board_columns at line 621)
- **UI Kanban:** `ui/command-center/src/components/projects/ProjectsView.tsx` (567 lines)

### Schema Details (from spectral_schema.rs:621–693)

**board_columns** already has:
- `column_kind TEXT NOT NULL DEFAULT 'manual' CHECK (column_kind IN ('manual', 'state'))`
- `state_binding TEXT` — designed for exactly this: binding a column to a lifecycle state

**cards** already has:
- `card_type TEXT NOT NULL DEFAULT 'standard' CHECK (card_type IN ('standard', 'goal', 'social_post'))`
- `metadata_json TEXT NOT NULL DEFAULT '{}'` — the extensibility slot for goal-specific state
- `assigned_to TEXT` — for worker assignment
- `created_by TEXT NOT NULL DEFAULT 'user' CHECK (created_by IN ('user', 'henry', 'hermes', 'codex', 'claude-code', 'librarian'))`

### Relic Cleanup Note

The `created_by` CHECK constraint includes `'hermes'`. This is a relic — Hermes was a reference architecture, not an agent in Permagent. **Phase 1.5 cleanup:** remove `'hermes'` from the allowed values and add `'orchestrator'` as a generic value. Do NOT ship a standalone migration; bundle into the next v9 schema migration when it lands.

---

## 2. Cross-Cutting Design Decisions

### Q1. Worker Registry Shape — LOCKED

**Decision:** Extend the existing `WorkerPersona` in `agent_identity.rs` (config-file based at `~/.permagent/agent.yaml`). No new DB table.

**Current WorkerPersona** (`agent_identity.rs:87–99`):
```rust
pub struct WorkerPersona {
    pub first_name: String,
    pub last_name: Option<String>,
    pub nickname: Option<String>,
    pub role: String,
    pub traits: Vec<String>,
    pub tone: String,
}
```

**Proposed extension — add three fields:**
```rust
pub struct WorkerPersona {
    // ... existing fields ...

    /// What this worker can do: code_edit, shell, web_search, memory_ops, etc.
    #[serde(default)]
    pub tool_kinds: Vec<String>,

    /// How to check if this worker is available on this machine.
    /// "bin_exists:<path>" — check binary exists (e.g., "bin_exists:codex")
    /// "api_credential:<env_var>" — check env var or keychain entry exists
    /// "model_loaded:<model_name>" — check Ollama/local model is pulled
    /// "always" — always available (e.g., the Orchestrator itself)
    #[serde(default = "default_availability")]
    pub availability_check: String,

    /// Cost classification for selection heuristics.
    /// "local_free" — runs locally, no API cost (Qwen, Librarian)
    /// "subscription" — user pays flat monthly (Codex, Claude Code)
    /// "paid_api" — per-token cost (direct API calls)
    #[serde(default = "default_cost_tier")]
    pub cost_tier: String,
}
```

**Why config-file, not DB:**
- Workers are per-machine, not per-project. A Mac mini running Qwen has different workers than a laptop.
- `agent.yaml` is already loaded by `load_agent_config()` and used by both `orchestrator.rs:428` and `summon.rs:1042`.
- Hot-reload via `SharedAgentConfig` (`agent_identity.rs:170`) already exists.
- No schema migration needed.

**Example agent.yaml:**
```yaml
primary:
  first_name: Henry
  # ...

workers:
  codex:
    first_name: Codex
    role: "Fast parallel coding agent"
    tool_kinds: [code_edit, shell, git]
    availability_check: "bin_exists:codex"
    cost_tier: subscription
  claude_code:
    first_name: Claude
    nickname: CC
    role: "Deep reasoning coding agent"
    tool_kinds: [code_edit, shell, git, web_search]
    availability_check: "bin_exists:claude"
    cost_tier: subscription
  qwen_coding:
    first_name: Qwen
    role: "Local coding model"
    tool_kinds: [code_edit, shell]
    availability_check: "model_loaded:qwen2.5-coder:32b"
    cost_tier: local_free
  librarian:
    first_name: Librarian
    role: "Memory description and organization"
    tool_kinds: [memory_ops]
    availability_check: "always"
    cost_tier: local_free
```

### Q2. Goal Lifecycle State Machine — LOCKED

**5 states:** Triage → Ready → InProgress → Review → Complete

```
 ┌─────────┐    ┌───────┐    ┌────────────┐    ┌────────┐    ┌──────────┐
 │ Triage  │───▶│ Ready │───▶│ InProgress │───▶│ Review │───▶│ Complete │
 └─────────┘    └───────┘    └────────────┘    └────────┘    └──────────┘
                                   ▲                │
                                   └────────────────┘
                                    (bounce-back)
```

**State storage:** `column_kind='state'` columns with `state_binding` set to the state name. Goal cards live in state-bound columns. Standard cards live in `column_kind='manual'` columns. The existing `move_card()` function (`cards.rs:513`) handles the physical move; the new lifecycle API layer adds transition validation on top.

**State-bound columns per project** (seeded on first goal creation, not on project creation — avoids cluttering projects that never use goals):
- `Triage` (state_binding: `triage`)
- `Ready` (state_binding: `ready`)
- `In Progress` (state_binding: `in_progress`)
- `Review` (state_binding: `review`)
- `Complete` (state_binding: `complete`)

**Transition rules:**
| From | To | Who | Condition |
|------|----|-----|-----------|
| Triage | Ready | Orchestrator (auto) or user | Goal is well-defined enough to assign |
| Ready | InProgress | Orchestrator | Worker selected and dispatch started |
| InProgress | Review | Orchestrator | Worker reports completion |
| Review | Complete | User or Orchestrator (configurable) | Work accepted |
| Review | InProgress | User or Orchestrator | Work rejected, bounce-back |

**metadata_json for goal cards:**
```json
{
  "goal_state": "in_progress",
  "worker_key": "codex",
  "worker_session_id": "20260527_3",
  "dispatched_at": "2026-05-27T14:30:00Z",
  "last_check_in": "2026-05-27T14:35:00Z",
  "review_notes": null,
  "attempt_count": 1,
  "depends_on": ["<card_id>"]
}
```

**Locked decisions:**
- **Review → Complete scope — LOCKED:** Always Review. The Orchestrator never auto-completes a goal. After a worker reports completion, the card moves to Review and waits for user approval. A power-user override (`metadata_json.auto_approve: true`) that allows specific goals to skip Review is deferred to a future polish PR — do not implement in PR 2B.

**Open questions for Jesse:**
- **Triage → Ready:** Orchestrator auto-promotes if goal has enough detail, or always requires user confirmation?
- **InProgress UI:** Show live worker output (requires WebSocket/SSE to card), or static "worker is running" until Review? The existing `is_session_busy()` check from `orchestrator.rs:206` could feed a polling endpoint.

### Q3. Worker Execution Mechanism — LOCKED

**Decision:** The goal lifecycle uses `summon.delegate(...)` for worker invocation. No new dispatch mechanism.

**Flow:**
```
User: "build the login page"
  → Orchestrator receives chat message
  → Orchestrator creates goal card (card_create, card_type='goal')
  → Orchestrator picks worker (see Q7)
  → Orchestrator calls summon.delegate(
      instructions: "<goal description + context>",
      worker_persona: "codex",
      async: true
    )
  → Summon spawns subagent session (SessionType::SubAgent)
  → Orchestrator writes session_id into card's metadata_json
  → Orchestrator moves card to InProgress
  → On completion: Summon marks task done → Orchestrator moves card to Review
```

**Key integration points:**
- `summon.rs:920` (`handle_delegate`) — the existing dispatch entry point
- `summon.rs:1040` (`resolve_worker_persona`) — already resolves persona from agent.yaml
- `summon.rs:74` (`BackgroundTask`) — tracks async task state
- `orchestrator.rs:360` (`handle_start_agent`) — alternative for long-lived sessions (not used for fire-and-forget goals)

**Completion detection:** The Orchestrator polls via `summon.load(source: "<session_id>")` or subscribes to `notification_subscribers` (`summon.rs:323`). When the delegate completes, the Orchestrator transitions the card to Review.

**Remote workers — LOCKED out of scope:** The Orchestrator dispatches to locally-available workers only. Henry running on Jesse's Mac mini at 192.168.2.200 is a **peer Orchestrator**, not a worker — each machine runs its own Orchestrator with its own worker roster. Peer Orchestrator interaction (one Orchestrator requesting work from another) is a Mesh/Forum concern (Phase 2+). Future contributors should not conflate "remote worker" with "peer Orchestrator": workers are local subagents on the same machine; peer Orchestrators are sovereign agents on other machines that communicate via the Forum protocol.

### Q4. State Persistence + Resumability

**What survives daemon restart:**
- Card state in SQLite (via Spectral) — the card's `column_id` (which encodes lifecycle state via `state_binding`) and `metadata_json` (which holds `worker_session_id`, `goal_state`, etc.) persist across restarts.
- Session metadata in `SessionManager` — session records persist.

**What doesn't survive:**
- In-flight Summon background tasks (`BackgroundTask` is in-memory, `summon.rs:74`).
- Active subagent state (the running agent process).

**Resume strategy:**
1. On daemon startup, scan for goal cards in `InProgress` state (query: `SELECT * FROM cards WHERE card_type='goal' AND column_id IN (SELECT id FROM board_columns WHERE state_binding='in_progress')`)
2. For each, check if the worker session (`metadata_json.worker_session_id`) is still alive via `AgentManager::is_session_busy()`
3. If dead: increment `attempt_count` in metadata, move card back to `Ready` for re-dispatch
4. If alive: re-register for completion notifications

**Failed-goal handling — LOCKED:**
- Cap `metadata_json.attempt_count` at **3 retries** on dispatch failure (worker crash, dispatch error, daemon restart with dead session).
- After 3 attempts, move the card back to **Triage** (not a new state) with `metadata_json.needs_human_attention: true` and `metadata_json.last_error: "<error string>"`.
- The Orchestrator surfaces "needs attention" goals in the ambient context summary (PR 2C) when injection fires.
- A dedicated `Failed` state column is deferred to a future polish PR. The 5-state machine remains: Triage/Ready/InProgress/Review/Complete.

### Q5. Ambient Kanban Context Shape (for 2C) — LOCKED

**Decision:** Smart injection via `add_system_prompt_extra("kanban_context", ...)` on the Orchestrator's session, plus `board_summary` MCP tool for on-demand detail.

**Injection point:** `prompt_manager.rs:253` — `add_system_prompt_extra(key, instruction)` appends to the system prompt.

**Refresh cadence — LOCKED:**

The cached board summary is refreshed:
1. At session start (first message in a new conversation)
2. After any goal state transition (dispatch, advance, review)
3. On a 5-minute stale check

The summary is **injected into the system prompt** ONLY when:
- The user's last message contains board-relevant keywords (case-insensitive match): `what`, `status`, `progress`, `working`, `stalled`, `running`, `doing`, `stuck`, `blocked`, `next`, `todo`, `task`, `goal`, `project`, `board`, `kanban`
- OR every 5th turn as a freshness floor regardless of keywords

When not injected, the `board_summary` MCP tool remains available for on-demand queries.

**Keyword list (canonical, stored as a const):**
```rust
const BOARD_KEYWORDS: &[&str] = &[
    "what", "status", "progress", "working", "stalled", "running",
    "doing", "stuck", "blocked", "next", "todo", "task", "goal",
    "project", "board", "kanban",
];
```

**Injected context format (compact, ~200 tokens for 5 projects / 30 cards):**
```
## Current Board State
Projects: 5 active | Cards: 30 active (8 goals, 22 standard)

Goals in flight:
- [InProgress] "Login page" → codex (session 20260527_3, 12min)
- [Review] "API auth middleware" → awaiting approval
- [Ready] "Dashboard layout" → unassigned

Needs attention:
- "Cache invalidation" — 3 dispatch failures, back in Triage

Stalled (no activity > 1hr):
- "DB indexing" in InProgress since 14:00 (codex, session 20260527_1)

Recent completions (24h):
- "DB schema migration" completed 2h ago
```

**Token budget:** Target < 300 tokens per injection. At ~4 chars/token, that's ~1200 chars. The format above is ~500 chars = ~125 tokens. Comfortable headroom for larger boards. For boards exceeding ~400 tokens, auto-filter to goals-only + active project.

### Q6. Capability Detection (for 2A)

**Detection strategies by availability_check value:**

| Check Type | Implementation | Latency |
|------------|----------------|---------|
| `bin_exists:<name>` | `which <name>` or `std::fs::metadata` on known paths | < 1ms |
| `api_credential:<env_var>` | `std::env::var(env_var).is_ok()` or keychain probe | < 1ms |
| `model_loaded:<model>` | HTTP GET to Ollama API (`/api/tags`) checking model list | ~50ms |
| `always` | No check, always available | 0 |

**When to probe:**
- **Startup:** Full probe of all workers, cache results
- **On demand:** Re-probe a specific worker before dispatch (in case state changed)
- **Periodic:** Optional background refresh every 5 minutes (low priority)

**Probe result struct:**
```rust
pub struct WorkerAvailability {
    pub worker_key: String,
    pub available: bool,
    pub last_checked: Instant,
    pub reason: Option<String>, // "binary not found", "model not pulled", etc.
}
```

**Cache:** `HashMap<String, WorkerAvailability>` behind a `Mutex`, held on the Orchestrator extension. TTL: 5 minutes.

### Q7. Worker Selection Algorithm

**"The Orchestrator picks a worker" means:** A deterministic rule-based selection with LLM fallback.

**Proposed algorithm (evaluate in order):**

1. **Filter unavailable:** Remove workers where `WorkerAvailability.available == false`
2. **Filter by capability:** Remove workers whose `tool_kinds` don't match the goal's requirements (inferred from goal tags or card metadata)
3. **Prefer cheapest:** Sort remaining by `cost_tier` (local_free > subscription > paid_api)
4. **Prefer least busy:** Among same-tier workers, pick the one with fewest active sessions (via `AgentManager::list_active_session_ids()`)
5. **Fallback to Orchestrator itself:** If no workers match, the Orchestrator handles it in its own session

**When deterministic fails (ambiguous capability match):** Fall back to an LLM call where the Orchestrator reasons about which worker fits, given the goal description and available workers. This is a single `complete_fast` call, similar to `summarize_conversation` at `orchestrator.rs:317`.

**Open question for Jesse:** Should user-defined priority overrides exist in agent.yaml? e.g., `priority: 1` on codex means "prefer codex over claude_code when both qualify." Or is cost_tier + least-busy sufficient?

---

## 3. PR 2A — Worker Registry + Capability Detection

### Goal
The Orchestrator can enumerate available workers, know what each can do, and detect what's installed/subscribed on this machine.

### Schema Changes
None (DB). Extend `WorkerPersona` in `agent_identity.rs` with three new `#[serde(default)]` fields: `tool_kinds`, `availability_check`, `cost_tier`.

### Files Modified

| File | Change |
|------|--------|
| `crates/goose/src/config/agent_identity.rs` | Add `tool_kinds`, `availability_check`, `cost_tier` to `WorkerPersona` (lines 87–99). Add default functions. |
| `crates/goose/src/agents/platform_extensions/orchestrator.rs` | Add `WorkerRegistry` struct with availability cache. Add `list_workers` and `check_worker` MCP tools. Add `select_worker(goal_tags) -> WorkerKey` method. |
| `~/.permagent/agent.yaml` (user config) | Users add capability fields to their worker definitions. Existing configs work unchanged due to `#[serde(default)]`. |

### New MCP Tools (on Orchestrator extension)

**`list_workers`** — List all configured workers with availability status.
```
Parameters: { refresh: Option<bool> }  // force re-probe if true
Returns: JSON array of { key, display_name, role, tool_kinds, available, cost_tier }
```

**`check_worker`** — Probe a specific worker's availability.
```
Parameters: { worker_key: String }
Returns: { available: bool, reason: Option<String> }
```

### Probe Implementation

New module: `crates/goose/src/config/worker_probe.rs` (~100 lines)
- `probe_worker(check: &str) -> (bool, Option<String>)`
- Handles `bin_exists:*`, `api_credential:*`, `model_loaded:*`, `always`
- For `model_loaded`, uses existing Ollama HTTP client (already in `crates/goose/src/agents/platform_extensions/librarian.rs` for the Librarian's model check)

### System Prompt Section
None for 2A. The Orchestrator doesn't need workers in its prompt until 2B (dispatch) and 2C (ambient context).

### Tests
- Unit: `worker_probe::probe_worker` with mocked filesystem/env
- Unit: `WorkerPersona` deserialization with and without new fields (backward compat)
- Unit: `list_workers` returns correct availability after probe
- Integration: Probe detects installed `codex` binary (skip in CI if not present)

### Estimated Effort
Small. ~200 lines of new Rust code + tests. ~150 lines of config schema extension. 1–2 sessions.

### Open Questions for Jesse
- Should `list_workers` be visible to the user (unprefixed tool), or Orchestrator-internal only? Currently proposing it as an Orchestrator tool (prefixed `orchestrator__list_workers`).

---

## 4. PR 2B — Goal Lifecycle + Orchestrator Core Dispatch

### Goal
The `card_type='goal'` lifecycle works end-to-end: create goal → Orchestrator picks worker → dispatches via Summon → tracks through Triage/Ready/InProgress/Review/Complete → reports result.

### Schema Changes

No new tables or columns. Uses existing:
- `board_columns.column_kind = 'state'` + `board_columns.state_binding` (spectral_schema.rs:626–628)
- `cards.metadata_json` for goal-specific state
- `cards.assigned_to` for worker key

**New state-bound columns** seeded per-project on first goal creation:

```sql
INSERT INTO board_columns (id, project_id, name, position, column_kind, state_binding) VALUES
  (<uuid>, <project_id>, 'Triage',      100, 'state', 'triage'),
  (<uuid>, <project_id>, 'Ready',       101, 'state', 'ready'),
  (<uuid>, <project_id>, 'In Progress', 102, 'state', 'in_progress'),
  (<uuid>, <project_id>, 'Review',      103, 'state', 'review'),
  (<uuid>, <project_id>, 'Complete',    104, 'state', 'complete');
```

Position 100+ ensures state columns sort after manual columns in the UI. The `seed_default_columns` function at `cards.rs:82` is the model — a similar `seed_goal_columns(pool, project_id)` function.

### Where Dispatch Logic Lives

**`orchestrator.rs`** — new methods on `OrchestratorClient`:
- `handle_goal_advance(card_id, target_state)` — validates transition, moves card
- `dispatch_goal(card_id)` — selects worker (from 2A), calls `summon.delegate(...)`, writes session_id to metadata_json
- `handle_goal_review(card_id, approve: bool, notes: Option<String>)` — transitions Review→Complete or Review→InProgress
- Completion listener: subscribes to Summon notification channel (`summon.rs:323`) for dispatch completion, then auto-transitions InProgress→Review

### Goal Creation — LOCKED

Goal cards are created via `project_manager.card_create` with `card_type='goal'` and a new optional `auto_dispatch: bool` parameter. The Orchestrator does NOT have a separate `goal_create` tool. This keeps card creation in one place (Project Manager extension).

When `card_type='goal'` and `auto_dispatch=true`, the Project Manager's `handle_card_create` method calls into the Orchestrator's `dispatch_goal(card_id)` method after card insertion. The Orchestrator still owns dispatch, lifecycle transitions, and review.

**Changes to `project_manager.rs`:**
- Add `auto_dispatch: Option<bool>` to `CardCreateParams` (default: false)
- In `handle_card_create`, after creating a goal card with `auto_dispatch=true`, invoke `OrchestratorClient::dispatch_goal(card_id)` (requires cross-extension call or shared function)

### New MCP Tools (on Orchestrator extension)

**`goal_advance`** — Manually advance a goal's lifecycle state.
```
Parameters: {
  card_id: String,
  action: String  // "ready", "dispatch", "review", "approve", "reject"
}
```
Validates transition rules, moves card between state-bound columns.

**`goal_status`** — Get detailed status of a goal including worker progress.
```
Parameters: { card_id: String }
Returns: { state, worker, session_id, duration, last_activity, attempt_count }
```
Uses `AgentManager::is_session_busy()` and Summon's `BackgroundTask` state.

### Worker Selection Logic

Implemented as `select_worker(&self, goal: &Card) -> Result<String, String>` on `OrchestratorClient`:

```
1. Load agent config (agent_identity.rs:140)
2. Load availability cache (from 2A)
3. Filter: available == true
4. Filter: worker.tool_kinds intersects goal_required_kinds
   (goal_required_kinds derived from metadata_json.tags or defaulting to ["code_edit", "shell"])
5. Sort: local_free first, then subscription, then paid_api
6. Among same tier: prefer worker with fewest active sessions
7. If no match: return Err("No suitable worker available")
   → Orchestrator falls back to handling it in its own session
```

### Dispatch Flow (detailed)

```
card_create(project="permagent-runtime", title="Add login page", card_type="goal", auto_dispatch=true)
  → Project Manager creates card (card_type=goal, column=Triage)
  → auto_dispatch=true triggers Orchestrator.dispatch_goal(card_id):
      select_worker(goal) → "codex"
      move card to Ready
      summon.delegate(
        instructions: "Goal: Add login page\n\nDescription: ...\nProject root: /dev/permagent-runtime",
        worker_persona: "codex",
        async: true
      ) → task_id = "20260527_3"
      update card metadata_json: { worker_key: "codex", worker_session_id: "20260527_3", dispatched_at: now }
      update card assigned_to: "codex"
      move card to InProgress
```

### Failed-Goal Handling — LOCKED

Scope for PR 2B (not deferred):
- On dispatch failure (worker crash, dispatch error, daemon restart with dead session): increment `metadata_json.attempt_count`
- After **3 failed attempts**, move card back to **Triage** with `metadata_json.needs_human_attention: true` and `metadata_json.last_error: "<error string>"`
- The ambient context summary (PR 2C) surfaces "needs attention" goals
- No dedicated `Failed` state column — deferred to future polish PR

### Tests
- Unit: State transition validation (all valid + invalid transitions)
- Unit: `seed_goal_columns` idempotency
- Unit: Worker selection with various availability/capability combos
- Unit: metadata_json updates on dispatch
- Integration: Full lifecycle Triage → Complete with mock worker
- Integration: Bounce-back from Review to InProgress

### Estimated Effort
Medium. ~500 lines new Rust in orchestrator.rs + ~100 lines in cards.rs (goal column seeding). ~200 lines tests. 2–3 sessions.

### Locked Decisions (PR 2B)
- **Goal creation placement:** Via `project_manager.card_create` with `auto_dispatch` param (not a separate Orchestrator tool). See "Goal Creation" section above.
- **Review always required:** When a worker finishes, the Orchestrator auto-moves to Review — never directly to Complete. Auto-approve override deferred to future polish PR.

---

## 5. PR 2C — Kanban-Aware Chat Surface

### Goal
The Orchestrator always knows the current board state. Users can ask "what am I working on?" and get real answers without explicit queries.

### Context Injection — LOCKED (Smart Injection)

**Mechanism:** `add_system_prompt_extra("kanban_context", formatted_summary)` on the Orchestrator's agent session.

**Implementation location:** New method `refresh_kanban_context(&self, session_id: &str)` on `OrchestratorClient`.

**Cache refresh triggers:**
1. At session start (first message in a new conversation)
2. After any goal state transition (dispatch, advance, review)
3. On a 5-minute stale check

**Injection triggers (when the cached summary is actually added to the prompt):**
- The user's last message contains a board-relevant keyword (case-insensitive): `what`, `status`, `progress`, `working`, `stalled`, `running`, `doing`, `stuck`, `blocked`, `next`, `todo`, `task`, `goal`, `project`, `board`, `kanban`
- OR every 5th turn as a freshness floor regardless of keywords

When injection does NOT fire, the `board_summary` MCP tool remains available for on-demand queries. This smart injection strategy avoids wasting ~100–300 tokens per turn on unrelated conversations while ensuring the Orchestrator is always prepared for board questions.

**Query:** Single SQL query joining `cards`, `board_columns`, and `projects`:
```sql
SELECT c.id, c.title, c.card_type, c.assigned_to, c.metadata_json,
       bc.state_binding, bc.name as column_name,
       p.name as project_name
FROM cards c
JOIN board_columns bc ON c.column_id = bc.id
JOIN projects p ON c.project_id = p.id
WHERE c.archived_at IS NULL
  AND p.status = 'active'
ORDER BY p.name, bc.position, c.position
```

**Format:** Compact text (see Q5 above). Goals get detail; standard cards get counts only.

### New MCP Tool

**`board_summary`** — Full board state on demand.
```
Parameters: {
  project_id_or_slug: Option<String>,  // omit for all projects
  include_standard_cards: Option<bool>  // default false — goals only
}
Returns: Formatted board state with card details
```

This tool is on the **Project Manager extension** (it's a read-only query, not an orchestration action). The Orchestrator calls it when it needs more detail than the injected summary provides.

### Token Implications

| Board Size | Injected Tokens | Notes |
|------------|-----------------|-------|
| 1 project, 5 cards, 2 goals | ~80 | Negligible |
| 5 projects, 30 cards, 8 goals | ~200 | Comfortable |
| 10 projects, 100 cards, 20 goals | ~400 | Still within budget |
| 20 projects, 500 cards | ~800 | May need filtering to active project only |

For boards exceeding ~400 tokens, auto-filter to goals-only + active project.

### System Prompt Changes

The Orchestrator's instructions (currently at `orchestrator.rs:117–118`) expand:

```
"Manage agent sessions and coordinate work across projects.

You have ambient awareness of all project boards. The current board state
is injected into your context and refreshed automatically. When users ask
about work status, progress, or what's happening — answer from this context
without needing to call tools.

For detailed board queries, use the board_summary tool."
```

### Tests
- Unit: `refresh_kanban_context` generates correct summary format
- Unit: Token count stays under budget for various board sizes
- Unit: Stale detection triggers refresh
- Integration: Ask "what am I working on?" returns board-aware answer

### Estimated Effort
Small. ~150 lines new Rust (query + format + refresh logic). ~50 lines instruction update. 1 session.

### Locked Decisions (PR 2C)
- **Injection strategy:** Smart injection — keyword-triggered + every-5-turns floor (see Q5 above for full keyword list)
- **Format:** Natural language — the LLM consumes it, not code

### Open Questions for Jesse
- Include standard cards in the summary, or goals only? Goals-only keeps it compact but misses user tasks.
- Include cards from ALL projects, or only the "active" project (if one is contextually active)?

---

## 6. PR 2D — Project Setup Via Chat

### Goal
User says "set up a project for X" → Orchestrator walks them through creation conversationally.

### Implementation

This is primarily a **system prompt + conversational flow** change. The existing `project_create` tool (`project_manager.rs:583–600`) already handles the actual creation. The Orchestrator just needs instructions on HOW to use it conversationally.

### System Prompt Addition

Added to the Orchestrator's instructions:

```
When users want to set up a new project, guide them through these fields:
1. Name (required) — ask for it
2. Root path — suggest based on the current working directory if available
3. Repo URL — ask, or offer to detect from git remote
4. Site URL — ask (optional)
5. Description — ask for a one-liner
6. Tags — suggest based on what they described

Then call project_create with the gathered fields.

After creation, offer two modes:
- "Roadmap mode": Help plan and decompose the work into goals
- "Task mode": Start with a single goal right away

If the user chooses roadmap mode, proceed with roadmap decomposition.
```

### Tool Access

The Orchestrator needs access to `project_create` from the Project Manager extension. Currently, tools are scoped per-extension. Two approaches:

**Option A:** The Orchestrator calls `project_create` via tool invocation (it's already available as an MCP tool on the same session).

**Option B:** The Orchestrator imports and calls `projects::create_project()` directly (Rust function call).

**Recommendation:** Option A — use the existing MCP tool. This keeps the Orchestrator as a coordinator, not a re-implementation. The tool call is visible in logs and debuggable.

### Git Remote Detection

Optional quality-of-life: when the user provides a `root_path`, the Orchestrator can detect the git remote:
```
delegate(instructions: "Run 'git -C <root_path> remote get-url origin' and return the result", extensions: [])
```
Or simpler: the Developer extension's shell tool can run this inline.

### Tests
- Unit: Instruction text includes project setup flow
- Integration: Simulated conversation where user says "set up a project" → Orchestrator asks name → creates project

### Estimated Effort
Small. ~100 lines of instruction text. No new Rust logic — relies on existing tools. 1 session.

### Open Questions for Jesse
- Should the Orchestrator auto-detect git remote, or always ask? Auto-detect is smoother but may be wrong for monorepos.

---

## 7. PR 2E — Roadmap Decomposition

### Goal
User says "help me build X" → Orchestrator generates a roadmap → decomposes into goal cards in dependency order → places on Kanban → executes sequentially with approval gateways.

### LLM Call Shape

**Single structured-output call** to the Orchestrator's provider:

```
System: "You are a project planner. Given a high-level objective,
decompose it into discrete goals that can each be completed by a
single coding agent in one session. Output valid JSON."

User: "Objective: {user_description}
Project: {project_name}
Root path: {root_path}
Existing board state: {board_summary}

Decompose this into 3-8 goals. Each goal should be:
- Completable in a single agent session (< 30 min of work)
- Have clear acceptance criteria
- Declare dependencies on other goals by index

Output JSON:
{
  \"goals\": [
    {
      \"title\": \"...\",
      \"description\": \"...\",
      \"acceptance_criteria\": [\"...\"],
      \"tags\": [\"code_edit\", ...],
      \"depends_on\": []  // indices of prerequisite goals
    }
  ]
}"
```

**Validation logic** (in Rust, after parsing the LLM response):
1. Parse as JSON, validate against expected schema
2. Verify `depends_on` indices are valid (no cycles, no self-references)
3. Topological sort to establish execution order
4. Reject if > 15 goals (too granular) or < 2 (not decomposed)

### User Approval Gateway

**Critical:** The user sees the proposed roadmap BEFORE any goal cards are created.

Flow:
```
User: "Help me build a REST API for user management"
  → Orchestrator calls LLM for decomposition
  → Orchestrator presents roadmap as formatted text:

    "Here's my proposed roadmap for the User Management API:

    1. Database schema + migrations
       Deps: none | Tags: code_edit, shell
    2. User model + CRUD endpoints
       Deps: #1 | Tags: code_edit
    3. Authentication middleware (JWT)
       Deps: #2 | Tags: code_edit
    4. Input validation + error handling
       Deps: #2 | Tags: code_edit
    5. Integration tests
       Deps: #3, #4 | Tags: code_edit, shell

    Shall I create these as goals and start executing? You can also
    ask me to add, remove, or modify goals before I begin."

  → User: "Looks good, go ahead" (or modifies)
  → Orchestrator creates goal cards in dependency order
  → Orchestrator begins dispatching from roots (no dependencies)
```

### Dependency Execution — LOCKED (Sequential Auto-Dispatch with Pause Override)

After roadmap approval, goals execute in topological dependency order **without per-goal user check-ins**. Each completed goal still goes through Review (per the locked Review-always-required decision in Q2).

Execution flow:
1. Create all goal cards in Triage
2. Move root goals (no dependencies) to Ready
3. Dispatch root goals (moves to InProgress)
4. On completion + Review approval: check if any dependent goals now have all deps Complete
5. Move newly-unblocked goals to Ready and dispatch

The `depends_on` field in `metadata_json` tracks card IDs (not indices — indices are converted to card IDs at creation time).

**Pause/Resume — new MCP tools (on Orchestrator extension):**

**`pause_roadmap`** — Stop auto-dispatching the next goal after the current one completes.
```
Parameters: { project_id_or_slug: String }
```
Sets a project-level flag (`metadata_json.roadmap_paused: true` on the project record, or a separate in-memory flag on the Orchestrator). The currently-running goal continues to completion; the next goal in the chain is NOT auto-dispatched.

**`resume_roadmap`** — Re-enable auto-dispatch and immediately dispatch any Ready goals.
```
Parameters: { project_id_or_slug: String }
```

### Tests
- Unit: JSON schema validation for roadmap output
- Unit: Topological sort with cycles detection
- Unit: Dependency resolution (unblocking logic)
- Unit: Pause prevents next dispatch; resume triggers it
- Integration: Full roadmap → approval → goal creation → sequential dispatch
- Edge case: User rejects roadmap, modifies, re-approves
- Edge case: Pause mid-chain, resume after manual intervention

### Estimated Effort
Medium-large. ~500 lines new Rust (LLM call, validation, dependency engine, approval flow, pause/resume). ~250 lines tests. 2–3 sessions. This is the hardest sub-PR.

### Open Questions for Jesse
- Should the roadmap LLM call use the same model as the Orchestrator, or a potentially cheaper/faster model for planning?
- Max goals per roadmap? Proposing 15 as hard limit, 3–8 as guidance in the prompt.

---

## 8. Total Effort Estimate

| Sub-PR | New Lines (est.) | Sessions (est.) |
|--------|------------------|-----------------|
| 2A — Worker registry + capability detection | ~350 | 1–2 |
| 2B — Goal lifecycle + Orchestrator dispatch | ~800 | 2–3 |
| 2C — Kanban-aware chat surface | ~200 | 1 |
| 2D — Project setup via chat | ~100 | 1 |
| 2E — Roadmap decomposition + pause/resume | ~750 | 2–3 |
| **Integration + edge cases** | ~200 | 1 |
| **Total** | **~2400** | **8–11** |

---

## 9. Recommended Ship Order

The locked order is confirmed:

```
2A  →  2B  →  2C ─┐
                   ├→  2E
            2D  ───┘
```

1. **2A first** — foundation. Everything else needs the worker registry.
2. **2B second** — the core lifecycle. Needs 2A for worker selection.
3. **2C and 2D in parallel** after 2B — both are surface-level additions that don't depend on each other. 2C adds ambient context; 2D adds project setup flow.
4. **2E last** — roadmap decomposition depends on 2B (goal lifecycle) being solid, and benefits from 2C (ambient context) and 2D (project setup) being available.

No dependency changes from the locked order.

---

## 10. Out of Scope

Explicitly excluded from this work:

- **Voice integration** (Epic #198) — separate work, separate epic
- **Mesh / distributed inference** — the Forum concept where Orchestrators meet other Orchestrators. Remote worker dispatch is a Mesh concern.
- **FK propagation to memories/entities/tasks/skills** (Epic #70) — orthogonal data model work
- **Lab View visual rendering of workers** — the 3D World View (`ui/command-center/src/components/world/`) is orthogonal. Workers being dispatchable doesn't require them to be rendered in 3D.
- **Social post card lifecycle** — `card_type='social_post'` exists in schema but its lifecycle is separate from goal lifecycle
- **`created_by` CHECK constraint cleanup** — removing `'hermes'` and adding `'orchestrator'` is Phase 1.5, bundled into the next schema migration (v9), not a standalone PR
- **HUD updates** — HenryHUD and LibrarianHUD (`ui/command-center/src/components/world/HenryHUD.tsx`) are not modified by this work. Goal lifecycle state could feed HUD data in a future PR.
