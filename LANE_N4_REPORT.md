# Lane N4: grounded planning complexity, a durable verdict, an honest planner receipt, and review before auto-land

Branch `lane/N4` in worktree `permagent-worktrees/laneN4`. It is based on `ed03d657` (M48) with Lane N3 (`b7c2b293`) merged in (`d378edae`).

**Nothing here has been compiled or tested.** No cargo command ran. Every touched `.rs` file passes `rustfmt --edition 2021 --check`.

Evidence throughout is M48 record r64 s5, `harness-refactor-shipping-quote-signature`.

| # | Commit | What |
|---|---|---|
| 1 | `15490d10` | Planning complexity reads grounded repo facts. The planner call runs on the routed model, not the provider's fast model. |
| 2 | `107f29f5` | The verdict, reasons, signals, intended route and physical planner are kept on the project and every goal card. |
| 3 | `130cdd2f` | `receipt cost_routing planner=` names the planner that answered, or `none`. The eval parser accepts `none`. |
| 4 | `884cbfe4` | A non-trivial verified goal needs an independent cross-family review before headless auto-land. |

The item-5 tests are in the commits for the items they cover.

## 1. Diagnosis

### 1a. s5 was not ROUTINE: the COMPLEX route was silently downgraded to Haiku

I ran N2's rules by hand on the objective the coordinator actually passed to `decompose_roadmap`. I pulled it from the run's `permagent.db` messages; it matches harness.log ~137–139.

- It names 7 path-shaped tokens: `src/checkout/checkout.ts`, `src/cart/cartSummary.ts`, `src/api/quoteHandler.ts`, `src/admin/manualQuote.ts`, `src/shipping/quote.ts`, `src/types.ts` and `src/**/*.test.ts`.
- That is at or above `PLANNING_MULTI_FILE_THRESHOLD` = 3 (base `planner_complexity.rs:249`), so the verdict was **COMPLEX**.
- The COMPLEX verdict routed the plan to the expert planner, `anthropic/claude-fable-5`. The receipt's `planner=anthropic/claude-fable-5` is the configured model of the provider built for that route.

So why did Haiku answer?

- `run_decomposition` (base `orchestrator.rs:13042` and `:13066`) called `AccountedFastCompletion::complete_fast_accounted`.
- That function runs `regular.use_fast_model()` (`agents/reply_parts.rs:2192–2196`).
- The Anthropic provider sets its fast model to `ANTHROPIC_DEFAULT_FAST_MODEL = "claude-haiku-4-5"` (`providers/anthropic.rs:32`, `:66`).
- Result: the gate chose Fable, and a Haiku call was billed. That is the \$0.006191 cost_ledger row on coordinator session `20260917_1`, turn 4.

The brief's hypothesis still holds for the fixture's own prompt (`task.yaml`). That prompt names surfaces and `quoteShipping` but no file path, in an 8-file repo. Under N2 it reads ROUTINE:

- there are 0 named files (`:249`),
- the repo is under 500 files, so the unscoped signal is off (`:272`),
- there are no enumerated lines.

A coordinator that passes the task through verbatim would get a cheap plan.

### 1b. The verdict was never durable

- `decompose_roadmap` writes `planner_complexity*` onto `projects.metadata_json.roadmap_proposal` (base `orchestrator.rs:11289`).
- `create_roadmap`'s materialization then consumes that single-use proof with `json_remove(metadata_json, '$.roadmap_proposal')` (base `:4404`).
- The verdict was deleted with it. The r64 project metadata is `{}`.

### 1c. The receipt printed a planner that never answered

- `set_planner(actual_planner.get_name(), actual_planner.get_model_config().model_name)` (base `orchestrator.rs:11217`) recorded the provider's **configured** model. It never saw the model in the response.
- `goose-cli/src/session/builder.rs:1368` also seeded the receipt with the coordinator at session start, so a run with no decompose call still claimed a planner.
- `permagent-eval` `harness_log.rs:634` required `planner=(\S+/\S+)`. A bare `none` would have made the whole cost_routing receipt unparseable. So no, it did not parse `none` before this lane.

### 1d. Auto-land bypassed review; it was not a trivial-skip misclassification

