# Lane R62e report (with R52c, R59b, R62f)

Branch `lane/R62e` from 53de25c5, worktree `permagent-worktrees/laneR62e`. Edit-only: nothing was compiled or run. `rustfmt --edition 2021 --check` is clean on every touched `.rs` file.

| Lane | Commit | Files |
|---|---|---|
| R62e | 664b3f54 | orchestrator.rs, decisions_effects.rs, cost_router/call_site_completeness.rs |
| R52c | 0abe1140 | orchestrator.rs |
| R59b | 365ca68e | recipes/builtin/permagent-coding.yaml, recipes/search_recipe.rs |
| R62f | 82f77f02 | after_turn.rs, agents/agent.rs, agents/reply_parts.rs |

Line references below are to 53de25c5.

## R62e: salvage and the fresh re-verify re-run the worker's proven check

### Root causes
- **Only auto-detection was ever re-run.** `run_fresh_verify_for_hold` (orchestrator.rs:19630) and `maybe_salvage_committed_work` (orchestrator.rs:19772, its `VerifyParams` block before the verify call) both built `VerifyParams { command: None }`.
  - A61 miss 1 (s2): the worker's genuine PASS was an explicit `node --experimental-strip-types …`. Salvage re-detected "package.json defines no build/test script" (verdict `error`) and handed the work to the reviewer unverified.
  - A61 miss 4 (s4): the hold re-detected `python3 tests/test_cli.py` and got ENVIRONMENT, although the worker had passed `/usr/bin/python3 -m pytest -q`.
- **The salvage reviewer saw only the diff.** `call_reviewer_provider` (orchestrator.rs ~20703) built `build_review_prompt(task_spec, diff, note, &[])`. On a call-site-only diff, haiku answered UNCERTAIN: "cannot verify quoteShipping signature accepts three parameters". The main review path already adds E7 call sites (goose-server `verification/mod.rs` `call_site_enumerations_for_review` → `build_review_prompt_with_call_sites` → `gate_decision_with_call_sites`). Salvage did not.

### Changes
- **Recorded-command lookup.**
  - `last_genuine_verify_command_in_messages` indexes every verify request's `command` and `path` arguments by id first, then takes the request behind the LAST response that passes `is_genuine_pass`. Because requests are indexed first, a response saved before its request (A61 bug A) still resolves. If that last genuine pass was auto-detected (no `command`), it returns `None`.
  - `session_recorded_verify_command(pool, card)` reads `worker_session_id`. It first asks the existing reader (`session_verify_evidence(..).is_genuine()`, unchanged; R62ab owns the pairing), and only then extracts the command.
- **`fresh_verify_params(recorded, worktree, time_box, criteria)`** is shared by both call sites.
  - It uses the recorded command, and falls back to auto-detection only when none is recorded.
  - A recorded command that now fails is final: there is no fallback.
  - A recorded `path` is kept only when it stays inside the worktree (relative with no `..`, or absolute under the worktree).
  - Vacuous-pass detection is unchanged, because both callers still gate on `is_genuine_pass`.
- **`run_fresh_verify_for_hold` now takes `pool`.** Both callers are updated: `maybe_hold_review` and `decisions_effects.rs` `land_approved_goal`.
- **Salvage reviewer context.**
  - `gather_salvage_review_context` runs `git grep -n -I -w -F` in the goal worktree, over tracked files only.
    - E7 call sites for every signature `detect_signature_changes` finds in the diff: at most 12 per symbol and 8 symbols.
    - Definition lines of the functions called on the diff's added lines: at most 3 per function, using `call_site_completeness::definition_in_line` (now `pub`) plus the `const|let|var name =` shape.
    - Lines are capped at 240 characters, cut on character boundaries.
  - `build_salvage_review_prompt` appends the definitions to the verify slot, which already carries salvage's "no automated checks ran" note. When call sites were found, it uses `build_review_prompt_with_call_sites`.
  - The verdict now goes through `gate_decision_with_call_sites`, matching the main review path.
  - `call_reviewer_provider` and `call_reviewer_provider_checked` now take the built prompt instead of task/diff/note.

