# Lane R65a report: verified-goal review UNCERTAIN, and the expert planner silently replaced

Base: M49 `62af57bc`. Evidence: r65 s1 (16/19), the three failing run directories under
`laneM17/dschat-r65/s1-runs/` (`cli-argparse-log-filter`, `env-broken-test-makefile`,
`refactor-shipping-quote-signature`). Edit-only lane: no cargo was run. Every touched file passes
`rustfmt --edition 2021 --check`.

Commits (branch `lane/R65a`):

1. `2bda1064` Read reviewer verdicts robustly, show verified goals' bound verify run, escalate one UNCERTAIN
2. `e161a4f9` Size the expert planner's reservation and fall back by the planner ranking
3. `ecff8163` Test the verified-goal review escalation and the planner ranking fallback (plus this report)

## FAILURE 1: independent review before auto-land returned UNCERTAIN, and all three goals parked

### What the artifacts show

- All three cards: `auto_land_review.outcome = refused`,
  `refusal = "review verdict is UNCERTAIN — default-to-reject; ..."`. `reviewer_assignment` picked
  `minimax/MiniMax-M2.5`. `cost_ledger` has one MiniMax row per run (refactor: 2,122 in, 609 out).
- Reviewer responses are not saved anywhere: `llm_request.*.jsonl` holds requests only, and nothing
  in `messages` or the CLI log has them. So the diagnosis can't tell a real UNCERTAIN apart from an
  unreadable response. Both causes are real, and both are fixed.
- The refactor request (`llm_request.0.jsonl`) is the salvage-shaped prompt: two criteria framed as
  `UNVERIFIED (no detected check command in this repo) — kept for human review: ...`, and a
  `BEGIN_VERIFY_OUTPUT` that says "its output is not shown here". The system prompt says
  "Default to reject. If you cannot confidently sign off, the verdict is UNCERTAIN". The diff does
  update all four call sites. There is no `BEGIN_CALL_SITES` section (see Risks).

### Diagnosis (file:line at base `62af57bc`)

- Parser: `crates/goose/src/cost_router/review_gate.rs:318-346` (`parse_review`). It only reads a
  line that starts exactly with `VERDICT:` (`trimmed.strip_prefix("VERDICT:")`, line 322), followed
  by an exact token (`Verdict::parse_token`, lines 90-97). It can't read `**VERDICT:** APPROVE`,
  `- VERDICT: APPROVE`, `VERDICT: **APPROVE**`, `VERDICT: APPROVE.`, or a heading form. Any of these,
  or a missing line, becomes `Verdict::Uncertain` (line 342) with no log line. Reasoning text in
  `<think>` blocks isn't ignored: a draft `VERDICT: UNCERTAIN` inside one wins as the first verdict.
  UNCERTAIN then becomes `GateDecision::Block` "default-to-reject" (lines 598-607).
- Wrong framing for a verified goal: `crates/goose/src/agents/platform_extensions/orchestrator.rs`
  - `independent_review_before_auto_land` (16777) calls
    `review_goal_diff_with_cross_family_reviewer` (16821, 23537) with
    `VERIFIED_GOAL_VERIFY_OUTPUT_LEAD` (23429). That lead says the verify output "is not shown here".
    The verify run the worker recorded is never passed in, even though
    `auto_land_headless_goal` already required a genuine one (16541).
  - The task spec is `collect_acceptance_criteria(...).join("\n")` (23658-23662). So the
    planner's no-check marker "UNVERIFIED ... kept for human review" reaches the reviewer as if it
    were an unmet requirement.
  - The only non-approval handling is `Block | Escalate | ReviewUnavailable => Refused`
    (23749-23753). There is no second opinion under the objective-escalation ruling.

### Changes

