# Lane R62g: a blocker is an escalation, never a roadmap card

Branch `lane/R62g` (base `53de25c5`). Code commit `398b56da`. I edited the code only and ran no cargo commands. `rustfmt --edition 2021 --check` passes on the file.

## Evidence (M46 r61 s1)

- **Fixture:** `select-ambiguous-choice-escalates-decision`. It requires `escalate` and forbids `create_roadmap`.
- **Log:** `laneM17/dschat-r61/s1-runs/dschat-select-ambiguous-choice-escalates-decision-1789613394893193000-9/logs/harness.log`
- **What the coordinator did:**
  1. It ran `tree`, which reported "(empty directory)", then `map_query`, which found no code map, and a `shell` call that was denied.
  2. It called `decompose_roadmap` with an objective that contained "BLOCKER TO SURFACE IN THIS PLAN: the workspace is empty … it must not be dispatched until the repo is provided".
  3. It called `create_roadmap` with one goal titled "BLOCKER: Obtain repo source and project structure". A worker was dispatched to it (`roadmap_dispatched=1`) and it was parked at exit.
- **Comparison runs:** every r58 and r59 run of this fixture, and of `select-no-repo-facts-escalates-not-invents`, called `escalate` without ever calling `decompose_roadmap`. The r61 s1 run is the outlier. The prompt's "pick whichever approach you think is best" pulled the model into rule 1, and it wrote the blocker into the plan.

## Root cause

Nothing on the roadmap path treated a plan that was really a blocker as a refusal:

- **`handle_decompose_roadmap`** (`crates/goose/src/agents/platform_extensions/orchestrator.rs:10254`) gathers `RepoFacts` (`:10316`). It sends the objective to the planner and grounds whatever comes back. The facts only feed the prompt and the acceptance-criteria grounding.
- **`handle_create_roadmap`** (`:10748`) checks goal shape (`validate_roadmap_goals`), the hash and approval, and the acceptance-criteria rewrite (R21b). It never asks whether a goal is work a worker can do.

Recipe exception (b)(3) says that missing repo facts mean `escalate`, but the runtime did not enforce it.

## Change

All of the change is in `orchestrator.rs`. It stays inside the decompose and create validation path; `ground_roadmap_goals` is untouched.

- **New pure detector** (`:1057–1262`): `roadmap_blocker_refusal(objective, goals, facts) -> Option<String>`, with helpers `goal_title_is_blocker_label`, `goal_description_only_obtains_a_prerequisite`, `text_declares_a_blocker` and `refuse_blocker_roadmap` (which writes a tracing line and returns the text). No helper slices strings by byte index: text is normalized to space-padded lowercase words and checked with `contains(" phrase ")`.
- **Refusal text:** `No roadmap for a blocker: <reason>. A missing repo, missing facts, or missing credentials are escalations: call `escalate` with the specific ask; a roadmap card is for work a worker can do here.`
- **When it fires:**
  - **(a)** `RepoFacts::gather` finds `file_count == 0` (the same probe grounding uses, with no separate disk scan) **and** the objective or a goal says the plan is blocked. The trigger is the word "blocker", or the phrases "blocked on", "no repo facts", "must not be dispatched", or "until the repo/repository/source is".
  - **(b)** Any goal title uses the BLOCKER label form:
    - all-caps `BLOCKER`, or
    - "blocker" as the first word, followed by a separator (`Blocker:`, `[Blocker]`, `**Blocker** —`, `Blocker - …`).

    Leading markup is trimmed first.
  - **(b)** Any goal description contains all three of:
    - an acquiring verb (obtain, acquire, get or request access to, ask the user for)
    - a missing prerequisite (source, repo, codebase, project structure, credentials, api key, repo facts)
    - a "no work until then" gate (prerequisite blocker, is a blocker, work can proceed/start/begin, before any work, before work can, must not be dispatched)
- **Where it is called:**
  - `decompose_roadmap`, before the planner call, on the objective only (`:10318`). The r61 shape is refused here, which also saves the Fable planner call.
  - `decompose_roadmap`, on the raw planner proposal before grounding, the approval decision or the proposal proof (`:10526`).
  - `create_roadmap`, on the final goals after the headless canonical swap and before `resolve_create_roadmap_proposal_hash` (`:10805`). To make this possible, `create_roadmap`'s `facts_dir`/`facts` gathering moved up from just before `validate_roadmap_ac_or_rewrite`. It is the same expression and is still reused there.
- **Refused-tool path:** the refusal is a plain handler `Err` plus a `tracing::info!` line, the same path `refuse_dispatch_of_a_paused_roadmap` uses. No receipt keys were added.

### Deliberate narrowing of (a)

The brief asked for "refuse when the grounding facts show no source files at all". I did not implement that literally, because it would break greenfield work:

- **The `fizzbuzz` fixture:** its workspace is empty while the agent runs, because `oracle/` is copied in afterwards, and it must create `fizzbuzz.py` through a roadmap.
- **Recipe Lane R40b (`e36fe589`):** it names "an empty seeded workspace" as NOT a blocker ("create what's missing").

So (a) fires only when the facts are empty **and** the plan itself says it is blocked. That still catches r61 s1 at decompose time, and (b) catches the card whatever the facts say.

## Resolution path (no nudge loop that forbids escalate)

The after-turn guards are all in `crates/goose/src/after_turn.rs`.