### Tests (orchestrator.rs)
- `last_genuine_verify_command_pairs_a_response_stored_before_its_request`
- `last_genuine_verify_command_ignores_vacuous_failed_and_non_verify_passes`
- `fresh_verify_params_uses_the_recorded_command_and_keeps_only_in_worktree_paths`
- `run_fresh_verify_for_hold_reruns_the_workers_recorded_passing_command`: runs the real verify with `test -f marker.txt` in a tempdir with no manifest. Covers three cases: nothing recorded fails, the recorded command passes, and the recorded command failing now returns false.
- `salvage_review_context_extracts_called_symbols_and_definition_lines`: pure test, including the prompt shape.
- `gather_salvage_review_context_shows_the_callee_signature_a_call_site_diff_omits`: uses a real git repo in a tempdir.

## R52c: fold a signature change split into one goal per call site

### Root cause
The s2 proposal had a `quoteShipping` signature change on `src/shipping/quote.ts` plus four call-site goals, each `depends_on:[0]`, each naming a different file and a different function. Neither existing merge caught it:
- `mechanical_shape_key` (orchestrator.rs:1451) needs identical masked text.
- `merge_overlapping_file_goals` (orchestrator.rs:1358) needs a file that two goals both name.

**Deviation from the brief:** a literal "non-empty intersection of named source files" would NOT have merged s2, because every goal named a distinct file. The overlap is through the shared callee.

### Changes
- **`merge_call_site_fan_out_goals(goals, facts)`.**
  - A goal B counts as a call-site dependent of root A when all of these hold:
    - B depends on A.
    - B names at least one source file (test files excluded via `tool_monitor::is_test_path`).
    - B names a camelCase or snake_case identifier that A also names. File-shaped tokens are dropped first, and product names like TypeScript are stoplisted.
  - A dependent's file set is its own files plus the root's, so it overlaps the root.
  - A root with two or more such dependents merges with them into one goal when the union of named source files is ≤ `CALL_SITE_FAN_OUT_MAX_FILES` (8). A larger union stays split.
  - If the merge would create a dependency cycle, it is discarded and the proposal is returned unchanged.
- **Wiring in `ground_roadmap_goals`.** The helper call and its receipt (`decompose_merged_call_site_fan_out=N`) sit between the overlapping-file merge and the R52b block. The cap's gate is now `merged_mechanical + merged_fan_out > 0`. Nothing else in the function changed.

### Tests (orchestrator.rs)
- `merge_call_site_fan_out_goals_folds_the_s2_signature_split`: uses the s2 titles and descriptions quoted from harness.log. Expects 5 goals → 1, every file kept, and a valid topological order.
- `ground_roadmap_goals_merges_an_overlapping_call_site_split`: end to end, including the receipt.
- `merge_call_site_fan_out_goals_keeps_disjoint_goals_split`: covers no dependency, and dependents that share no identifier with the root.
- `merge_call_site_fan_out_goals_keeps_an_oversized_union_split`: 10 files stay split; 8 files merge.
- `goal_code_identifiers_reads_camel_and_snake_case_but_not_file_names`
- `ground_roadmap_goals_keeps_a_split_when_every_goal_names_a_real_file` is untouched and still holds: it has no dependency, and fewer than 3 goals.

## R59b: recipe rule for read-only review requests

### Root cause (A61 miss 3)
After R59a, the s3 coordinator wrote the review of `payment.py` inline. It answered the Rule 5 nudge with "escalate is for a blocker" and ended in prose again, which parked it with `goals=0` and no escalation. The recipe had no sentence saying that a review request is itself a roadmap.

### Changes
- **Recipe sentence.** One sentence at the end of rule 1, directly after exception (b), in `permagent-coding.yaml`: a request to review or assess code without changing it becomes ONE read-only goal. That goal's description and acceptance criteria ask only for a written review report, worded as a review and never as a fix, so `infer_expects_change` marks it read-only. It is never reviewed inline ending in prose, and never escalated.
  - This fits the fixture: `must_not_call` lists only mutating tools, and the coordinator calls `decompose_roadmap`, `create_roadmap` and `goal_advance`.
- **Header.** An R59b line was added to the header lane block.
- **Test.** `builtin_permagent_coding_recipe_resolves_and_parses` now asserts the sentence.

## R62f: prompt noise in headless coordinators

### (a) Turn-budget nudge
- **Root cause.** `turn_budget_nudge_text` (after_turn.rs:3083) was gated only on `session_config.max_turns.is_some()` (agent.rs:3584). The headless CLI gives the coordinator a turn cap too, so it received "3 actions remain … git commit NOW" (A61 miss 2).
- **Fix.** New `after_turn::turn_budget_nudge_applies(max_turns, session_type)`: true only for an explicit cap on a `SubAgent` session. Goal workers are created as `SubAgent` (goal_engine.rs); the CLI coordinator is a `User` session. `reply_internal` computes it once from `session.session_type` and passes it as the nudge's cap flag.
- **Test.** `nudge_applies_only_to_capped_sub_agent_sessions`.