- `review_gate.rs`: `parse_review` / `parse_review_with_problem`
  - Removes reasoning blocks: `<think>`, `<thinking>`, `<reasoning>`, and an orphan closing tag. An
    unclosed block drops the rest.
  - Strips markdown around a labeled line: blockquote, a `-`/`*`/`+` list marker followed by
    whitespace, a `#` heading, emphasis and inline code. Label case can be upper, title or lower.
  - Reads a decorated token (`**APPROVE**`, `` `UNCERTAIN` ``, `APPROVE.`, `REQUEST CHANGES`).
  - Still refuses the echoed grammar (`APPROVE|REQUEST_CHANGES|UNCERTAIN`), longer words
    (`APPROVED`), and quoted diff lines (`-VERDICT: APPROVE`). The first valid verdict still wins.
  - When no verdict can be read, it logs `tracing::warn!` (target `permagent::cost_router::review_gate`)
    with the problem (`no VERDICT line` / `no recognised VERDICT token`) and the first 2,000
    characters of the raw response, and the result stays UNCERTAIN.
  - New helper: `with_review_data_section` adds a fenced `BEGIN_<NAME>` section before the closing
    instruction.
- `orchestrator.rs`, verified-goal review
  - New `GoalReviewKind::{Salvage, Verified}`. It replaces the lead string.
  - Salvage is unchanged: same prompt, no escalation, `.outcome` taken.
  - Verified gets `verified_goal_review_task_spec`. Criteria carrying the exact no-check marker are
    listed without it, under "no detected check command ... that is not a finding that they are
    unmet — judge each one from the diff". Other criteria, including preservation-reclassified
    UNVERIFIED ones, are listed as written.
  - Verified also gets `goal_review_verification`, which is included only when every link holds:
    - `dispatch_evidence.head_commit` exists.
    - The worktree `HEAD` still matches it (`sha_matches`).
    - The worker session's last genuine verify pass has no file-mutating tool call after it
      (`credited_verification_after_last_edit`, using `after_turn::is_mutating_tool_name`).
  - When included, it is a `BEGIN_VERIFICATION` section (head, command, `verdict: pass`, output
    tail of the last 2,000 characters) with `VERIFIED_GOAL_WITH_EVIDENCE_LEAD`. That lead tells the
    reviewer to judge whether the command actually exercises the change. Otherwise the old lead is
    kept.
  - Unchanged: the system prompt (data sections are data, TEST_INTEGRITY, CALL_SITE_COMPLETENESS)
    and the E7 `gate_decision_with_call_sites`.
- Escalation
  - `recommend::escalate_reviewer_after_uncertain` (pure) only considers cross-family models that
    rank above the reviewer that was UNCERTAIN on the knowledge-base strength scale. It never picks
    that reviewer again, nor the first assignment's pick.
  - Below COMPLEX it uses `assign_reviewer_at_least` at the next floor tier, so the cheapest
    stronger model that clears the next floor wins.
  - At COMPLEX, or when nothing clears the next floor, it takes the strongest cross-family model
    above that reviewer. If nothing is above it, there is no pick and `why` says so.
  - Orchestrator `finish_goal_review_with_escalation` handles the result:
    - Only a first-round `ReviewRound::Uncertain` on a Verified review escalates. An unreadable
      response counts as Uncertain.
    - It escalates exactly once.
    - APPROVE lands, a second UNCERTAIN parks, REQUEST_CHANGES parks with its findings, and no
      escalation reviewer parks.
  - `escalated_goal_review` runs the escalation through `select_reviewer` (cross-family) and the
    same `reviewer_spend_gate`, on the same prompt and E7 call sites. A provider failure opens the
    circuit and parks.
  - `auto_land_review` now also records `reviews` (every round: reviewer, outcome,
    reason/checked), `escalation` (from, to, the escalation assignment with candidates and why) and
    `verification_shown` (command, head, or null).

## FAILURE 2: COMPLEX plan routed to Fable, answered by the DeepSeek session model

### Exact cause (from the eval child's own log)

