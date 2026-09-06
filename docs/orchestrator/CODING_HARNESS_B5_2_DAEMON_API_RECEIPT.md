# B5.2 receipt — daemon budget projection API

Date: 2026-09-05 (America/Halifax)

Status: implemented; promotion gate pending daemon test-runner capacity.

## Contract delivered

`HarnessRunView` now carries the canonical, read-time `budget` projection
(`budget-projection.v1`) for update, active, and history responses. It is
recomputed through `SessionManager::budget_projection` using the current
`BudgetConfig`; the existing `tokens` and `spendUsd` fields remain only as
legacy compatibility fields and are never used as budget authority.

Projection/query failures and unusable unknown projections for a bound session
return an explicit service-unavailable response. An unbound task remains a
successful, explicit partial projection with unknown task scope rather than a
fabricated zero. Active-list hydration failures also return service-unavailable
instead of an empty successful list. Durable history hydration and the existing
terminal monotonicity/45-second registry TTL remain unchanged.

## Verification

```text
CARGO_INCREMENTAL=0 cargo check -p permagent-daemon
passed

CARGO_INCREMENTAL=0 cargo test -p permagent-daemon --lib --no-run
passed (daemon test executable compiled)

CARGO_INCREMENTAL=0 cargo test -p permagent --lib session::budget_projection -- --nocapture
13 passed; 0 failed

Existing named recovery gates reused (no claims of route-runtime execution):
`harness_snapshot_store_is_idempotent_private_and_restart_safe`,
`terminal_hydration_overrides_late_active_heartbeat`,
`harness_terminal_state_cannot_regress_to_running`, and
`harness_terminal_result_cannot_be_overwritten_or_attempts_decrease`.

New deterministic route/helper coverage:
`harness_run_view_exposes_versioned_budget_and_legacy_compatibility_fields`,
`projection_unavailable_is_explicit_service_unavailable`,
`durable_read_failure_is_explicit_service_unavailable`,
`bound_unknown_projection_is_not_presented_as_a_zero`, and
`stale_active_runs_expire_from_live_view_but_remain_addressable_for_history`
(all compiled into the daemon test target; runtime execution is covered by the
SIGKILL limitation below).

git diff --check -- crates/goose-server/src/routes/coding_session.rs
passed
```

The focused daemon test executable was invoked once with
`CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1` and the route test filter, but
the host terminated the process with SIGKILL during test startup after linking
(no assertion failure was reported). This is recorded as a verification-gate
capacity limitation, not promoted as a passing runtime route test. A direct
binary retry was not treated as evidence because it lacks the Cargo-provided
`@rpath/libsherpa-onnx-c-api.dylib` environment.

No provider calls, CLI/UI changes, new stores, or projection/session-manager
internals were added by B5.2.
