# Lane N3: the reviewer DELEGATE path uses the objective reviewer assignment

Branch `lane/N3`, based on `ed03d657` (M48). I only edited files. No cargo command ran, so nothing here has been compiled or tested yet. `rustfmt --edition 2021 --check` is clean on every touched `.rs` file.

## Evidence check on r64 s5 (read this first)

The run dir does not show a delegate reviewer running on Haiku:

- `logs/harness.log` has no `delegate` tool call. The coordinator's tool calls were `routing_roster`, `map_query`, `analyze`, `read` ×6, `decompose_roadmap`, `create_roadmap` and `goal_status`.
- The one `anthropic/claude-haiku-4-5` row in `cost_ledger` (\$0.006191, 1581 in / 922 out) is on the **coordinator** session `20260917_1` at turn 4, 11:45:15.537. It has no `parent_session_id` and no `goal_id`.
  - That is 23 ms before the roadmap approval decision `01a0af2f-6b28…` was created (`decision_audit`, 11:45:15.560).
  - It is 4 s before the goal was dispatched (11:45:19.65), so no code existed yet to review.
  - It was the `decompose_roadmap` call, most likely the planner. The receipt still prints `planner=anthropic/claude-fable-5`, and no Fable row is in the ledger. That mismatch belongs to another lane.
- The goal card's `review_notes` read "Pre-approved: unattended headless Auto landing — durable verifier pass confirmed". The goal auto-landed on a verifier pass. No goal-level independent reviewer ran either, which is why there is no `reviewer_assignment`.
- The acceptance criteria that the plan wrote into the card say: `src/admin/manualQuote.ts passes "standard" as the third argument to quoteShipping`. The coordinator's summary calls this a deliberate choice. The oracle failure ("admin tool must support package type") is therefore a **planning** miss. Any reviewer checking the diff against those criteria would have approved it.

So this lane would not have changed s5's outcome. The wiring gap it closes is still real. R63c's own report lists it as a risk: after auto-land, the delegate path sizes the review with `git diff --shortstat HEAD`, which reads 0. I implemented it as briefed.

## Changes (file:line at the lane commit)

### `crates/goose/src/cost_router/recommend.rs`
- `:1296` `ReviewComplexity` derives `PartialOrd, Ord` (ROUTINE < MODERATE < COMPLEX).
- `:1631` `assign_reviewer` is now a wrapper for `assign_reviewer_at_least(facts, Routine, None, …)`. Its behaviour is unchanged.
- `:1644` new `assign_reviewer_at_least(facts, minimum, minimum_reason, worker, available)`. It raises the verdict and floor to `minimum` and records the reason. The delegate path uses it for an unmeasured review.

### `crates/goose/src/agents/platform_extensions/orchestrator.rs`
- `:22971` `enumerate_signature_call_sites(worktree, changes)`: R62e's `git grep` call-site loop, pulled out of `gather_salvage_review_context`, which now calls it. The goal gate and the delegate path count call sites from this one source.
- `:23331` `write_reviewer_assignment(...)`: the single `reviewer_assignment` writer.
  - `record_reviewer_assignment` (goal gate) delegates to it and writes the same keys as before.
  - The delegate path also adds a `diff_source` key.
  - It also emits the `reviewer assignment` tracing line.
- `:23396` `ReviewDiffSource` has five values: `goal_worktree_fork_point`, `goal_fork_point`, `goal_landed_range`, `working_tree`, `unmeasured`.
- `:23445` `assign_delegate_reviewer(pool, session, author, available) -> DelegateReviewAssignment`. The dispatch and the mandate share this one function.
  1. **Goal context** (`:23547` `review_goal_for_session`): if the session is a goal worker (`goal_card_for_worker_session`), use that goal. Otherwise use the most recently finished goal this session created that has a recorded head. The recency key is `completed_at`, else `updated_at`.
  2. **Trunk-relative diff** (`:23602` `goal_trunk_relative_diff`):
     - Trunk tip uses the goal gate's rule: `HEAD` of `resolve_trusted_landing_root` when the goal worktree still exists, else the project's `root_path`. Failing both, `detect_trunk_branch` runs in the goal's directory (`developer/verify.rs:1387`, made `pub(crate)`).
     - Goal worker: `git diff <merge-base trunk HEAD>` in its worktree, covering committed and uncommitted work.
     - Recorded head not on trunk: `merge-base(trunk, head)..head`.
     - Recorded head already on trunk (landed): `baseline_commit..landed_head`. `baseline_commit` is read top-level first, then from `dispatch_evidence`. The head is `dispatch_evidence.landing.landed_head`, else `head_commit`.
     - It never diffs against `HEAD` in the coordinator's directory.
  3. **No goal range:** `git diff HEAD` in the session's working dir.
  4. **Nothing measurable:** `assign_reviewer_at_least(…, Moderate, UNMEASURED_REVIEW_REASON)`, which fails toward a capable reviewer.
  5. **Call sites:** `enumerate_signature_call_sites` over the diff's `detect_signature_changes`, then `ReviewFacts::from_diff(&diff, call_sites)` → `recommend::assign_reviewer`.
  6. Emits a tracing line, `delegate reviewer assignment`, with the goal, diff_source, verdict, floor, lines, call sites, pick and why.