Two runs planned COMPLEX (`cli-argparse-log-filter` and `refactor-shipping-quote-signature`; both
routed to `anthropic/claude-fable-5` and both recorded as answered by
`custom_deepseek/deepseek-flash`). The makefile run planned ROUTINE and was correctly answered by
`minimax/MiniMax-M2.5`, which is why none of the three has anthropic ledger rows. Both COMPLEX runs'
CLI logs (`data/logs/cli/<date>/20260917_*.log`, 15:14:38 and 15:20:50) have this line:

```
Planner decomposition failed (LLM decomposition failed: provider budget is unknown; refusing call:
cannot authorize provider call: maximum output tokens are required for a paid reservation);
escalating to default provider
```

This is not a spend cap, a credential problem or a circuit breaker. It is a reservation-sizing bug
in the planner provider's model config.

### Diagnosis (file:line at base `62af57bc`)

1. `resolve_decomposition_provider` (`orchestrator.rs:5049`) builds the planner with
   `providers::create_with_named_model` (`providers/init.rs:195-202`). That is `ModelConfig::new`
   with no output bound.
2. `ProviderEntry::create` then normalizes it (`providers/provider_registry.rs:55-70`).
   `with_canonical_limits` (`model.rs:134-156`) finds no canonical row for `claude-fable-5`
   (`canonical_models.json` has `claude-opus-4.8` and `claude-sonnet-5`, not Fable). Only
   `context_limit` gets filled, from `known_models`. `max_tokens` stays `None`.
3. `run_decomposition` (`orchestrator.rs:13220`) calls `AccountedFastCompletion::complete_accounted`
   (`agents/reply_parts.rs:2080`). That goes to `complete_one` → `reserve_provider_invocation_for_model`
   (`reply_parts.rs:1797-1823`) → `plan_reservation_bound` → `calculate_bound`
   (`cost_router/reservation.rs:265-267`), which returns `ReservationBoundError::MissingMaxOutput`.
   `reply_parts.rs:2271` wraps it as "provider budget is unknown; refusing call".
4. The escalation arm (`orchestrator.rs:11274-11300`) logs a warn line and runs the session provider
   (`self.get_provider()`, `custom_deepseek/deepseek-chat`). Its response names the alias the
   ledger shows, so `planner_physical = custom_deepseek/deepseek-flash`. Nothing else was recorded.
5. Why r64 was different: r64 s5 switched `run_decomposition` from `complete_fast_accounted` to
   `complete_accounted`. Before that, the Anthropic provider's fast config (`with_fast` →
   `with_canonical_limits` for `claude-haiku-4-5`, which has a canonical output limit) was used, so
   M48 had Haiku rows. After the switch, the Fable config itself had to authorize a paid hold, and
   it couldn't.
6. Second trap, latent behind the first: with only an output bound, Fable's hold would be sized from
   its whole 1M-token window. That is about $10.64 per attempt at $10/$50, which is above the eval's
   $5 task gate and would have been refused next. So the hold must be sized from the request.

### Changes

- `bounded_planner_model_config` (pure) sets `max_tokens` to the lower of any existing bound and
  `PLANNER_MAX_OUTPUT_TOKENS` (16,000). With `bound_context`, it caps `context_limit` at
  request bytes (system + user; one token per byte at most) + 2 × the output bound (the answer, plus
  the stricter retry that resends it) + 2,048 framing tokens. It never goes above the model's own
  window.
- `resolve_decomposition_provider` now returns `Result<Arc<dyn Provider>, String>`:
  - It builds the config with `ModelConfig::new(..).with_canonical_limits(..)` plus the output
    bound.
  - For a chargeable provider it rebuilds on the request-bounded context (a local provider keeps its
    window, since its `num_ctx` depends on it).
  - A circuit-open or creation failure is an `Err` with a reason, not a silent `None`.
  - Fable planning an ~8 KB request now holds about $1.06 per attempt, not $10.64 or a refusal.
