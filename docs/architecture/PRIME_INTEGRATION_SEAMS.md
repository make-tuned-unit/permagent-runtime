# Prime × Permagent seam inventory

**Date:** 2026-08-20
**Status:** implementation roadmap for the Prime Agent DAG
**Source goals:** `docs/planning/prime_integration_roadmap_goals.json`

Prime Agent’s six primitives map onto existing Permagent seams. This table is
the inventory (goal 0). Feature work lives in the numbered follow-on goals.

| Prime concept | Current Permagent file/API | Gap | Proposed goal id |
| --- | --- | --- | --- |
| RLM kernel (persistent eval context across turns) | none as a control plane; closest is goal metadata + `cost_router::GoalEscalationState` handoff on re-dispatch (`orchestrator.rs` `dispatch_goal_fn`) | **missing** — no session-scoped get/set/list store that outlives a single LLM turn | 3 (seam), 4 (inject into dispatch brief) |
| Async subagents | `crates/goose/src/agents/subagent_handler.rs` `run_subagent_task` (awaited); `goal_engine::InternalSubagentEngine` returns a `JoinHandle` for *goal* workers, not generic subagent work | **partial** — goal engines spawn, but review/audit helpers cannot fan out two in-process subagents without blocking | 1 (spawn+join API), 2 (parallel review fan-out) |
| Executable skills | `skill_md.rs` + `platform_extensions/skills.rs` load **markdown** `SKILL.md` folders (agentskills.io). No runner that execs a package and returns structured stdout | **missing** — skills are prompts, not runnable artifacts | 5 (package + runner), 6 (`run_executable_skill` tool) |
| Goal threading (resume with prior attempt context) | `resume_in_progress_goals` / `resume_single_goal` (`orchestrator.rs`); `requeue_goal` preserves `attempt_count` + `last_error`; dispatch brief does **not** re-inject them (W4/W5) | **partial** — metadata survives; the next worker starts cold; dead-session resume can still fabricate Review success if a re-attached session goes idle | 7 |
| Bounded refinement | `goal_transition::goal_budget` attempt/token/wallclock caps; verify-loop escalation (`cost_router`); completion checks run in `goose-server` verification *after* Review | **partial** — caps park the goal; there is no distinct check-failure rework budget that auto-requeues with check stdout | 8 |
| A2A messaging | `agents/platform_extensions/goal_a2a.rs` — `message_goal` addressed by goal id, resolving card → live state → live worker; typed [`A2aRefusal`]; `events::a2a_message` → activity journal | **closed** — goal-id addressed, Complete/Cancelled refused explicitly as PERMANENT (distinct from a not-yet-running target), every delivery audited on the timeline by sender/recipient/length/body-hash and never by body text, and written through to RLM | 9 (deliver+refuse), 10 (feedback → RLM → next brief) |

Related existing spine (not a Prime gap, but the DAG this inventory rides on):

- `create_roadmap` / `trigger_roadmap_dispatch` / `spawn_dag_driver_loop` in `orchestrator.rs`
- Goal state machine: `goal_state.rs` + `goal_transition.rs`
- Decision Inbox: `approve_review`, `unblock`

## Shipped

Landed by the Prime DAG implementation (goals 0–11):

- **RLM control plane** — `crates/goose/src/rlm.rs` (`get` / `set` / `list`, session-keyed). Re-dispatch briefs quote recovered state as data-not-instructions.
- **Async subagent spawn** — `spawn_subagent_task` / `spawn_subagent_work` in `subagent_handler.rs`. Two handles can be outstanding before either join.
- **Parallel review fan-out** — `review_fanout` module; opt-in via goal or project metadata `review_fanout: true`. Security + debugger briefs fold into `approve_review` detail.
- **Executable skills** — `crates/goose/src/executable_skills.rs` plus `skills/examples/hello-json/`. Orchestrator tool `run_executable_skill` refuses paths outside the skills root.
- **Goal threading** — dispatch briefs on `attempt_count > 0` include `last_error`, RLM snapshot, A2A inbox, and worktree pointer. Resume never promotes a dead session to Review without worktree evidence (W5).
- **Bounded refinement** — `crates/goose/src/goal_refinement.rs`. Metadata `refinement_budget` caps auto-rework after completion-check failure; exhaustion parks with `unblock`.
- **A2A** — orchestrator tool `message_goal` (`from_goal`, `to_goal`, `body`),
  addressed by GOAL ID: the id resolves to the card, its live state, and the
  live worker steering it. InProgress only; writes RLM + card metadata; steers a
  live worker when one exists. Refusals are typed (`goal_a2a::A2aRefusal`) and
  say which kind of no they are — a Complete or Cancelled target is
  `Terminal` and permanent (`is_permanent()`), a Triage/Ready/Review/Failed one
  is `NotRunning` and retryable. Every delivery emits `a2a_message`, which the
  durable activity journal records with the sender, the recipient, the body's
  length and its SHA-256 — and never the body: an audit trail for instructions
  passing between agents must prove the message existed without republishing
  what it said. RLM write-through is the in-memory `rlm::set` + metadata
  snapshot; a `TODO(prime-rlm, #1129)` in `goal_a2a.rs` names
  `rlm::write_a2a_feedback` as the durable replacement.
- **E2E smoke** — `trigger_roadmap_dispatch` 2-goal promote path (lib test) plus this Shipped section.

### Live smoke (optional)

Against a running `permagentd` with this build:

1. Rebuild/reinstall `/Applications/Permagent.app` so the daemon actually loads these seams.
2. Ask Henry: create the roadmap from `docs/planning/prime_integration_roadmap_goals.json` via `create_roadmap` (do not re-decompose).
3. Confirm the DAG driver log line `Goal DAG driver loop started` and that root goal 0 dispatches first.