- `:23524` `record_delegate_reviewer_assignment(pool, live, selected, selected_source)` writes `reviewer_assignment` to the goal card through `write_reviewer_assignment`. It does nothing when there is no goal.

### `crates/goose/src/agents/platform_extensions/summon.rs`
- `:2715` `resolve_review_dispatch`: R63c's precedence is kept.
  - A `RolePin` source, or a dedicated `PERMAGENT_PACK_REVIEW_*` pin, still wins, and no assignment is computed in that case.
  - Otherwise the cross-family candidate is `assign_delegate_reviewer(...).assignment.pick_role_model()`. It replaces `cheapest_cross_family_reviewer(author, available, git-diff-HEAD lines)`.
- `:2782`: after the dispatch resolves, the assignment is recorded on the goal card with `selected` = the dispatched model and `selected_source` = `assigned` (or `review_dispatch` when the dispatch did not take the pick, e.g. a same-family fallback).
- `ReviewDispatchResolution` / `ReviewAuditInputs` carry `review_floor`. `ReviewAuditInputs` drops `Eq` because of the `f64`.
- **Receipt audit (item 5):**
  - With no dedicated pin, `delegate_audit_targets` already makes the review candidate both the pin exception and the reference. Now that the candidate is the `assign_reviewer` pick, a child on that pick is counted as `cheapest_capable` and `pack_honored` by construction. The assignment is, by definition, the objective cheapest model that clears the floor.
  - There was one gap. `audit_delegate_dispatch` also accepts any child no pricier than the reference, which for review would count a cheaper model below the complexity floor as capable (for example, an explicit `model: claude-haiku-4-5` on a complex review).
  - `:3644` `review_floor_holds` closes it. It applies only when the reference is the assigned reviewer: a child that is not the pick must clear the floor on its knowledge-base strength, and an unrated model counts as unproven.
  - `delegate_cost_audit` (`:2621`) ANDs this in.
  - The pure `audit_delegate_dispatch` and the receipt line format are unchanged.
- The R63c test helper `r63_review_dispatch` now takes `ReviewFacts` and uses `assign_reviewer`. The three R63c tests pass `r63_one_line()` (routine, the same Haiku answer as before).

### `crates/goose/src/after_turn.rs`
- `:592` `ReviewerMandate::assess`: the prediction's cross-family candidate is the same `assign_delegate_reviewer` pick, using `SessionManager::instance().pool_clone()`, so the prediction equals the dispatch. `select_reviewer` and the spend gate are untouched.

### `crates/goose/src/cost_router/reviewer_pick.rs`
- The doc on `cheapest_cross_family_reviewer` now notes that neither delegate path calls it any more. It is kept because its own tests still use it.

## Tests

`orchestrator.rs` (real git tempdir plus in-memory spectral DB):
- `n3_the_r64_s5_signature_refactor_is_complex_and_not_reviewed_by_haiku` (`:32912`):
  - Fixture: 5 TS files; `quoteShipping` gains `packageType`; checkout, cart and API pass `order.packageType`; admin passes `"standard"`; plus a test file.
  - A coordinator-created goal card, with `git diff HEAD` asserted to be 0.
  - Before the land: `goal_fork_point`, COMPLEX.
  - After the `--ff-only` land: `goal_landed_range`, COMPLEX, floor 0.80, 5 code files, signature `quoteShipping`, ≥ 4 call sites, pick `claude-sonnet-5`, not Haiku.
  - `reviewer_assignment` on the card: complexity, floor, pick, `selected`, `selected_source`, `diff_source`, why, and 4 candidates with strength and price.
- `n3_the_mandate_prediction_equals_the_delegate_dispatch` (`:33015`): the assessment runs as the dispatch (and records), then again as the mandate. The assignments are equal, and `resolve_actual_review_dispatch` gives the same route and warning (Sonnet).
- `n3_a_one_line_typo_review_is_routine_and_gets_the_cheapest_clearing_model` (`:33077`): no goal, working tree → ROUTINE, floor 0.60, `claude-haiku-4-5-20251001`.
- `n3_an_unmeasured_review_fails_toward_a_capable_reviewer` (`:33119`): clean tree, no goal → `unmeasured`, MODERATE 0.75, the reason is recorded, pick Sonnet.

