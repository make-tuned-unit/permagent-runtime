# Prime × Permagent seam inventory

**Date:** 2026-08-20
**Status:** implementation roadmap for the Prime Agent DAG
**Source goals:** `docs/planning/prime_integration_roadmap_goals.json`

Prime Agent’s six primitives map onto existing Permagent seams. This table is
the inventory (goal 0). Feature work lives in the numbered follow-on goals.

| Prime concept | Current Permagent file/API | Gap | Proposed goal id |
| --- | --- | --- | --- |
| RLM kernel (persistent eval context across turns) | `crates/goose/src/rlm.rs` over the `rlm_context` table in `permagent.db` (`spectral_schema.rs:1807` `apply_rlm_context_schema`) | **done** — transactional, versioned, TTL'd get/set/list/delete that survives a daemon restart; tools `context_set/get/list/delete` (`orchestrator.rs:3439`); brief injection via `dispatch_brief.rs:51` | 3, 4 |
| Async subagents | `crates/goose/src/agents/subagent_handler.rs` `run_subagent_task` (awaited); `goal_engine::InternalSubagentEngine` returns a `JoinHandle` for *goal* workers, not generic subagent work | **partial** — goal engines spawn, but review/audit helpers cannot fan out two in-process subagents without blocking | 1 (spawn+join API), 2 (parallel review fan-out) |
| Executable skills | `skill_md.rs` + `platform_extensions/skills.rs` load **markdown** `SKILL.md` folders (agentskills.io). No runner that execs a package and returns structured stdout | **missing** — skills are prompts, not runnable artifacts | 5 (package + runner), 6 (`run_executable_skill` tool) |
| Goal threading (resume with prior attempt context) | `resume_in_progress_goals` / `resume_single_goal` (`orchestrator.rs`); `requeue_goal` preserves `attempt_count` + `last_error`; dispatch brief does **not** re-inject them (W4/W5) | **partial** — metadata survives; the next worker starts cold; dead-session resume can still fabricate Review success if a re-attached session goes idle | 7 |
| Bounded refinement | `goal_transition::goal_budget` attempt/token/wallclock caps; verify-loop escalation (`cost_router`); completion checks run in `goose-server` verification *after* Review | **partial** — caps park the goal; there is no distinct check-failure rework budget that auto-requeues with check stdout | 8 |
| A2A messaging | `send_message` (session id) and `steer_goal` (live CLI worker). No goal-to-goal API; Complete/Cancelled are not explicitly refused as A2A targets | **partial** — the pipes exist; they are not addressed by goal id, not audited as A2A, and do not write through to RLM | 9 (deliver+refuse), 10 (feedback → RLM → next brief) |

Related existing spine (not a Prime gap, but the DAG this inventory rides on):

- `create_roadmap` / `trigger_roadmap_dispatch` / `spawn_dag_driver_loop` in `orchestrator.rs`
- Goal state machine: `goal_state.rs` + `goal_transition.rs`
- Decision Inbox: `approve_review`, `unblock`

## Shipped

Landed by the Prime DAG implementation (goals 0–11).

**Correction (2026-08-25):** two entries below were first landed as in-process
prototypes and listed here as shipped before they were durable. The RLM row is
now accurate as written. The executable-skills row is **still a prototype** —
see its note.

- **RLM control plane** — `crates/goose/src/rlm.rs`. Durable: every cell is a
  versioned row in `permagent.db`'s `rlm_context` table, so state survives a
  daemon restart, and the in-process map is only a read-through cache. Writes
  take an optional `expected_version` and refuse on conflict rather than
  overwriting; credential-shaped values are refused outright (`credential_shape`);
  expired cells are swept on the daemon's WAL-checkpoint tick
  (`wal_checkpoint.rs:67`). Each version change mirrors a summary into the Brain
  (`state.rs:413`) so goal state is also recallable — the Brain is never the read
  path. Re-dispatch briefs quote recovered state as data-not-instructions.
- **Async subagent spawn** — `spawn_subagent_task` / `spawn_subagent_work` in `subagent_handler.rs`. Two handles can be outstanding before either join.
- **Parallel review fan-out** — `review_fanout` module; opt-in via goal or project metadata `review_fanout: true`. Security + debugger briefs fold into `approve_review` detail.
- **Executable skills** — *prototype → being replaced.* `crates/goose/src/executable_skills.rs`
  plus `skills/examples/hello-json/`. Orchestrator tool `run_executable_skill`
  refuses paths outside the skills root, but reaches `tokio::process::Command`
  with **no approval gate**, no manifest inputs schema, no verify contract, no
  registry row and no receipt. Replaced by `skill_run` in the executable-skills
  PR; see `PRIME_RLM_AND_SKILLS.md`.
- **Goal threading** — dispatch briefs on `attempt_count > 0` include `last_error`, RLM snapshot, A2A inbox, and worktree pointer. Resume never promotes a dead session to Review without worktree evidence (W5).
- **Bounded refinement** — `crates/goose/src/goal_refinement.rs`. Metadata `refinement_budget` caps auto-rework after completion-check failure; exhaustion parks with `unblock`.
- **A2A** — orchestrator tool `message_goal` (`from_goal`, `to_goal`, `body`).
  InProgress only; steers a live worker when one exists. The control-plane write
  goes through `rlm::write_a2a_feedback` (`rlm.rs:636`), a bounded, version-checked
  ring on `rlm_context`. It replaced `persist_rlm_snapshot`, which read-modify-wrote
  the whole `cards.metadata_json` blob with no version guard and so silently lost
  any concurrent write to `attempt_count`, `last_error` or `worktree_path`.
- **E2E smoke** — `trigger_roadmap_dispatch` 2-goal promote path (lib test) plus this Shipped section.

### Live smoke (optional)

Against a running `permagentd` with this build:

1. Rebuild/reinstall `/Applications/Permagent.app` so the daemon actually loads these seams.
2. Ask Henry: create the roadmap from `docs/planning/prime_integration_roadmap_goals.json` via `create_roadmap` (do not re-decompose).
3. Confirm the DAG driver log line `Goal DAG driver loop started` and that root goal 0 dispatches first.