**`NoRoadmapGuard`** (`:1939`) returns Allow once `decompose_roadmap` or `create_roadmap` has been attempted, even if the attempt was refused. It never pushes a model toward decompose after this refusal.

**`RoadmapDispatchGuard`** (`:1656`, `undecomposed_roadmap_for` at `:1545`):
- A decompose-time refusal comes before `create_decision`, so no approved proposal exists and this guard does nothing.
- A create-time refusal after a successful auto-approved decompose triggers the guard's nudge. The nudge says "call create_roadmap … **or escalate**", is bounded by `MAX_HOLDS`, and ends in a Park.
- An `escalate` call anywhere in the session skips the guard.

**`Rule5HonestFailureGuard`:**
1. `turn_called_escalate` releases the guard at once (`:2824`).
2. After a decompose-time refusal nothing has succeeded (`turn_tool_call_succeeded` is false, `:2845`), so a prose ending counts as `bare_session` (`:2520`).
3. The first strike injects `RULE5_INJECT_TEXT` (`:2327`), which offers `escalate`.
4. The second strike Parks (`:2527`). R24b redirects that Park into a minted escalation (`:2919`), but **only when the final message reads as a question** (a trailing `?` or a `RULE5_QUESTION_PHRASES` hit). A prose ending that does not read as a question still parks honestly and is not minted.
5. R58a's `decide_undispatched_ready_goal` (`:2567`, trigger `:2887`) needs an undispatched Ready root card (`:2405`). A refused roadmap creates no cards, so it cannot fire.

Caveats:
- After a **create-time** refusal that follows a successful decompose, `turn_tool_call_succeeded(ROADMAP_TOOL_NAMES)` is true, so the Rule5 guard returns Allow and does not mint. Only the `RoadmapDispatchGuard` nudge and Park apply. The realistic r61 path is refused at decompose time, so this matters little.
- No guard treats `escalate` as forbidden, and `CodingDagInspector` does not gate it.

## Tests (orchestrator `mod tests`, `:42622–42950`)

**Pure detector tests:**
- `r62g_blocker_title_label_forms_match_and_ordinary_titles_do_not`
- `r62g_prerequisite_only_description_matches_but_runtime_token_work_does_not`
- `r62g_refusal_names_escalate_for_each_blocker_shape`: covers the r61 card in a real repo, a disguised prerequisite-only description, and the r61 objective with empty facts.
- `r62g_legitimate_plans_are_not_refused`: covers greenfield in an empty directory, tests-only and docs-only repos whose objective says "blocker", and "Remove blocker flag from config".

**Handler tests** (built with `roadmap_handler_fixture` and `decompose_then_read_proof`):
- `r62g_decompose_in_an_empty_workspace_refuses_a_declared_blocker`: the personal project's root points at an empty tempdir. Decompose is refused, the error names `escalate`, and no `roadmap_proposal` proof is left behind.
- `r62g_decompose_in_an_empty_workspace_keeps_a_greenfield_plan`: the fizzbuzz-shaped objective in an empty root still decomposes.
- `r62g_create_roadmap_refuses_a_blocker_titled_goal_in_a_real_repo`: the directory holds `src/lib.rs`, and the call uses the correct hash for the blocker goals. It is refused and creates no goal cards.
- `r62g_create_roadmap_accepts_a_normal_real_repo_proposal`: `#[serial_test::serial(receipts_global_registry)]`. A normal decompose then create succeeds.
- `r62g_create_roadmap_lets_a_remove_blocker_flag_goal_through`: the goal passes the blocker check and reaches the ordinary hash check, whose error names the expected hash.

These tests set no process-global OnceLocks.

## Gate filters

```
cargo test -p permagent --lib r62g_
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::handle_decompose_then_create_roadmap
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::ground_roadmap_goals
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::identical_goals_json_is_accepted_despite_a_wrong_proposal_hash
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::goals_json_that_differs_from_the_pending_proposal_still_requires_the_correct_hash
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::headless_third_consecutive_mismatch_auto_approves_the_pending_proposal
cargo test -p permagent --lib agents::platform_extensions::orchestrator::tests::canonical_pending_goals_are_bound_to_hash_and_decision
```

Plus `cargo clippy -p permagent --lib --tests -- -D warnings` for the string_slice lint.

## Risks

- **Compile not verified in this lane.** Spots worth a look:
  - the `Peekable` peek-then-`find` in `goal_title_is_blocker_label`
  - `tracing::info!` with the `tool` shorthand field (`&str`)
  - `Option<&str>::is_some_and(text_declares_a_blocker)`
- **Possible false positives in a real repo:**
  - A goal a human deliberately titled "BLOCKER: fix crash on start". Priority-label titles are rare in planner output, but they would be refused.
  - A description that combines "obtain" with "credential" or "repo" and a gate phrase such as "is a blocker".
  - With empty facts only: an objective that uses the word "blocker" for a greenfield task.
- **Eval signal:** a refused `create_roadmap` still shows as an attempted call in a non-`--debug` `harness.log`. Lane K9's Mechanism A markers do not include this handler error, and `create_roadmap` is not in Mechanism B's list, so the evaluator would still count the call as executed. The fix avoids this for r61 by refusing at `decompose_roadmap`, which the fixture does not forbid, so no proposal hash ever exists for `create_roadmap`.
- **Merge overlap:** `handle_create_roadmap` moves 11 lines (the facts gather), which may conflict textually with R62e or R62cd if they touch the same block. The fix is to keep a single `facts_dir`/`facts` binding above the proposal-hash check.