- The reviewer mandate's trivial skip is `review_gate::review_required` (`cost_router/review_gate.rs:494`): skip when no product source is touched, or at most `TRIVIAL_MAX_LINES` (3) lines change. It is controlled by the E1 knob `PERMAGENT_REVIEWER_MANDATE_SKIP_TRIVIAL` (`after_turn.rs:749`).
- By that rule the s5 diff (5 files, dozens of lines) is plainly **non-trivial**.
- The skip was never consulted, though. `auto_land_headless_goal`'s `VerifyEvidence::Genuine` arm (base `orchestrator.rs:16286`) recorded `status: pass` (`:16389`) and called `finish_headless_auto_land` (`:16423`).
- That function auto-answered `approve_review` with "Pre-approved: unattended headless Auto landing — durable verifier pass confirmed" (`decisions.rs:2834`). Nothing reviewed anything.
- The goal-level cross-family gate (`dispatch_cross_family_reviewer_for_salvage`) only ran for unverified salvage.
- `GOAL_REVIEW_HOOK` (base `:15869`) runs after the land, and only in the daemon.

So auto-land bypassed review for every change size.

## 2. Changes

### Item 1: grounded assessment (`cost_router/planner_complexity.rs`, `orchestrator.rs`)

**Named paths**

- When the repo's file list is known (`PlanningFacts.repo_files` is non-empty), a named path counts only if it resolves to a real file: an exact relative path, or a `/<path>` suffix (`path_resolves`, `:374`).
- Unresolved paths go to `unresolved_named_files` and are not counted. That drops globs and files that don't exist.
- Named test files (`tool_monitor::is_test_path`) go to `named_test_files` and are not counted toward the multi-file line or languages.
- With no repo files (no repo, or greenfield) the lexical N2 count stands.

**Identifier resolution**

- `orchestrator.rs:1788` `text_code_identifiers(text)` is R52c's camelCase/snake_case extractor, factored out of `goal_code_identifiers` so both share it.
- `orchestrator.rs:1101` `planning_identifier_references` sorts the identifiers and calls `planner_complexity.rs:486` `scan_identifier_references(root, repo_files, identifiers)`.
- The scan reads only non-test source files from the repo-facts walk, which already skips vendored and build directories.
- For each identifier it records the files that **define** it and the files that **reference** it as a whole word:
  - a definition is the identifier directly preceded by a keyword: `fn`, `function`, `def`, `func`, `class`, `const`, `let`, `type`, and so on;
  - whole-word matching uses a byte-level word-boundary check (no string slicing).
- It runs in `spawn_blocking` at `orchestrator.rs:11020`.

**New COMPLEX signal: a signature change that fans out**

The objective asks for a parameter change (`requests_parameter_change`, `:434`), and a named identifier meets both conditions:

- it is defined in exactly one non-test source file;
- it is referenced from at least `SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD` other non-test source files.

`requests_parameter_change` means: in one sentence, a non-negated change verb (add, more, extra, new, remove, drop, reorder, change, gain…) appears alongside a parameter noun (parameter, argument, signature, arity) or the phrase "calling convention". Negation means "not", "never", "no", "without" or "don't" within the two words before the verb.

The wording only says what kind of change is asked for. The fan-out read from the repo is what promotes it. Difficulty prose still never promotes (the `assess.rs` rule), and the tests check that.

**The planner call**

`run_decomposition` (`orchestrator.rs:13197`) now calls `complete_accounted`: the routed provider's own model, never its fast model.

**New verdict fields**

`unresolved_named_files`, `named_test_files`, `parameter_change_requested`, `identifiers` (identifier, defined_in, referencing_files count), `identifier_reference_files` and `signature_changes`. The tracing line adds `signature_changes` and `identifier_reference_files`.

### Item 2: durable record (`orchestrator.rs`)

- `run_decomposition` reports the model the response named (`ProviderUsage.model`) through `answered_model`.
- `answered_planner` (`:13170`) builds `provider/model`. It falls back to the configured model when the response names none.
- The proof records it as `planner_physical` (`:11428`).
- `roadmap_planning_record` (`:579`) copies `planner_complexity`, `planner_complexity_reasons`, `planner_complexity_signals`, `planner`, `planner_reason`, `planner_physical` and `proposal_hash`. Keys listed in `ROADMAP_PLANNING_PROOF_KEYS`.
- The proof-consuming `UPDATE` (`:4474`) now also does `json_set(..., '$.roadmap_planning', json(?))` in the same statement.
- Every materialized goal card gets `roadmap_planning` (`:4545`).

