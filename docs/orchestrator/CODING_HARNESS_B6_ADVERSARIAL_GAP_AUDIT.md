# B6 adversarial-integration gap audit

**Date:** 2026-09-04 (America/Halifax)  
**Status:** historical gap audit; B6 is not passed  

## September 6 source reconciliation

The missing-fixture statements below describe the original audit, not the
current implementation. Root verified six `b6_` tests now exist in
`crates/goose/src/session/session_manager.rs`: bounded retry storm, restart
reconciliation, compaction/continuation identity, mixed billing, duplicate
child completion, and atomic claim race. Do not reimplement these from this
historical plan. The next gate is coordinated execution/review of the existing
tests and integrated B6.7 evidence; source presence alone does not pass B6.
**Authority:** existing Spectral sessions, reservations, and cost ledger only

No production code, B5 projection files, provider transports, GUI code, or
external provider was used or changed. This audit maps the B6 contract in
`CODING_HARNESS_TASK_BUDGET_DAG_4.md` to the deterministic tests already in
the repository and identifies the smallest no-model fixture sequence needed to
close each gap.

## Decision summary

The accounting primitives are strong, but B6 is not yet an integration pass.
The current suite proves several properties in isolation—idempotent ledger
writes, durable task identity, fail-closed unknown usage, and atomic sibling
reservation scope—but it does not yet replay the adversarial events through
one task containing compaction, restart, child completion, mixed billing
classes, and a selection-to-claim race.

| B6 fault | Evidence | Status | Remaining proof |
|---|---|---|---|
| Retry storm | `providers::retry::tests::retry_decision_table`; `billing_error_costs_zero_retries`; `remote_network_errors_are_still_retried`; `max_physical_attempts_covers_rate_limit_retry_envelope`; `agents::reply_parts::tests::accounted_fast_paid_failure_does_not_dispatch_the_regular_fallback` | **Partial** | One fake paid invocation must exercise bounded transient retries while every physical attempt has distinct reservation/invocation attribution and the storm terminates without an unbounded fallback loop. |
| Compacted continuation | `session::session_manager::tests::budget_task_identity_survives_resume_and_history_replacement`; `context_mgmt::tests::code_and_diffs_survive_compaction_byte_identical`; `test_progressive_removal_on_context_exceeded` | **Partial** | Join compaction, a continued turn, and accounting in one fixture; prove the same durable task survives and no compacted/replayed attempt creates a duplicate ledger row. |
| Daemon death/restart | `usage_rollup_is_idempotent_across_concurrency_and_restart`; `reservation_is_idempotent_across_concurrency_restart_and_settlement`; `existing_current_schema_repairs_missing_cost_reservations_on_restart`; `budget_task_identity_survives_resume_and_history_replacement` | **Partial** | Simulate death after reservation and before terminal usage, reopen the manager, reconcile the hold as unknown, and prove restart cannot dispatch the same invocation twice or reset the task. |
| Duplicate child completion | `orchestrator::tests::completion_noops_if_card_not_in_progress`; `nudge_does_not_double_dispatch_an_already_claimed_goal`; `session_manager::tests::usage_rollup_is_idempotent_across_concurrency_and_restart` | **Partial** | Deliver the same child completion concurrently twice, with the same durable child/invocation ID, and assert one state transition, one decision/effect, and one ledger row. Current tests cover terminal-state and dispatch idempotence separately, not the physical child callback plus accounting boundary. |
| Mixed local/subscription/API workers | `council::debate::tests::local_and_subscription_calls_skip_holds_but_keep_tier_attribution`; `session_manager::tests::test_cost_ledger_append_and_rollup`; `agents::reply_parts::tests::accounted_fast_local_failure_retries_the_full_model_as_a_separate_attempt`; `live_caller_reserves_before_dispatch_and_settles_exact_usage` | **Partial** | Interleave all three billing classes under one task and assert local/subscription rows remain attributed, only paid API consumes a hold, and the combined task/session cap is exact. |
| Missing/unknown usage | `agents::reply_parts::tests::accounted_fast_missing_usage_keeps_the_paid_hold_unknown`; `accounted_fast_paid_failure_does_not_dispatch_the_regular_fallback`; `accounted_fast_cancelled_paid_dispatch_keeps_the_hold_unknown`; `accounted_fast_refuses_paid_dispatch_without_a_durable_task`; `council::debate::tests::live_caller_marks_provider_error_and_missing_usage_unknown`; `live_caller_marks_timeout_and_cancellation_unknown` | **Proven at seam level** | Add one restart/reconciliation assertion so an unknown hold remains budget-consuming after manager recreation. B6 should not call this fully integrated until that persistence path is exercised. |
| Cap crossed between selection and atomic claim | `session_manager::tests::reservation_scope_is_root_lineage_and_sibling_holds_block`; `reservation_is_idempotent_across_concurrency_restart_and_settlement`; `reservation_requires_task_and_expiry_becomes_unknown`; `orchestrator::tests::task_over_its_ceiling_is_gated_while_the_session_is_fine`; `council::debate::tests::gate_refusal_never_dispatches_or_creates_a_hold` | **Partial** | Two distinct invocations must pass the same preselection snapshot concurrently and race the atomic reservation. With a one-call hard cap, exactly one may be granted; the other must be refused before fake provider dispatch, with no duplicate charge. |