### (b) Tour offer
- **Root cause.** `prepare_tools_and_prompt` injects `tour_offer` whenever `tour_completed` is unset (reply_parts.rs:1059). A61 miss 5 found the coordinator offering a tour in 12 headless logs.
- **Fix.** The onboarding block (tour offer and learn-next hint) is skipped when `self.is_headless()` or `is_headless_dispatch_policy_installed()`.
  - The per-agent flag is checked first, but the headless CLI does not set it (only the scheduler does).
  - No per-session headless flag exists for CLI harness sessions, so the process-level policy is what catches them. A daemon or interactive session never installs that policy.
  - The learn-next hint is included because A61 lane 9 names it alongside the tour offer.
- **Test.** `a_headless_session_prompt_never_offers_the_tour`.
  - It uses the per-agent flag and sets no OnceLock.
  - Its non-headless control runs only when `tour_completed` is unset and no other test in the binary has installed the headless policy.

## Gate filters

```
# R62e
cargo test -p permagent --lib last_genuine_verify_command
cargo test -p permagent --lib fresh_verify_params_uses_the_recorded_command
cargo test -p permagent --lib run_fresh_verify_for_hold_reruns
cargo test -p permagent --lib salvage_review_context
cargo test -p permagent --lib maybe_hold_review_
cargo test -p permagent --lib salvage_
cargo test -p permagent --lib dispatch_cross_family_reviewer
cargo test -p permagent --lib call_site_completeness
# R52c
cargo test -p permagent --lib merge_call_site_fan_out_goals
cargo test -p permagent --lib goal_code_identifiers
cargo test -p permagent --lib ground_roadmap_goals
cargo test -p permagent --lib merge_mechanical_call_site_goals
cargo test -p permagent --lib cap_roadmap_goal_count
# R59b
cargo test -p permagent-cli --lib builtin_permagent_coding_recipe_resolves_and_parses
# R62f
cargo test -p permagent --lib turn_budget_nudge
cargo test -p permagent --lib a_headless_session_prompt_never_offers_the_tour
cargo test -p permagent --lib the_prompt_says_which_model_is_actually_serving
```

Also run `cargo check -p permagent-server`. The server does not call `run_fresh_verify_for_hold`; this confirms nothing else did. Then clippy with `-D warnings`.

## Risks
- **Recorded command on the wrong PATH.** The re-run uses verify's own process PATH. Until R62ab's PATH fix lands, a recorded `python3 -m pytest` can still hit ENVIRONMENT. Absolute interpreter paths like `/usr/bin/python3` work today.
- **Evidence-reader gate.** The recorded command is only consulted when `session_verify_evidence` credits a genuine pass. Until R62ab makes pairing order-independent, an out-of-order genuine PASS (A61 bug A) still gets no re-run. Extraction itself is already order-independent.
- **Last worker session only.** The command comes from the latest `worker_session_id`. A genuine pass that only happened in an earlier redispatch session is not found.
- **Stricter salvage gate.** The reviewer gate is now `gate_decision_with_call_sites`. A diff that changes a signature holds unless the reviewer emits a CALLSITE line for every enumerated site. That matches the main review path, but a salvage that used to approve with a bare APPROVE can now hold.
- **Reviewer prompt size.** Context is bounded to roughly 30k characters on top of the ≤40k-character diff.
- **Fan-out merge scope.** R52c merges any fan-out of ≥2 dependents that share a camelCase or snake_case identifier with their root. A real independent fan-out that happens to share an identifier (for example, two features both using a `userId`) would now be one goal if it spans ≤8 files. The fan-in shape (integration goal depending on two roots, as in failing-test) is not affected. It runs before R62cd's shared-oracle chaining, so any edges R62cd adds come after this merge.
- **Test depends on `test` builtin.** `run_fresh_verify_for_hold_reruns_…` needs `sh` with `test`. Its no-recorded-command case assumes auto-detection in a manifestless tempdir is not a genuine pass.
- **Process-global tour gate.** The R62f tour gate is process-level for the CLI harness. Any interactive session sharing a process that installed the headless policy would also lose the tour offer; none exists today.
- **Nudge now off for non-`SubAgent` capped sessions.** Any capped session that is not `SubAgent` no longer gets the turn-budget nudge (for example a scheduled capped run). Before, every capped session did.
