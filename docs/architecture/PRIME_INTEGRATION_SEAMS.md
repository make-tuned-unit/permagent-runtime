# Prime × Permagent seam inventory

**Date:** 2026-08-20
**Status:** implementation roadmap for the Prime Agent DAG
**Source goals:** `docs/planning/prime_integration_roadmap_goals.json`

Prime Agent’s six primitives map onto existing Permagent seams. This table is
the inventory (goal 0). Feature work lives in the numbered follow-on goals.

| Prime concept | Current Permagent file/API | Gap | Proposed goal id |
| --- | --- | --- | --- |
| RLM kernel (persistent eval context across turns) | none as a control plane; closest is goal metadata + `cost_router::GoalEscalationState` handoff on re-dispatch (`orchestrator.rs` `dispatch_goal_fn`) | **missing** — no session-scoped get/set/list store that outlives a single LLM turn | 3 (seam), 4 (inject into dispatch brief) |
| Async subagents | `agents/platform_extensions/fanout.rs` (`run_bounded`, `subagent_cost`) behind the `delegate_many` tool; `subagent_handler::spawn_subagent_task` / `spawn_subagent_work` for a single handle | **closed** — N children run with a configured cap on how many are in flight (`PERMAGENT_FANOUT_CONCURRENCY`, default 2), each routed on its own through `cost_router::delegate`'s precedence, results joined in request order with per-child ledger cost by `subagent_id`, and a parent cancel reaching every child | 1 (spawn+join API), 2 (parallel review fan-out) |
| Executable skills | `skill_md.rs` + `platform_extensions/skills.rs` load **markdown** `SKILL.md` folders (agentskills.io). No runner that execs a package and returns structured stdout | **missing** — skills are prompts, not runnable artifacts | 5 (package + runner), 6 (`run_executable_skill` tool) |
| Goal threading (resume with prior attempt context) | `resume_in_progress_goals` / `resume_single_goal` (`orchestrator.rs`); `requeue_goal` preserves `attempt_count` + `last_error`; dispatch brief does **not** re-inject them (W4/W5) | **partial** — metadata survives; the next worker starts cold; dead-session resume can still fabricate Review success if a re-attached session goes idle | 7 |
| Bounded refinement | `goal_transition::goal_budget` attempt/token/wallclock caps; verify-loop escalation (`cost_router`); completion checks run in `goose-server` verification *after* Review | **partial** — caps park the goal; there is no distinct check-failure rework budget that auto-requeues with check stdout | 8 |
| A2A messaging | `send_message` (session id) and `steer_goal` (live CLI worker). No goal-to-goal API; Complete/Cancelled are not explicitly refused as A2A targets | **partial** — the pipes exist; they are not addressed by goal id, not audited as A2A, and do not write through to RLM | 9 (deliver+refuse), 10 (feedback → RLM → next brief) |

Related existing spine (not a Prime gap, but the DAG this inventory rides on):

- `create_roadmap` / `trigger_roadmap_dispatch` / `spawn_dag_driver_loop` in `orchestrator.rs`
- Goal state machine: `goal_state.rs` + `goal_transition.rs`
- Decision Inbox: `approve_review`, `unblock`

## Shipped

Landed by the Prime DAG implementation (goals 0–11):

- **RLM control plane** — `crates/goose/src/rlm.rs` (`get` / `set` / `list`, session-keyed). Re-dispatch briefs quote recovered state as data-not-instructions.
- **Async subagent spawn** — `spawn_subagent_task` / `spawn_subagent_work` in `subagent_handler.rs`. Two handles can be outstanding before either join.
- **Bounded fan-out** — `agents/platform_extensions/fanout.rs`, behind the
  orchestrator-side tool `delegate_many`. At most
  `fanout::MAX_FANOUT_CHILDREN` children per call and at most
  `PERMAGENT_FANOUT_CONCURRENCY` (default 2) in flight at once; each child is
  resolved through the same `build_delegate_recipe` → `build_task_config` path a
  single `delegate` takes, so `cost_router::delegate`'s precedence applies per
  child and pins are honoured with no silent escalation. Results join in request
  order, each carrying its own routing receipt, its own `subagent_id`, and the
  spend read back from `cost_ledger` under that id. Every child runs on a token
  derived from the caller's, so cancelling the fan-out cancels the children and a
  child still queued never starts.
- **Parallel review fan-out** — `review_fanout` module; opt-in via goal or project metadata `review_fanout: true`. Security + debugger briefs fold into `approve_review` detail.
- **Executable skills** — `crates/goose/src/executable_skills.rs` plus `skills/examples/hello-json/`. Orchestrator tool `run_executable_skill` refuses paths outside the skills root.
- **Goal threading** — dispatch briefs on `attempt_count > 0` include `last_error`, RLM snapshot, A2A inbox, and worktree pointer. Resume never promotes a dead session to Review without worktree evidence (W5).
- **Bounded refinement** — `crates/goose/src/goal_refinement.rs`. Metadata `refinement_budget` caps auto-rework after completion-check failure; exhaustion parks with `unblock`.
- **A2A** — orchestrator tool `message_goal` (`from_goal`, `to_goal`, `body`). InProgress only; writes RLM + card metadata; steers a live worker when one exists.
- **E2E smoke** — `trigger_roadmap_dispatch` 2-goal promote path (lib test) plus this Shipped section.

### Live smoke (optional)

Against a running `permagentd` with this build:

1. Rebuild/reinstall `/Applications/Permagent.app` so the daemon actually loads these seams.
2. Ask Henry: create the roadmap from `docs/planning/prime_integration_roadmap_goals.json` via `create_roadmap` (do not re-decompose).
3. Confirm the DAG driver log line `Goal DAG driver loop started` and that root goal 0 dispatches first.
