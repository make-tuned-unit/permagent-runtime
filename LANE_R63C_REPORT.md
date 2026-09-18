# Lane R63c: delegate cost routing, receipts and live-goal refusal

Branch `lane/R63c` (base e03e6d46), commit `ce42f127`. Edit-only. cargo was not run. rustfmt `--check` passes on every touched file.

## Evidence (M47 r63 s5, `dschat`, harness-overreach-narrow-coupon-trim)

- harness.log ~181: the call was `delegate` with `worker_persona: reviewer`. It came after goal_status ×2 and view_session ×2. The coordinator described it as "the mandate: an independent, different-model reviewer".
- cost_ledger: the child session 20260917_3 made 5 calls on `anthropic/claude-fable-5`, about $1.97 in total. The largest cost was a $1.09 cache write on turn 0. Every other call went to deepseek-chat, plus one $0.0025 haiku call from the coordinator at turn 3.
- The goal card's `completed_at` is 07:36:42.73. The delegate ran at about 07:36:47, so the goal was already **complete** by then (see "Behaviour" below).
- The printed receipt was `dispatches=1 cheapest_capable=1 pack_honored=1`.

## Root cause (file:line at e03e6d46)

1. **Routing.** The delegate did not inherit the planner model, and no per-call model param was passed. The problem is in `summon.rs:2561` `resolve_review_dispatch`, at `:2580-2583`. It passed the derived map's REVIEW entry as review's cross-family candidate (`:2602`).
   - The recommender ranks REVIEW by orchestration strength first and cost second (`recommend.rs:355` `review_rank`, used at `:524`). With an `ANTHROPIC_API_KEY` present, that entry is `claude-fable-5`, which is also the most expensive model in the knowledge base.
   - `review_gate.rs:847-849` (X-cost-2, step 2) escalates review to that candidate even when a `Role::Hard` pack pin exists. It does not need `PERMAGENT_DELEGATE_ALLOW_ESCALATION`. So the DeepSeek HARD pin that review maps through (`packs.rs:273`) was skipped on purpose, but the escalation landed on the frontier model.
   - `after_turn.rs:586` made the same prediction with `derived.clone()`.
   - Delegates for other roles already respected pins and the session model (`delegate.rs` `decide_delegate_model`).
2. **Receipts.** Only `orchestrator.rs:8041-8047` (`dispatch_goal_fn`) ever incremented `cost_routing`. No delegate spawn path (`summon.rs:1449` sync, `:3056` async, and the `delegate_many` fan-out closure) counted or audited its child.

## Changes

- `cost_router/reviewer_pick.rs`: new `cheapest_cross_family_reviewer(author, available, changed_lines)`. It runs `select_reviewer` with the configured and derived steps skipped, so it returns the cheapest different-family model that clears the size-scaled floor (0.60 for small diffs, 0.75 for large ones).
  - `summon::resolve_review_dispatch` now uses it as the cross-family candidate, with `after_turn::changed_lines` made `pub(crate)`. It returns a `ReviewDispatchResolution` that carries the pin and the candidate for the audit.
  - `after_turn::ReviewerMandate::assess` uses the same candidate.
  - Review still requires a cross-family reviewer. It now picks the cheapest capable one instead of the strongest.
- `cost_router/delegate.rs`: new pure `audit_delegate_dispatch(actual, pin, pin_exception, reference, nominal_cost)` → `DelegateCostAudit`. It uses `routing_matrix::nominal_call_cost`, the same yardstick the goal audit uses.
- `summon.rs`: `delegate_audit_targets` decides which pin and reference a child is judged against:
  - **Review with a dedicated pin** (`PERMAGENT_ROLE_REVIEW_*`, else `PERMAGENT_PACK_REVIEW_*`): that pin is both the pin and the reference.
  - **Review without one:** the pin is the HARD pack pin, and the cheapest cross-family reviewer is its one permitted exception. The reference is that cheapest reviewer, else the HARD pin, else the session pair.
  - **Other roles:** the pin is the role pin, else the pack pin. The reference is that pin, else the session pair.
  - `resolve_provider` audits the child's **effective** provider/model, so an explicit per-call `provider`/`model` is still counted and receipted. It attaches `cost_audit` to the routing receipt (`_meta.model_routing`).
  - `record_delegate_dispatch` counts the child at spawn in all three paths. In `delegate_many`, a child is counted when it actually spawns, not when it is resolved.
- `routing_receipt.rs`: new `record_delegate_dispatch(audit, provider)`. It increments `dispatches`, `cheapest_capable` and `pack_honored` as the audit says, plus `routed_local` or `routed_cloud`. `fallbacks` is left alone. **The line format and keys are unchanged**, so the driver needs no update.
- `summon.rs` `live_goal_delegate_refusal`: `delegate` and `delegate_many` are refused for a top-level coding coordinator (`is_top_level_coding_harness`) while one of its own goals (`goal_cards_created_by_session`) is `in_progress` or `review`.
  - The error reads "delegate refused: goal <id> is running; use goal_status or view_session to inspect it, or wait for review…". It is returned as the handler's `Err`, which becomes a tool error, and counted as `dag_enforcement bypass_attempts`.
  - It fails open if the store can't be read.