“Partial” means the named tests are useful evidence for the underlying seam,
not that the B6 adversarial contract is satisfied. The only seam-level proof
currently complete is unknown-usage handling: post-dispatch error, missing
usage, cancellation, and pre-dispatch paid refusal all preserve the hold or
refuse before transport. The integrated restart and race cases are still
missing.

## Authoritative seams reviewed

- `SessionManager::reserve_provider_invocation` is the atomic authorization
  boundary. It validates task identity and reservation bounds, accounts for
  settled plus active holds, and uses the Spectral transaction.
- `SessionManager::settle_provider_invocation` is the exactly-once terminal
  usage/ledger boundary. `release_provider_invocation` is legal only for a
  proven pre-dispatch failure; `mark_provider_invocation_unknown` preserves a
  post-dispatch or missing-usage hold.
- `AccountedFastCompletion` and the primary stream wrapper create distinct
  physical invocation IDs and route paid refusal, settlement, error, missing
  usage, and cancellation through the same boundary.
- `spend_snapshot` reads task identity and recursive session lineage from
  Spectral and fails closed for unknown or unattributed sessions.
- Child sessions inherit the parent task identity through
  `create_session_with_parent`; this is the only lineage source the B6 fixtures
  should use.
- Retry policy is bounded by `RetryConfig::max_physical_attempts`; B6 must
  measure actual fake-provider calls rather than infer them from retry text.

## Smallest sequential B6 fixture DAG

The nodes below are deliberately sequential: later fixtures depend on the
same invariants but use fresh temporary Spectral databases, so a failing node
cannot contaminate another node's ledger. Each node is no-model and must use a
counting fake provider or direct reservation API. No paid provider call is
permitted.

```text
B6.0 contract/cap freeze
  -> B6.1 bounded retry storm
  -> B6.2 compaction + continuation
  -> B6.3 death/restart reconciliation
  -> B6.4 duplicate child completion
  -> B6.5 mixed billing classes
  -> B6.6 selection/claim cap race
  -> B6.7 integrated adversarial receipt
```

### B6.0 — Freeze failure semantics and fixture caps

Record the following invariants before adding tests:

1. Unknown task/session identity refuses paid dispatch.
2. A post-dispatch error, cancellation, timeout, or missing usage is
   `unknown`, not released and not retried as an unmetered fallback.
3. A retry is a new physical invocation ID and reservation; a duplicate
   delivery of one ID is a no-op.
4. Local/subscription calls are ledger-attributed but do not create paid
   reservations.
5. Authorization uses the atomic reservation transaction, not a prior budget
   snapshot alone.

### B6.1 — Bounded retry storm

Use a fake paid provider that returns transient errors for every call. Use a
test-only `RetryConfig` with zero backoff and a small retry count (for example,
`max_retries = 2`, rate-limit floor `0ms`) so the test is deterministic and
fast. Assert:

- physical provider calls are at most `max_physical_attempts()`;
- each attempt has a distinct invocation/reservation ID;
- the terminal failure leaves the final post-dispatch hold unknown (or the
  exact settled terminal usage if the fake returns usage);
- no regular/full fallback is dispatched after a paid error; and
- no retry or escalation continues after the cap/terminal classification.