`summon.rs` (pure, pins via one `env_lock::lock_env` per test, same as R63c; no OnceLock):
- `n3_a_complex_review_is_assigned_a_capable_reviewer_and_the_audit_accepts_it` (`:6064`):
  - `dschat` packs, s5 diff with 4 call sites → COMPLEX 0.80 → dispatch `claude-sonnet-5`.
  - The audit counts Sonnet as cheapest-capable and pack-honoured.
  - A Haiku child is not cheapest-capable (below the floor), and neither is a Fable child (pricier).
- `n3_a_one_line_typo_review_is_assigned_the_cheapest_clearing_model` (`:6118`): ROUTINE → Haiku, and the audit counts it cheapest-capable.
- `n3_a_dedicated_review_pack_pin_still_wins_over_a_complex_assignment` (`:6154`): a `PERMAGENT_PACK_REVIEW_*` pin wins on a complex diff. No assignment is made; the audit judges against the pin.
- `n3_a_review_role_pin_still_wins_over_a_complex_assignment` (`:6188`): a `PERMAGENT_ROLE_REVIEW_*` pin gives a `RolePin` source and dispatches on the pin.

No receipts-registry test was added, so none needs `serial(receipts_global_registry)`.

## Gate filters

The crate is `-p permagent` (crates/goose).

```
cargo test -p permagent --lib n3_
cargo test -p permagent --lib r63c_
cargo test -p permagent --lib cost_router::recommend
cargo test -p permagent --lib cost_router::reviewer_pick
cargo test -p permagent --lib cost_router::delegate
cargo test -p permagent --lib cost_router::routing_receipt
cargo test -p permagent --lib agents::platform_extensions::summon
cargo test -p permagent --lib after_turn::tests::actual_route
cargo test -p permagent --lib orchestrator::tests::reviewer_assignment_is_recorded_on_the_goal_card
cargo test -p permagent --lib orchestrator::tests::gather_salvage_review_context
cargo test -p permagent --lib orchestrator::tests::dispatch_cross_family_reviewer
cargo test -p permagent --lib developer::verify
cargo test -p permagent --lib identity_name_guard
cargo clippy -p permagent --all-targets -- -D warnings
```

## Risks

- **Not compiled.** These are the most likely spots for a nit:
  - the `nonempty` closure reused across several `Option::filter` calls in `goal_trunk_relative_diff` (it captures nothing, so it is `Copy`)
  - the `let … else` pattern in `review_floor_holds`
  - the four-tuple in `resolve_review_dispatch`
- **The s5 premise does not hold** (see the top section). This lane would not have fixed the admin-tool miss.
- **Which goal a coordinator's reviewer covers.** It is the most recently finished goal that session created. A reviewer delegate that covers a whole multi-goal roadmap is measured on only the latest goal. A delegate about something unrelated is still measured on that goal, which fails toward a capable reviewer.
- **Landed range uses the recorded `baseline_commit`.** A Y16b redispatch re-points the baseline at the prior attempt's tip, so for a multi-attempt goal that has already landed, earlier attempts' commits can fall outside the measured range and the review can read smaller than it was. An unlanded goal uses the true `merge-base`.
- **Author is still the delegating session's model** (R63c's choice, kept so the dispatch's same-family labelling matches). For a coordinator reviewing a worker on a different family, cross-family is judged against the coordinator's family, not the worker's.
- **Extra cost per reviewer delegate.** One DB scan (`goal_card_for_worker_session` walks goal cards when the session has no goal identity), up to ~5 git calls (20 s timeout each) and one `git grep` per changed signature (at most `SALVAGE_REVIEW_CONTEXT_MAX_SYMBOLS`). The mandate runs the same work again when it fires.
- **An unmeasured review is now MODERATE (0.75) instead of the 0.60 small-diff floor.** On the `dschat` roster an interactive reviewer delegate with a clean tree gets Sonnet rather than Haiku. That is the brief's "fail toward capable" rule, and it costs more in that case.
- **The audit tightens for explicit per-call models.** A cheaper child below the complexity floor on an assigned review now prints `cheapest_capable=0`, where R63c's audit printed 1.
- **Delegate records overwrite the key.** A delegate `reviewer_assignment` overwrites a goal-gate one on the same card (same key, last writer wins). `diff_source` tells the two apart.