- Docs: `CODING_HARNESS_HELDOUT_QUALIFICATION.md` now describes how delegate spawns are counted (no key change).

## Gate filters

```
cargo test -p permagent --lib r63c_
cargo test -p permagent --lib cost_router::reviewer_pick
cargo test -p permagent --lib cost_router::delegate
cargo test -p permagent --lib cost_router::routing_receipt
cargo test -p permagent --lib agents::platform_extensions::summon
cargo test -p permagent --lib after_turn::tests::actual_route
```

The package is `permagent` (crates/goose). `permagent-eval` was not changed, so no eval filter is needed.

New tests:
- summon.rs: `r63c_a_delegate_under_pinned_packs_resolves_to_the_pin_and_a_reviewer_to_the_cheapest_cross_family`, `r63c_a_dedicated_review_pack_pin_is_honoured_by_the_reviewer_delegate`, `r63c_a_delegate_without_pins_picks_the_cheapest_capable_not_the_planner_model`, `r63c_a_spawned_delegate_child_is_counted_on_receipt_cost_routing` (serial receipts_global_registry), `r63c_live_goal_refusal_fires_only_for_in_progress_or_review`, `r63c_a_coordinator_delegate_while_its_goal_is_in_progress_is_refused` (end-to-end through `handle_delegate` and `handle_delegate_many`, serial), `r63c_interactive_sessions_and_workers_are_not_gated_by_a_live_goal` (serial).
- delegate.rs: `a_fable_reviewer_under_pinned_packs_is_neither_cheapest_nor_pack_honoured`, `an_unpinned_expensive_child_is_pack_honoured_but_not_cheapest`, `a_pinned_child_is_honoured_and_an_explicit_departure_is_not`.
- reviewer_pick.rs: `the_review_dispatch_candidate_is_the_cheapest_capable_cross_family_model_not_the_strongest`.
- routing_receipt.rs: `a_delegate_child_is_counted_as_a_dispatch_with_its_own_audit` (serial).

Pack env in tests goes through one `env_lock::lock_env` call per test. Its mutex is not re-entrant, so a nested second lock would deadlock. No OnceLock is set.

## Behaviour note (item 3)

The refusal would **not** have fired in s5. The goal was already complete when the coordinator delegated, and the delegate was the recipe-mandated independent reviewer, not an inspection of a live worker. The s5 cost is fixed by the routing change: the same call now routes to the cheapest capable cross-family model (haiku for a 1-line diff) instead of fable. It is also receipted, so a fable child would now read `dispatches=2 cheapest_capable=1 pack_honored=1` and fail the cost gate. The live-goal refusal covers the duplicate-work shape the brief describes. It also refuses a reviewer delegate while a goal is in Review, because the goal's own review gate owns that work. The ReviewerMandate cannot conflict, since it only fires on turns where the session itself made edits, and a coordinator can't edit.

## Risks

- **Receipt semantics widen.** Records whose coordinator delegates now show `dispatches` above the number of goal dispatches. A record that delegated an honest escalate no longer gets the R20a neutral vote, because `dispatches` is no longer 0.
- **Single-model tiers still use a cross-family reviewer.** On `dschat`, a reviewer delegate still calls Anthropic (now haiku or sonnet rather than fable) and counts as pack-honoured through the documented exception. If the tier should keep review entirely on DeepSeek, it needs `PERMAGENT_PACK_REVIEW_*`. That would record `reviewer_not_independent` and fail verification integrity, which is exactly why X-cost-2 exists.
- **Review floor size.** In the delegate path, `changed_lines` is `git diff --shortstat HEAD` in the coordinator's working directory. That is 0 after an auto-land, which applies the small-diff floor (0.60). A large landed change could therefore be reviewed by the cheap end of the ladder. The mandate prediction has the same lenient behaviour.
- **Extra discovery on reviewer delegates.** Each reviewer delegate without a dedicated pin now calls `discover_available_models_async`, a bounded Ollama probe, instead of the cached derived map.
- **Retry not counted separately.** The R36b reviewer retry on the author pair is not counted as a second dispatch; the audit reflects the first target.
- **Goal-level review gate not changed.** `orchestrator.rs:22708` still passes the derived REVIEW entry into `select_reviewer`, so the goal-level independent reviewer can still pick fable. That is outside this lane (R63b owns orchestrator.rs) and is a follow-up.
- **Resumed roadmaps are not gated.** The refusal only considers goals this session created (`created_by_session_id`).