### Item 3: receipt

- `routing_receipt.rs:100` adds `PLANNER_NONE = "none"`. It replaces `unknown/unknown` when nothing was set. The key and line format are unchanged.
- The decompose handler calls `set_planner` (`orchestrator.rs:11314`) as soon as an answer exists, with the answering provider and the model from the response. An answer that then fails to parse, or is refused as a blocker, still names who answered.
- `goose-cli/src/session/builder.rs` no longer seeds the planner with the coordinator at session start. Its source-text test now asserts there is no `set_planner` call. `cli.rs` comment updated.
- `permagent-eval/src/harness_log.rs:634`: the regex is now `planner=(\S+/\S+|none)`. `none` is kept at any dispatch count, because a resume-only run dispatches without planning. The legacy `unknown/unknown` sentinel logic is unchanged.

### Item 4: review before headless auto-land (`orchestrator.rs`)

**The rule:** `auto_land_review_requirement(facts)` (`:16702`) is pure.

- **Required** when N2's `assess_review_complexity` is MODERATE or above, or when `review_required` says Review.
- **Skip** only when the mandate's own rule skips (no product source, or ≤ 3 changed lines) *and* the complexity is ROUTINE. So a small signature change with callers is never waved through.
- The E1 A/B knob is **not** read on this path. Auto-land always applies the mandate's default (skip trivial). That keeps the knob scoped to the after-turn hook, and it avoids env-dependent landing tests.

**The flow:** `independent_review_before_auto_land` (`:16754`), called from `auto_land_headless_goal` (`:16551`) after the Genuine / unverified-AC checks and before the pass receipt.

1. It measures the goal's trunk-relative diff with N3's `goal_trunk_relative_diff`, then counts call sites (`enumerate_signature_call_sites`) and builds `ReviewFacts`.
2. **Skip:** the goal lands as before.
3. **Required:** the diff goes to `review_goal_diff_with_cross_family_reviewer` (`:23514`).
   - This is the salvage gate's body, extracted and shared: the `assign_reviewer` pick into `select_reviewer`, the spend gate, the E7 call-site `gate_decision`, and the unavailable-provider retry.
   - Its verify slot says a verifier pass exists, and asks for review against every criterion and every surface named.
4. Only `Approved` continues to land. `Refused`, `Unreachable` and `DiffTooLarge` all call `park_refused_auto_land` with the reason (Y10b).
5. An unmeasurable diff (no head, no trunk, or an empty range) is NotRequired: there is nothing to read, and landing itself handles "nothing to land".
6. The outcome is written to card metadata `auto_land_review` (required, reason, diff_source, outcome, reviewer / checked or refusal, recorded_at) and traced.

**Supporting changes**

- `dispatch_cross_family_reviewer_for_salvage` (`:23458`) now just computes its diff and calls the shared function with the salvage lead. Its behaviour is unchanged.
- `build_salvage_review_prompt` is now `#[cfg(test)]`; its existing tests still pin the composition. Prompts are built through `build_goal_review_prompt`.

## 3. Signals and thresholds (named constants, with doc comments)

| Constant | Value | Role |
|---|---|---|
| `PLANNING_MULTI_FILE_THRESHOLD` | 3 | Distinct **resolved, non-test** named files, so COMPLEX (the meaning changed to grounded + non-test) |
| `PLANNING_CROSS_LANGUAGE_THRESHOLD` | 2 | Unchanged |
| `LARGE_REPO_FILE_THRESHOLD` | 500 | Unchanged |
| `SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD` | 3 | Other non-test source files referencing a singly-defined identifier whose parameters the objective changes, so COMPLEX |
| `PLANNING_MAX_IDENTIFIERS` | 16 | Identifiers resolved per objective (sorted, so deterministic) |
| `PLANNING_SCAN_MAX_FILE_BYTES` | 4 MiB | Larger files are skipped |
| `PLANNING_SCAN_MAX_TOTAL_BYTES` | 64 MiB | Scan budget. Counts become a lower bound, so the scan can only miss a promotion, never invent one |
| `PARAMETER_CHANGE_VERBS` / `PARAMETER_NOUNS` / `NEGATION_WORDS` / `DEFINITION_KEYWORDS` | lists | Wording and definition vocabularies |

