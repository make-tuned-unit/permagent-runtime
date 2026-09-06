# B5.0/B5.1 receipt — canonical budget projection

Date: 2026-09-04 (America/Halifax)

Status: passed (B5.1 independently reopened and six review findings fixed).
This child implements only the projection contract and the
canonical SessionManager/Spectral read seam. It does not modify daemon/API,
CLI, UI, or provider paths.

## Contract

`BudgetProjection` is versioned as `budget-projection.v1` and is recomputed on
every read from existing Spectral sources. It has separate task and root
session scopes, each containing:

- current-source soft/gate/hard cap triplets;
- settled, pending-held, unknown, effective-used, and clamped remaining USD;
- an explicit `ok`, `soft`, `gate`, `hard`, or `unknown` band;
- `complete`, `partial`, or `unknown` completeness plus bounded errors.

The projection also carries durable task ID/root session ID, ledger-derived
billing evidence (provider/model/call/estimated/timestamp), source labels,
projection version, and `asOf`. A successful authoritative empty aggregate is
`Some(0.0)`; an unbound task, invalid source, or failed query remains `None`
and cannot authorize spend. Unknown holds remain visible and force an
`unknown` band. Remaining is finite and clamped at zero.

## Canonical query seam

`SessionManager::budget_projection` delegates to one query implementation in
`session::budget_projection`. It reads only `sessions`, `cost_ledger`,
`cost_reservations`, and the supplied current `BudgetConfig`; it creates no
table/store and copies no spend snapshot. The root session's durable task ID
comes from `extension_data`. A child/grandchild request first resolves its
canonical top-level root with a cycle-safe recursive ancestor query. Root,
task, tree, ledger, and reservation reads execute in one SQLite read
transaction. Totals use SQL aggregates with invalid-value predicates; only
one parsed latest ledger evidence row per scope is fetched. Task rows are
bounded by the indexed durable task ID; session rows are bounded by the
indexed recursive root tree. The task and session vectors remain separate so
rows present in both scopes cannot double count. Nested grandchildren count
once, same-root siblings are included, and unrelated roots are excluded from
session scope. Active pending/unknown reservations outrank stale ledger
evidence. Pending and unknown reservations remain separate from settled ledger
dollars.

## Fixture coverage

In-memory/pure fixtures cover zero, exact cap, over-cap clamp, pending,
unknown, nested descendants, sibling exclusion, invalid/nonfinite amounts,
invalid caps, unbound tasks, and query-failure unknown state. A temporary
Spectral fixture additionally inserts root/child/grandchild/same-root-sibling
and unrelated-root sessions, ledger rows, and pending/unknown reservations.
It calls the query from both the parent and grandchild, verifies identical
canonical totals, includes the same-root sibling, excludes the unrelated root,
and proves the bounded SQL shape with a source/index contract test. Additional
fixtures cover zero-cost estimated/unpriced rows against
`budget_verdict_with_unpriced`, active-hold billing precedence, RFC3339 offset
ordering and call-ID ties, malformed timestamps, cap normalization parity,
ancestor cycles, and a golden serialization shape.

## Verification

```text
CARGO_INCREMENTAL=0 cargo test -p permagent --lib session::budget_projection -- --nocapture
13 passed; 0 failed

CARGO_INCREMENTAL=0 cargo test -p permagent --lib child_session_inherits_and_contributes_to_parent_budget_task
1 passed; 0 failed

CARGO_INCREMENTAL=0 cargo test -p permagent --lib reservation_scope_is_root_lineage_and_sibling_holds_block
1 passed; 0 failed

CARGO_INCREMENTAL=0 cargo check -p permagent --lib
passed

git diff --check
passed
```

No provider calls, daemon/CLI/UI edits, or new persistence tables were used.
