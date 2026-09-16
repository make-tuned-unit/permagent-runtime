# Lane R57a — a paused roadmap is resumed, never dispatched goal by goal

Fixture: `select-existing-roadmap-resumes-not-redecomposes`
(`/Users/j/Documents/dev/permagent-worktrees/laneM17/tasks-m41/select-existing-roadmap-resumes-not-redecomposes/task.yaml`),
`selection_check.must_call: [resume_roadmap]`. Failing in 3 of the last 10
headless DeepSeek records.

## Root cause

The `roadmap_paused` tag was a rule about *automatic* dispatch only. Every
automatic trigger asks one arbiter which goals may run —
`eligible_ready_goals`
(`crates/goose/src/agents/platform_extensions/orchestrator.rs:12176-12198`,
pre-change numbering), which returns an empty list for a tagged project:

- the after-turn/approval nudge — `dispatch_ready_goals_with` →
  `eligible_ready_goals` (`orchestrator.rs:12783`), already refused;
- the headless drain's Ready sweep — `redispatch_ready_goals` →
  `eligible_ready_goals` → `run_scoped_ready_goals`
  (`orchestrator.rs:16270-16320`), already refused;
- `resume_roadmap` → `dispatch_eligible_goals` (`orchestrator.rs:12202`),
  which is the intended way in and removes the tag first.

Nothing enforced it on the path the coordinator itself uses.
`handle_goal_advance`'s `GoalAction::Dispatch` arm
(`orchestrator.rs:8949` / dispatch arm at `orchestrator.rs:9085`) addresses
ONE card by id, loads that card, validates the transition, and calls
`OrchestratorClient::dispatch_goal_to` (`orchestrator.rs:4304`) → the free
`dispatch_goal_fn` (`orchestrator.rs:6262`). It never asks the card's
project a question, so the pause tag was invisible to it.

Observed failure (`.../dschat-r59/q2-runs/dschat-select-existing-roadmap-resumes-not-redecomposes-1789600765150211000-11/logs/harness.log`):
`goal_list → check_worker → goal_status → routing_roster → goal_advance` —
the seeded project's single Ready goal ("Add a retry policy to the client")
was dispatched directly, ran and landed, and the roadmap was never resumed:
`roadmap_paused` stayed on the project, every other goal behind it stayed
frozen, and `must_call: resume_roadmap` failed. A passing record
(`q1-runs/...-11/logs/harness.log`) is `goal_list → check_worker →
resume_roadmap → goal_status` — the same information, a different tool
choice. The recipe's exception (a) offered `goal_advance` *or*
`resume_roadmap` "as the state calls for" without ever saying which state
calls for which, so the difference was a coin flip the runtime did not
settle.

## What changed