### Spot-check

I ported the new assessment to Python and ran it against each fixture's `task.yaml` prompt and workspace.

| Objective | Result |
|---|---|
| s5 task prompt | **COMPLEX**: `quoteShipping` defined in `src/shipping/quote.ts`, referenced from 5 files, 4 callers |
| s5 coordinator objective | **COMPLEX**: 6 resolved files, plus the signature signal |
| `select-trivial-typo-fix-skips-roadmap` | ROUTINE |
| `select-mechanical-rename-skips-roadmap` | ROUTINE (`old_name`: 1 caller, no parameter change) |
| `select-named-check-uses-verify-tool` | ROUTINE |
| `select-checks-worker-before-dispatch` | ROUTINE |
| `select-failing-test-decompose-before-create` | ROUTINE (was COMPLEX under N2) |
| harness-cli / loc / overreach / editapply, defaults-rs-parse-amount | ROUTINE |

`select-failing-test-decompose-before-create` flips because `test_checkout.py` no longer counts, leaving 2 source files.

## 4. Tests added

**`cost_router::planner_complexity`**

- `r64_s5_signature_refactor_prompt_is_complex_against_its_fixture_repo`: the verbatim fixture files (`R64_S5_FIXTURE_FILES`) plus the task prompt give COMPLEX. The same prompt without repo facts gives ROUTINE.
- `a_one_file_typo_fix_stays_routine_in_a_grounded_repo`
- `named_paths_count_only_when_they_resolve_and_are_not_tests`
- `fan_out_without_a_parameter_change_stays_routine`, including "do not change its signature"
- `a_parameter_change_with_a_small_fan_out_is_routine_even_with_difficulty_prose`
- `parameter_change_wording_is_read_per_sentence_and_respects_negation`
- `identifier_scan_finds_definitions_and_whole_word_references_outside_tests`

**`orchestrator::tests`**

- `n4_decomposition_facts_make_the_r64_s5_objective_complex_and_a_typo_routine`: `RepoFacts::gather` + `planning_identifier_references` end to end.
- `n4_planning_verdict_is_persisted_after_create_roadmap`: the proof is consumed; `roadmap_planning` is on the project and both cards.
- `n4_answered_planner_prefers_the_model_the_response_named`
- `n4_a_complex_verified_change_cannot_headless_auto_land_without_an_independent_review`:
  - a real git fixture with the 5-file `quoteShipping` refactor on a goal branch, plus a Genuine verify;
  - the review is required; the reviewer cannot run (the worker session is not readable);
  - result: parked Failed, HEAD not advanced, no pass receipt, `auto_land_review.required = true`, outcome `refused`.
- `n4_a_trivial_verified_change_still_headless_auto_lands_per_the_mandate`: a 2-line rate change lands; `auto_land_review.required = false`.
- `n4_auto_land_review_requirement_follows_the_mandate_and_review_complexity`: docs-only skips, a 2-line tweak skips, a signature change with callers is required, 10 lines + test is required.

**Receipts**

- `cost_router::routing_receipt::tests::planner_field_names_the_answering_planner_or_none` (serial `receipts_global_registry`). Updated the zero-line / reset tests to `none`, plus `receipts.rs` `sidecar_receives_all_four_contract_lines_on_a_counter_change`.
- `permagent-eval` `harness_log::tests::routing_receipt_parses_planner_none_at_any_dispatch_count`
- `goose-cli` `session::builder::tests::cost_routing_planner_is_not_seeded_with_the_coordinator_at_session_start` (replaces the R22b-b seeding test)

## 5. Gate filters

