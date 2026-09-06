# B5 projection/recovery gap audit

Status: read-only audit complete — B5 is not promoted. No provider, GUI, or
production-code work was performed for this audit.

## Finding

The existing implementation has durable ingredients, but no authoritative
budget projection. `HarnessRunView` adds only the current session's
`accumulated_total_tokens` and `accumulated_cost_usd` to the run snapshot.
The B5 gate requires cap, settled, held/unknown, remaining, billing class,
task/session scope, and evidence provenance. Those values must be recomputed
from Spectral on every read and remain nullable when a source cannot be read.

The existing sources are:

- `sessions`: durable session identity, `parent_session_id`, and the budget
  task ID in `extension_data` (`budget_task_id`).
- `cost_ledger`: authoritative settled rows, with `task_id`, session lineage,
  provider/model, `cost_tier`, `is_chargeable`, and `is_estimated`.
- `cost_reservations`: authoritative pending/unknown authorization holds,
  with amount, task/session scope, lease, and state.
- budget configuration: current task/session soft, gate, and hard ceilings.
- `harness_run_snapshots`: durable operational/DAG evidence and terminal state,
  but not a spend snapshot.

## Gap matrix

| B5 field | Current authoritative data | Current projection | Exact gap |
|---|---|---|---|
| Cap | `BudgetConfig` from `/api/governance/budget` / config | absent from `HarnessRunView`, CLI announcement, and Build run UI | Add task and session ceiling triplets, plus cap source and read time. Historical runs also need the cap/config revision used at dispatch, or the UI must label the value as current rather than historical. |
| Settled | `SUM(cost_ledger.cost_usd)` by `task_id` or canonical session tree | `HarnessRunView.spend_usd` is only `sessions.accumulated_cost_usd` for one session; `SpendAnnounceResp.sessionUsd` is the same rollup | Add exact task and session settled queries. Reconcile rollups against ledger and expose query status instead of silently trusting a stale/missing rollup. |
| Held | `cost_reservations.state='pending'`, `SUM(amount_usd)` | absent | Add pending amount/count per task and session scope. Do not fold it into settled. |
| Unknown | `cost_reservations.state='unknown'`; current reservation gate only returns an in-process `Unknown(reason)` | absent; no durable reason column exists | Add unknown amount/count and a conservative blocked/unknown state. If reason provenance is required, persist a bounded reason or expose reservation IDs/state/lease evidence; do not invent a reason from transcript text. |
| Used | No single B5 semantic exists | session rollup only | Define explicitly as `settled + pending + unknown` for authorization remaining. Also expose each component so “used” cannot be mistaken for billed/settled dollars. |
| Remaining | No field | absent | Compute `max(cap - settled - pending - unknown, 0)` only when all required reads and caps are finite. Return nullable/unknown status on read failure or unresolved invalid configuration; never use negative or fabricated zero. |
| Billing class | `cost_ledger.cost_tier`/`is_chargeable`; pending reservations imply paid authorization but carry no provider metadata | `HarnessRunSnapshot.billing_class` is client-announced and `SpendAnnounce` only exposes provider/model/estimated | Add ledger-derived class and provenance. Keep client route class as `reported_route_billing_class` only. `cost_reservations` has no provider/model/class columns, so pending/unknown should be classified as `paid_api` by reservation semantics, with source marked reservation rather than pretending a provider was observed. |
| Task scope | `sessions.extension_data` → `budget_task_id`; `cost_ledger.task_id`; reservation `task_id` | run carries `session_id` but no task ID or task totals | Resolve task ID from the durable session, then aggregate all ledger/reservation rows with that task ID, including child sessions. Null task identity must be an explicit `unbound`/blocked state, not a new task or zero spend. |
| Session scope | `session_id` plus recursive `parent_session_id` tree; reservation code already uses a session-tree concept | run view reads only the named session; `cost_by_parent_session` covers direct children only | Define one canonical tree query/helper and use it for settled, held, unknown, and billing evidence. State whether “session” means the top-level root and test nested grandchildren and sibling exclusion. |
| Evidence provenance | ledger/reservation/session/config tables and timestamps | `evidence`/`result` are worker summaries, not spend evidence; no `asOf`, source, query status, or IDs | Add typed projection provenance: projection version, `asOf`, cap source, ledger source, reservation source, scope source, completeness (`complete|partial|unknown`), and bounded error/unknown indicators. |

## Current path-specific risks

1. `harness_run_view` turns a failed/missing `get_session` into `tokens=None`
   and `spend_usd=None`, which is honest at the Rust boundary, but the current
   Build surface has no explicit unavailable state and can simply omit the
   number.
2. `/api/coding-sessions/spend` uses `unwrap_or(0.0)` for session turn/total
   cost, total tokens, and failed `spend_since`. This can announce false zeroes
   after a rollup/query failure. Its payload has no cap, hold, unknown,
   remaining, task scope, or provenance fields.
3. `session_spend_changed` carries non-null numbers only. The liveness store
   ignores replayed frames by design, but there is no REST rehydrate for
   `codingSpend`; a Build reload/reconnect can therefore show no run total
   until the next CLI announcement.
4. CLI heartbeat and terminal announcements swallow POST failures. Spectral
   ledger rows remain durable, but the active run projection can be missing
   until restart/history hydration; this is not evidence that spend was zero.
5. `GET /api/coding-sessions/harness-runs` hydrates persisted rows but returns
   only active entries after the 45-second heartbeat TTL. It silently falls
   back to the in-memory registry when durable hydration fails. That can return
   an empty 200 response during a Spectral outage. The history endpoint exists,
   but the Command Center API has no history method and Build does not consume
   it.
6. The run's `billing_class` is accepted from the CLI update. It is useful
   routing context but is not ledger evidence and must not authorize a cap or
   label settled spend.