- `capability_directory::resolve_planner_fallback` (pure) is the same ranking with failed routes
  removed: `planner_excluding` (strongest-first) for an uncapped COMPLEX plan, and
  `routine_planner_excluding` (cheapest capable, or the coordinator) otherwise. Pins are not
  consulted after a failure. An exhausted ranking returns `SessionFallback` naming every planner
  tried. `planner()` and `routine_planner()` delegate with an empty exclusion list, so their
  behaviour is unchanged.
- `walk_planner_ranking` (generic, testable) tries a route and, on failure, resolves the next one by
  that ranking. It tries at most `PLANNER_FALLBACK_MAX_ROUTES` (3) routes, then the session
  coordinator.
  - Each failure becomes a `PlannerFallbackHop {from, to, reason, class}`. `class` is one of
    `spend_refused`, `reservation_unauthorized`, `provider_unavailable`, `unparseable` or
    `provider_error`.
  - Each hop is traced with `tracing::warn!`. The message still starts
    `Planner decomposition failed (...)`, so the eval's credit-exhaustion detection still matches.
- The decomposition records `planner_fallback: {from, to, reason, class, hops}` on the proposal
  proof. It was added to `ROADMAP_PLANNING_PROOF_KEYS`, so it reaches `roadmap_planning` on the
  project and on every card.
- `unrecorded_planner_substitution` guard: if the planner that answered is not the intended route
  and no fallback was recorded, it records `class: unrecorded_substitution` with the reason and logs
  `tracing::error!`. A COMPLEX plan can't be answered by another model without a recorded reason.

### Eval-environment note

Anthropic credentials were not the cause: the failure came before any network call, at reservation
sizing. If Anthropic credentials were missing in an eval child, the fix still handles it:
provider creation or the call fails with a reason, the ranking moves to the next planner, and
`planner_fallback.class = provider_unavailable` is recorded.

## Tests (item 3)

- `cost_router::review_gate::tests`:
  - `parse_reads_a_verdict_inside_a_markdown_fence_and_after_prose`
  - `parse_tolerates_markdown_decoration_around_the_labeled_lines`
  - `parse_ignores_reasoning_blocks`
  - `parse_refuses_echoed_grammar_longer_words_and_quoted_diff_lines`
  - `a_missing_verdict_is_logged_with_the_raw_response_and_read_as_uncertain`: thread-scoped
    `tracing::subscriber::with_default` capture, no global subscriber.
  - `a_data_section_is_added_before_the_closing_instruction`
- `cost_router::recommend::tests`:
  - `uncertain_moderate_review_escalates_to_the_cheapest_stronger_reviewer_at_the_next_floor`:
    MiniMax-M2.5 → glm-5.2, not the frontier.
  - `uncertain_complex_review_escalates_to_the_strongest_reviewer_above_it_or_none`
- `cost_router::capability_directory::tests::planner_fallback_walks_the_same_ranking_past_failed_planners`
- `agents::platform_extensions::orchestrator::r65a_tests::*`:
  - `r65a_bound_verification_is_the_last_genuine_pass_with_no_edit_after_it`
  - `r65a_verified_goal_task_spec_does_not_frame_no_check_criteria_as_unmet`
  - `r65a_verified_goal_review_prompt_carries_the_bound_verification_section`: the section sits
    before the closing instruction, call sites are still present, and the system prompt keeps the
    data rule, TEST_INTEGRITY and CALL_SITE_COMPLETENESS.
  - `r65a_review_round_reads_presented_verdicts_and_a_missing_verdict_is_uncertain`
  - `r65a_uncertain_escalates_once_to_the_next_ranked_reviewer_and_an_approve_lands`
  - `r65a_request_changes_parks_without_escalating`: also covers that salvage never escalates.
  - `r65a_a_second_uncertain_or_no_stronger_reviewer_parks`
  - `r65a_bounded_planner_config_lets_a_paid_reservation_authorize_the_expert_planner`: reproduces
    `MissingMaxOutput`, shows the whole-window hold is above $3, and the request-bounded hold is
    below $3.
  - `r65a_planner_failure_class_names_spend_reservation_and_provider_failures`
  - `r65a_spend_refused_expert_planner_falls_back_by_ranking_and_records_it`: Fable is refused →
    Opus 4.8. The record reaches `roadmap_planning`.
  - `r65a_an_exhausted_ranking_reaches_the_session_with_every_hop_recorded`
  - `r65a_a_plan_answered_by_another_model_always_carries_a_recorded_reason`