```
cargo test -p permagent --lib cost_router::planner_complexity
cargo test -p permagent --lib n4_
cargo test -p permagent --lib cost_router::routing_receipt
cargo test -p permagent --lib receipts::tests::sidecar_receives_all_four_contract_lines_on_a_counter_change
cargo test -p permagent --lib orchestrator::tests::headless_auto_land
cargo test -p permagent --lib orchestrator::tests::a_refusal_without_auto_land_leaves_the_approve_review_open
cargo test -p permagent --lib orchestrator::tests::read_only_auto_land
cargo test -p permagent --lib orchestrator::tests::land_salvage_approved_by_reviewer
cargo test -p permagent --lib orchestrator::tests::dispatch_cross_family_reviewer
cargo test -p permagent --lib orchestrator::tests::build_salvage_review_prompt
cargo test -p permagent --lib orchestrator::tests::gather_salvage_review_context
cargo test -p permagent --lib orchestrator::tests::goal_code_identifiers
cargo test -p permagent --lib orchestrator::tests::merge_call_site_fan_out
cargo test -p permagent --lib orchestrator::tests::prior_plan_history_reads_parked_held_and_pending_proposal
cargo test -p permagent --lib orchestrator::tests::auto_launch_session_creates_roadmap_with_no_human_answer
cargo test -p permagent --lib orchestrator::tests::materialize_roadmap_transaction
cargo test -p permagent --lib orchestrator::tests::create_roadmap
cargo test -p permagent --lib orchestrator::tests::decompose
cargo test -p permagent --lib orchestrator::tests::resolve_decomposition_provider
cargo test -p permagent --lib -j1 -- --ignored replay_slice_smoke     # the decompose request now uses the regular model config
cargo test -p permagent --lib n3_
cargo test -p permagent --lib identity_name_guard
cargo test -p permagent-eval --lib harness_log
cargo test -p permagent-eval --lib qualification
cargo test -p permagent-cli --lib session::builder::tests::cost_routing_planner   # crate goose-cli is package permagent-cli; use --bin permagent if session is not in its lib
cargo clippy -p permagent -p permagent-eval -p permagent-cli --all-targets -- -D warnings
```

## 6. Risks

**Behaviour changes**

- **Planner cost goes up where the fast model used to hide it.** Every decompose now bills the routed model (Fable for COMPLEX, and the coordinator's own model for `RoutineCoordinator`) instead of the provider's fast model. That is what the gate decided, but routine plans on an Anthropic coordinator no longer quietly drop to Haiku.
- **Headless auto-land now costs one cross-family reviewer call per non-trivial goal.** It also parks the goal when:
  - no cross-family reviewer is available (single-family rosters),
  - the spend gate refuses,
  - the reviewer's provider is unreachable, or
  - the trunk-relative diff exceeds `REVIEWER_GATE_MAX_DIFF_CHARS` (40,000 chars).

  Large legitimate multi-file goals will park rather than land unreviewed. That follows the brief ("always gets an independent review"), but expect bench outcomes to shift: fewer auto-lands, more parked-with-reason. A reviewer that approves wrongly (s5's own criteria said admin passes `"standard"`) still lands. The verify-slot lead asks it to check every surface the task names, but the task spec it sees is the card's title and criteria.

**Classification limits**

- **Signature detection is heuristic.** A definition needs a keyword directly before the name, so class methods (`name(args) {`) and Go methods with receivers are not seen as definitions. A parameter change to those does not promote; this is a false negative, not a spurious COMPLEX.
- **The wording check can misfire.** A sentence like "add a note about the argument" next to a widely-called identifier can promote. That needs a singly-defined identifier with 3 or more caller files, so spurious flips should be rare.
- **Named test files no longer count** toward the multi-file line. `select-failing-test-decompose-before-create` flips COMPLEX → ROUTINE.

**Receipts and eval**

- **`planner=none` breaks old expectations.** Any downstream expecting `unknown/unknown` for zero-dispatch runs now sees `none`; `harness_log` handles both. Resume-only runs that dispatched used to print the coordinator. They now print `none`, and eval keeps the receipt. If a qualification gate required a slash-form planner, it would now see `none` (I found none that does).

**Timing and shared state**

- **Scan cost:** up to 64 MiB of file reads per decompose in a large repo, off the async runtime.
- **The complex auto-land test** relies on `SessionManager::instance().get_session("verify-<uuid>")` failing (not found), so the reviewer is never dispatched. That is the same global-storage lookup the salvage tests already hit. If a test environment ever made that session readable, the reviewer path would read the global config.
- **`auto_land_review` lives in card metadata.** It is written before the landing effect runs. If a later effect rewrites metadata from a stale snapshot the key could be lost; I found none that does.

**Not compiled.** The most likely spots for a compile nit:

- the `CASE WHEN ? IS NULL` double bind,
- the tuple-returning `match` in `independent_review_before_auto_land`,
- the `split_once` / `unwrap_or` lifetime unification at the `set_planner` call,
- `Option::is_none_or` in `sentences`.