7. `BudgetCeilings::sanitized` preserves non-finite values so authorization
   callers can fail closed, but the pure band comparison itself does not turn a
   NaN ceiling into an explicit unknown. B5 must reject invalid caps before
   projection; this remains a dependency to re-check in the B4 authorization
   gate.
8. Any future `cap - used` implementation must clamp and validate both inputs.
   The current code has no remaining field, so no negative value is currently
   emitted; adding a naïve subtraction would create a new failure mode.

## Required canonical queries/helpers

These must be shared by API and CLI-facing projections rather than duplicated:

1. Resolve `task_id` from `sessions.extension_data` for the run session.
2. Resolve the canonical session tree from the run session, including the root
   and all descendants exactly once; exclude siblings outside that tree.
3. Settled task: `SUM(cost_ledger.cost_usd) WHERE task_id = ?`.
4. Settled session: `SUM(cost_ledger.cost_usd)` joined to the canonical tree.
5. Reservations by task/session scope: separate sums/counts for `pending` and
   `unknown`; retain finite/non-negative validation and lease/state evidence.
6. Latest billing evidence: latest ledger `cost_tier`, `is_chargeable`,
   provider, model, `is_estimated`, `call_id`, and timestamp. Pending/unknown
   reservations are `paid_api` by authorization semantics, not by CLI text.
7. Projection arithmetic: settled, pending, unknown, effective used,
   remaining, band, and a completeness/error state. A query failure, missing
   task identity, invalid cap, non-finite amount, or unresolved unknown hold
   must never become `$0.00`.

## Sequential implementation DAG (B5 remains planned)

```text
B5.0 contract freeze and null semantics
  -> B5.1 canonical Spectral budget projection
  -> B5.2 daemon run/history API and restart recovery
  -> B5.3 CLI spend announcement compatibility
  -> B5.4 Build rehydrate/rendering
  -> B5.5 adversarial integration and evidence gate
```

### B5.0 — Contract freeze

Define a versioned `BudgetProjection` with separate task/session scopes:
`cap`, `settledUsd`, `heldUsd`, `unknownUsd`, `effectiveUsedUsd`,
`remainingUsd`, `band`, `billingClass`, and `provenance`. Null means unavailable;
zero means a successful authoritative query found no amount. Define whether an
unresolved unknown hold makes `remainingUsd` numeric-but-blocked or nullable;
the authorization rule must remain fail-closed either way.

Primary write scope: new projection types/pure arithmetic only.

### B5.1 — Canonical Spectral query seam

Add one session-manager/app-view seam for task identity, canonical descendants,
ledger rollups, reservations, billing evidence, finite validation, and bounded
errors. Do not add a second store or copy spend into harness snapshots.

Primary write scope: `crates/goose/src/session/` and the projection module;
tests use in-memory Spectral fixtures only.

### B5.2 — Daemon API and recovery

Attach the projection to `HarnessRunView`, both active and history responses.
Return a durable-read error rather than a successful empty active list when the
Spectral query fails. Recompute after restart from the same session/ledger IDs;
preserve terminal snapshot monotonicity and the 45-second active TTL. Add the
history route contract to the client-facing API.

Primary write scope: `crates/goose-server/src/routes/coding_session.rs` and
daemon route tests.

### B5.3 — CLI announcement

Keep the ledger as the authority. Extend `/spend` response/event compatibility
only after the daemon projection exists; preserve older clients and stop using
`unwrap_or(0)` for failed reads. Include task/session scope and provenance or
explicitly let the daemon fill them from the session ID.

Primary write scope: `crates/goose-cli/src/session/spend_announce.rs`, event
serialization tests, and no provider calls.

### B5.4 — Build rehydrate/rendering

Add active/history reads and reconnect hydration. Render unavailable, unknown,
estimated, held, and blocked states distinctly from zero; clamp display-only
remaining values as a second defense. Keep the existing statusline's compact
form, with a detail surface for cap and evidence provenance.

Primary write scope: `ui/command-center/src/lib/api.ts`, Build cost/run
components, and TypeScript tests.

### B5.5 — Verification gate

Required no-provider tests:

- pure zero/positive/exact-cap/over-cap arithmetic, NaN/infinity, query-failure,
  pending, unknown, and negative-clamp cases;
- task and recursive session-tree fixtures with nested children and excluded
  siblings;
- settled ledger plus pending/unknown reservations reconcile exactly once;
- local, subscription, paid, unknown-provider, and estimated billing evidence;
- missing rollups never announce false zero;
- daemon restart preserves terminal run identity and recomputes spend;
- active TTL, history projection, durable-read failure, and terminal late
  heartbeat behavior;
- CLI event/replay compatibility and Build reload/reconnect hydration;
- API/UI never display negative remaining or turn unknown into zero.

Promotion requires all fields to agree across Spectral, daemon JSON, CLI/event,
and Build after restart. Until then B5 stays planned and B6 must not treat the
existing session-only meter as a passed cap projection.

## Non-overlapping ownership

| Lane | Files it may write | It must not modify |
|---|---|---|
| Projection | `crates/goose/src/session/`, new projection tests | daemon routes, CLI, UI |
| Daemon | `crates/goose-server/src/routes/coding_session.rs`, route tests | Spectral query implementation, CLI, UI |
| CLI/events | `crates/goose-cli/src/session/spend_announce.rs`, event tests | daemon routes, UI, schema |
| Build | `ui/command-center/src/lib/api.ts`, Build cost/run components/tests | Rust, CLI, Spectral |
| Integration | dedicated test/receipt/docs files only | production implementation lanes |

No live provider, GUI, rebuild, or periodic monitoring was used for this
projection audit.