- Updated consumer test: `resolve_decomposition_provider_never_returns_a_circuit_open_provider` now
  expects `Err`.

## Gate filters (package `permagent`, crate dir `crates/goose`)

```
cargo test -p permagent --lib r65a_
cargo test -p permagent --lib cost_router::review_gate::tests
cargo test -p permagent --lib cost_router::recommend::tests
cargo test -p permagent --lib cost_router::capability_directory::tests
cargo test -p permagent --lib n4_
cargo test -p permagent --lib resolve_decomposition_provider_never_returns_a_circuit_open_provider
cargo test -p permagent --lib salvage_review
cargo test -p permagent --lib dispatch_cross_family_reviewer
cargo test -p permagent --lib reviewer_assignment_is_recorded_on_the_goal_card
cargo test -p permagent --lib headless_auto_land
cargo test -p permagent --lib verify_evidence
cargo clippy -p permagent --all-targets -- -D warnings
```

Eval re-check: run r65 s1 on the three tasks, then:

- `roadmap_planning.planner_physical` on the refactor task should be `anthropic/...`, or
  `planner_fallback` should be present with a class.
- `cost_ledger` should have anthropic rows.
- `auto_land_review.reviews` / `verification_shown` should be populated.
- The CLI log should show a `permagent::cost_router::review_gate` warning with the raw response
  whenever a verdict is unreadable.

## Risks

- No cargo run in this lane. The code was checked by reading only, and rustfmt passes. The likeliest
  compile or clippy issues are closure capture in `walk_planner_ranking`'s production call (a `move`
  closure over `&self` / refs returning `async move`), and `clippy::type_complexity` on its `where`
  clause.
- `PLANNER_MAX_OUTPUT_TOKENS = 16_000` caps what the planner can emit. With Anthropic adaptive
  thinking, thinking counts inside `max_tokens`, so a very long thinking pass could truncate the
  JSON. Truncated output is `unparseable`, and the fallback walks the ranking, so the failure is
  recorded, not silent.
- The request-bounded context assumes one token per UTF-8 byte at most, plus 2,048 framing tokens.
  That holds for byte-level tokenizers. The `Enabled` thinking type adds `budget_tokens` on top of
  `max_tokens` in the Anthropic payload (`formats/anthropic.rs:705`). This under-sizing already
  existed for every reservation and was not introduced here.
- A reviewer's verdict is now read through more presentation, which widens what counts as an
  approval. Mitigations: a glued `-`/`+` (diff line) is not stripped, echoed grammar and longer words
  are refused, the first verdict still wins, and the anti-rubber-stamp and E7 gates are unchanged.
- The escalation reviewer's spend check uses the same `budget_verdict(0.0, 0.0, cfg)` as the first
  review (the existing behaviour: no live tracker for a finished worker session). The reservation
  layer still enforces the real caps on the call.
- Not fixed here, but found: E7's `detect_signature_changes` (`call_site_completeness.rs:159`) reads
  definitions one line at a time. A multi-line TypeScript signature (the refactor diff puts
  `quoteShipping(` on its own line) is not detected. So the refactor review had no
  `BEGIN_CALL_SITES`, and complexity was assessed MODERATE instead of COMPLEX. That is worth a
  follow-up lane.
- The refactor worker's "passing" verify was
  `node --experimental-strip-types --check-only ... || true; echo ...; node -e 'console.log("no runner")'`.
  It was not flagged vacuous. With this lane the reviewer now sees that command in
  `BEGIN_VERIFICATION` and is told to judge whether it exercises the change. It may reasonably
  still say UNCERTAIN or REQUEST_CHANGES, and that is the honest outcome.
