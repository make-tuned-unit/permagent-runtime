# Coding Harness Instrumentation DAG 2

Status: deterministic implementation gates passed — 2026-09-04. Held-out
qualification remains on Hold; D2 has named producer gaps.

## Objective

Turn the existing tested observability schemas into authoritative, durable run
evidence without increasing provider spend or creating a memory system beside
Spectral. This DAG does not run external model benchmarks. It makes the next
benchmark trustworthy enough to run.

## Graph

```text
D0 freeze contracts and baseline
 ├── D1 durable run ledger ───────────┐
 ├── D2 authoritative runtime events ├── D4 integration and scorecard
 └── D3 request-context attribution ─┘
```

The three implementation lanes have disjoint primary ownership. D4 begins only
after every lane returns a bounded diff and deterministic evidence.

## D0 — Contract freeze

Inputs:

- run wire fields remain camelCase and backwards compatible;
- absent measurements remain `null`, never inferred as zero;
- provider/model/billing come from the active route;
- prompt text remains bounded to the local authenticated live-view path;
- Spectral is the sole durable memory system;
- provider usage is authoritative and pre-call token counts are labelled estimates.

Gate: the current 133 evaluator library tests, 12 evaluator CLI tests, 13
harness projection tests, 8 CLI heartbeat tests, and 6 context-packet tests are
the locked regression baseline.

## D1 — Durable run ledger

Persist versioned harness-run snapshots through the existing Spectral/session
SQLite migration and repository patterns. Preserve terminal outcomes across
daemon restart. Keep the small in-memory registry as an active projection only;
do not introduce a JSON sidecar or secondary memory database.

Gate:

- migration is additive and idempotent for new and existing databases;
- upserting one run cannot rebind its session or regress a terminal state;
- a new manager/daemon instance can read the retained terminal result;
- bounded fields and null semantics round-trip exactly.

## D2 — Authoritative runtime events

Populate counters and outcome evidence from structured session/tool/gate events.
Do not scan prose or infer an absent event as zero. Bind child runs to their
parent when the runtime has a real parent session/run ID.

Gate:

- tool, retry, gate, and verification counters increment once per real event;
- repeated heartbeats do not increment or erase counters;
- failure, timeout, denial, and success retain distinct terminal evidence;
- unavailable sources remain null with no fabricated result.

## D3 — Request-context attribution

Carry typed project-memory and Spectral-recall contribution metadata to the
same `ContextPacket` emitted immediately before the provider request. This is
telemetry over context already selected for the request, not a new retrieval or
storage path.

Gate:

- all five packet slots reflect their actual request contribution or explicit
  missing reason;
- Spectral provenance is deduplicated and survives the join;
- no prompt/memory body is written to logs;
- provider usage joins once per turn and remains the authoritative token count.

## D4 — Integration and scorecard

Run focused tests, formatting, `git diff --check`, Command Center typecheck, and
the daemon/CLI/evaluator cross-crate check. Add failure-path tests discovered in
review. Update the retained scorecard from evidence only.

Exit criteria:

- no unresolved instrumentation P1;
- the no-model conformance suite is green;
- a restart round-trip produces a qualification-ready run artifact;
- remaining unknown fields are explicitly named;
- status remains `Hold` until three distinct held-out runs pass the existing
  qualification contract.

## Outcome

| Lane | Result | Retained evidence |
|---|---|---|
| D1 durable run ledger | Pass | Additive/idempotent Spectral-session SQLite schema; private prompt context is redacted; session binding and terminal monotonicity survive restart; a late active heartbeat cannot resurrect a terminal run. |
| D2 runtime events | Partial | Structured tool and gate request IDs are deduplicated; post-reply retry zero is authoritative; success/failure/denial/cancellation are distinct. Timeout classification, verifier output, and parent-run linkage remain `null` because their structured producers are not yet joined. |
| D3 context attribution | Pass | The exact installed repo-orientation and filtered Spectral recall blocks carry typed attribution to the provider-request seam; mixed sources, duplicates, anonymous records, top-k, truncation, and provenance are covered without logging bodies. |
| D4 integration | Pass for deterministic scope | 7 context tests, 15 harness/durability tests, 14 heartbeat/event tests, 12 Brain bridge tests, 133 evaluator library tests, 12 evaluator CLI tests, cross-crate check, Command Center typecheck, formatting, and diff hygiene passed. |

Observability moves from **Poor** to **Good**, not Excellent. The next bounded
DAG must connect authoritative verifier, timeout, and parent/child events, then
run three distinct held-out qualifications. No externally billed inference was
used in this DAG.