1. **`crates/goose/src/agents/platform_extensions/orchestrator.rs`**
   - New free fn `refuse_dispatch_of_a_paused_roadmap(pool, card)`, placed
     immediately after `eligible_ready_goals` so the pause rule stays
     defined in one region. `Ok(())` for an unpaused project (one indexed
     tag read via `projects::get_project`); otherwise a refusal:
     `"roadmap <slug> is paused; call resume_roadmap to continue it — goals
     of a paused roadmap are not dispatched one by one. resume_roadmap
     clears the pause and dispatches every eligible Ready goal itself,
     including '<title>' (<card id>); called from this project's own
     directory it takes no arguments. Goal not dispatched."`
     Machine detail (goal id, project id, slug) goes to `tracing::info!` on
     `permagentd::brain`, not into the message.
   - Called from **`handle_goal_advance`'s `GoalAction::Dispatch` arm**, as
     its first statement — ahead of R22a's
     `ensure_worker_checked_for_dispatch`, so a refused dispatch spends no
     worker probe and prints no synthetic `▸ check_worker` banner into the
     transcript for a dispatch that was never going to happen.
   - Called from **`OrchestratorClient::dispatch_goal_to`** as the
     structural backstop: every OrchestratorClient dispatch entry funnels
     through it (`goal_advance{dispatch}`, `handle_goal_status`'s
     dead-worker auto-retry at `orchestrator.rs:9927`, and
     `dispatch_eligible_goals` → `dispatch_goal`). `resume_roadmap` removes
     the tag *before* calling `dispatch_eligible_goals`, so the resolution
     path is unaffected.
   - Refusal shape follows the house style (`write_scope::metadata_conflict`
     at `orchestrator.rs:6187`, R22a's "worker was checked and is not
     available"): a plain `Err(String)` out of the tool handler. That is the
     same path `permagent-eval`'s `harness_log` already scans, so
     `refused_tool_calls` receipts stay honest — **no new receipt line was
     added**.
   - Verified the refusal has a real resolution path: `handle_resume_roadmap`
     (`orchestrator.rs:10813`) removes `roadmap_paused`
     (`projects::remove_tag`) and then dispatches every eligible Ready goal
     itself through `dispatch_eligible_goals`, records the
     `dag_enforcement/roadmap_resumed` count, and resolves its project by
     cwd when called with no arguments (`resolve_resume_project`,
     `orchestrator.rs:660`) — which is exactly how the fixture's seeded
     project is found.

2. **`crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`** — one
   sentence inside exception (a), same voice and structure as its
   neighbours: "When the roadmap is PAUSED, `resume_roadmap` is that call —
   it clears the pause and dispatches the Ready goals itself; never dispatch
   a paused roadmap's goals one by one with `goal_advance`." Plus the usual
   lane line in the header comment block.

3. **Tests** — three new `#[tokio::test]`s in `orchestrator.rs`'s `mod
   tests`, next to `nudge_respects_a_paused_roadmap` /
   `dispatch_eligible_skips_when_paused`, reusing `setup_goal_in_state` and
   the `tempfile` + `SessionManager` + `OrchestratorClient` fixture the
   neighbouring dispatch tests use:
   - `goal_advance_dispatch_refuses_a_paused_roadmaps_ready_goal_and_names_resume_roadmap`
     — the fixture's own shape through the real tool handler; asserts the
     refusal says "is paused", names `resume_roadmap`, says "not dispatched
     one by one", and that the card stayed in Ready with no `worker_key`.
   - `resume_roadmap_clears_the_pause_so_the_same_dispatch_path_proceeds`
     — refused, then `handle_resume_roadmap` clears the tag, then the same
     `dispatch_goal_to` call gets past the pause gate and fails only on the
     pre-existing, unrelated "no working directory is known" refusal.
     `#[serial_test::serial(receipts_global_registry)]` like its
     `handle_resume_roadmap_*` neighbours (it reaches
     `receipts::record_count`).
   - `dispatch_of_an_unpaused_projects_goal_is_untouched_by_the_pause_gate`
     — applicability.
   Tests 2 and 3 pin the worker (`Some("permagent")`) so ranked selection
   never probes an external CLI, the machine-independence technique
   `dispatch_goal_to_and_resume_roadmap_consult_the_installed_headless_policy`
   already uses. No process-global `OnceLock` is set by any of them.

4. **`crates/goose-cli/src/recipes/search_recipe.rs`** — added the inline
   expectation for the new recipe sentence to the existing
   `builtin_permagent_coding_recipe_resolves_and_parses` test (it already
   pins other instruction text this way).

## Gate filters (coordinator runs these — this lane ran no cargo)

```
cargo test -p permagent --lib roadmap_paused
cargo test -p permagent --lib paused_roadmap
cargo test -p permagent --lib -- goal_advance_dispatch_refuses_a_paused_roadmaps_ready_goal_and_names_resume_roadmap resume_roadmap_clears_the_pause_so_the_same_dispatch_path_proceeds dispatch_of_an_unpaused_projects_goal_is_untouched_by_the_pause_gate
cargo test -p permagent --lib resume_roadmap
cargo test -p permagent --lib dispatch_goal_to
cargo test -p permagent --lib nudge_respects_a_paused_roadmap
cargo test -p permagent-cli --lib builtin_permagent_coding_recipe_resolves_and_parses
cargo test -p permagent-cli --lib recipes::
```

Broader regression sweep for the touched dispatch seam:

```
cargo test -p permagent --lib dispatch_goal
cargo test -p permagent --lib goal_advance
```

`cargo fmt --check` was pre-verified out of band with `rustfmt --edition
2021 --check` on both edited Rust files (clean); no `cargo` was run.

## Snapshot files

**None need regeneration.** Checked explicitly:

- `crates/goose/src/agents/snapshots/permagent__agents__prompt_manager__tests__{basic,one_extension,typical_setup,all_platform_extensions}.snap`
  contain the orchestrator *extension instructions*, not per-tool
  descriptions and not the recipe — no tool description or extension
  instruction string was touched, so these are unaffected.
- `NON-NEGOTIABLE EXECUTION PROTOCOL` appears in exactly one file in the
  tree (`permagent-coding.yaml`); no `.snap` anywhere embeds the recipe
  instructions, and `crates/goose-cli` has no `.snap` files at all.

If CI disagrees, the only candidates are the four prompt-manager snapshots
above (`cargo insta` / `INSTA_UPDATE=always cargo test -p permagent --lib
prompt_manager`).

## Risks / residuals

- **Behaviour change on an existing path**: `handle_goal_status`'s
  dead-worker auto-retry (`orchestrator.rs:9927`) now also refuses while the
  roadmap is paused, because it goes through `dispatch_goal_to`. That is
  consistent with `pause_roadmap`'s own contract ("no NEW goals will be
  auto-dispatched"); a genuinely running worker is untouched. Called out so
  it is a decision, not a surprise.
- **Deliberately NOT guarded**: `project_manager`'s `card_create` with
  `auto_dispatch` (`project_manager.rs:1973`) calls the free
  `dispatch_goal_fn` directly. That is a UI path creating a brand-new card
  rather than continuing an existing roadmap, so it was left alone to keep
  the diff surgical. Moving the check into `dispatch_goal_fn` would cover it
  (and every hook caller) at the cost of one tag read per dispatch — a
  one-line follow-up if the coordinator wants it.
- **Cost**: `dispatch_goal_to` now does two extra reads per dispatch (card +
  project/tags). Negligible next to spawning a worker, but it is a real
  addition on the hot path; `handle_goal_advance` pays it twice (early
  check, then the backstop). Kept for the transcript-honesty reason above.
- `after_turn::RoadmapDispatchGuard`'s nudge text still says "call
  goal_advance dispatch or explain". It only fires for goal cards created by
  *this* session's own `create_roadmap`, so it cannot see the fixture's
  seeded roadmap; but a session that creates a roadmap and then pauses it in
  the same turn would now be nudged toward a call that refuses. Exotic, not
  fixed here, listed so it is on the record.
- The refusal text is matched by substring in the new tests
  (`"is paused"`, `"resume_roadmap"`, `"not dispatched one by one"`) —
  rewording the message means updating those three assertions.