This is the missing bridge between `retry_decision_table` and the accounting
fixtures. It must not sleep for the production 20-second rate-limit floor.

### B6.2 — Compacted continuation

Create one durable user session, begin one budget task, write code-bearing
messages, run the existing compaction seam with a counting local fake, replace
the conversation, then reopen/read the same session and perform one
continuation. Assert:

- the task ID before compaction, after replacement, and after continuation is
  identical;
- protected code/diff content is byte-identical;
- each physical completion has one ledger row; and
- a replay of the terminal usage for either invocation changes no totals.

This should reuse `budget_task_identity_survives_resume_and_history_replacement`
and `code_and_diffs_survive_compaction_byte_identical` as the fixture halves,
not create a second compaction or memory store.

### B6.3 — Daemon death/restart reconciliation

Reserve a paid invocation in a temporary Spectral database, stop/drop the
manager before provider terminal usage, and reopen it. Set or advance the
lease through the existing test seam, then assert the row becomes unknown and
still consumes the cap. A second attempt with the same invocation ID must not
dispatch; a new invocation must be refused when the unknown hold crosses the
cap. Also assert the task ID and session lineage survive reopening.

This is a bounded extension of
`reservation_is_idempotent_across_concurrency_restart_and_settlement` and
`reservation_requires_task_and_expiry_becomes_unknown`. It must model the
crash boundary without spawning a daemon or calling a provider.

### B6.4 — Duplicate child completion

Create a parent and child via `create_session_with_parent`, bind the child to
the parent's task, and submit the same child completion/usage callback twice
concurrently. Assert one durable transition/effect, one usage ledger row, one
task roll-up, and no duplicate unblock/review decision. A second callback after
restart must remain a no-op.

This closes the gap between the existing card-state guards and the accounting
idempotency test.

### B6.5 — Mixed billing classes

Under one task, run three counting fakes in a deterministic order:

1. local-free worker;
2. subscription worker; and
3. paid API worker with authoritative usage.

Assert three attributed ledger rows, one paid reservation only, exact task and
recursive session totals, and no relabelling of local/subscription spend as
paid API. Repeat the local/subscription pair as child sessions to prove parent
lineage does not create a paid hold.

### B6.6 — Selection/claim cap race

Use one parent task with a hard cap equal to one bound. Start two concurrent
reservation attempts with different invocation IDs after both have observed the
same preselection snapshot. Assert exactly one `Granted`, exactly one
`Refused`/fail-closed result, one fake provider dispatch at most, and one
settled or unknown row. The test must not use a prior pure `budget_verdict` as
authorization; only the atomic reservation result may permit dispatch.

The existing sibling-scope test proves the shape of the refusal, but not two
distinct concurrent claims racing the same remaining allowance.

### B6.7 — Integrated gate

Run only the six B6 fixture filters, the existing B4 accounting filters, and
the static dispatch inventory. The gate is passed only if all fixture ledgers
reconcile after any restart and no fake provider count exceeds its cap. Record
the exact database, reservation, invocation, and task assertions in a receipt.

## Cap/attempt budget table

| Fixture | Test-only cap/rework | Required bounded result |
|---|---|---|
| Retry storm | 2 configured retries, 0ms test backoff; physical envelope asserted from `max_physical_attempts()` | finite calls, distinct attempts, terminal unknown/settled hold |
| Compaction continuation | one compaction + one continuation; one terminal usage per physical ID | same task, no duplicate ledger row |
| Restart | one pending reservation, one lease-expiry reconciliation, one new-claim attempt | unknown remains consuming; no replay dispatch |
| Duplicate child | exactly two concurrent identical callbacks | one effect, one row, one roll-up |
| Mixed billing | exactly 3 workers: local, subscription, paid API | 3 ledger rows; 1 paid hold |
| Cap race | task/session hard cap = one reservation bound; exactly 2 distinct claims | at most 1 grant/dispatch |

These caps are test controls, not production-policy changes. Production retry,
budget, and escalation values remain unchanged.

## Promotion rule

B6 should remain planned/partial until B6.2, B6.3, B6.4, and B6.6 exist as
deterministic no-model fixtures. The current evidence is enough to proceed
without changing accounting architecture, but not enough to claim that a
compaction/restart/duplicate/race sequence cannot duplicate work, reset a task,
or authorize paid work after a cap is crossed.
