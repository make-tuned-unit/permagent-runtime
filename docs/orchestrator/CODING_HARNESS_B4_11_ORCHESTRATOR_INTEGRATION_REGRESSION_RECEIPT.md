# B4.11 receipt — orchestrator integration regressions

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This bounded child repaired four test-fixture regressions in
`agents/platform_extensions/orchestrator.rs`:

- three verify-loop fixtures now create a durable Spectral session and bind its
  `budget_task.v1` identity before invoking the budget gate;
- the task-ceiling fixture now inserts its spend row with the active task ID
  and explicitly proves the ledger attribution before evaluating the verdict.

The production `spend_snapshot` fail-closed behavior for unknown sessions and
missing task identity was unchanged. No provider calls were made.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Four isolated regressions | passed | `verify_loop_single_model_goal_parks_never_swaps`, `verify_loop_below_threshold_keeps_fixing_no_park`, `verify_loop_corroborated_signals_fire_below_raw_threshold`, and `task_over_its_ceiling_is_gated_while_the_session_is_fine`: 1 passed each. |
| Full orchestrator module | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib 'agents::platform_extensions::orchestrator::tests::' -- --nocapture`: 124 passed, 0 failed. |
| Shared accounting gates | passed | Six `accounted_fast_*` fixtures and `primary_stream_reservation_refuses_paid_dispatch_without_a_task`: 7 passed, 0 failed. |
| Integrated library check | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib` completed successfully. |
| Diff/resource gate | passed | `git diff --check` passed; free disk remained above the 8 GiB verification floor. |

B4 remains active. This receipt closes only the orchestrator integration
regression gate and does not alter unrelated routing, provider transport, UI,
or voice work.
