# Coding Harness Authoritative Evidence DAG 3

Status: passed — 2026-09-04

## Objective

Close the remaining observability producer gaps without parsing transcripts or
inventing values. This child DAG is the active frontier of
`CODING_HARNESS_MASTER_PROGRAM_DAG.yaml`.

```text
A0 freeze producer contracts
 ├── A1 verifier attempts/verdict/evidence ─┐
 ├── A2 typed timeout/failure outcome ─────┼── A4 integration and promotion
 └── A3 parent/child run attribution ──────┘
```

## A0 — Contract freeze

- Existing verification receipts and goal transitions are authoritative.
- Existing typed process/runtime outcomes are authoritative.
- Existing session/dispatch identities are authoritative.
- Prose, terminal text, inferred zeroes, and synthetic parent IDs are not.
- Spectral/session storage remains the only durable substrate.

Gate: DAG 2 tests stay green and every absent producer remains `null`.

## A1 — Verifier evidence

Join declared verification, attempt count, latest structured verdict, and a
bounded non-secret evidence summary from the existing verification lifecycle.
Repeated heartbeats must not increment attempts or replace passing evidence
with an older result.

Gate: pass/fail/uncertain and repeated-event fixtures retain exact evidence.

## A2 — Timeout and failure outcome

Carry typed timeout and failure categories from runtime outcomes into the run
result. Preserve precedence for cancellation and denial. Do not classify by
matching assistant prose.

Gate: success, failure, timeout, denial, and cancellation are mutually
distinguishable in deterministic payload tests.

## A3 — Parent/child attribution

Propagate the real dispatching run/session identity through internal and
external workers and retain it with child evidence and cost. A resumed session
creates a new invocation without breaking its real parent link.

Gate: parent-child fixtures reconcile identity and spend once; unknown parents
remain null.

## A4 — Integration and successor

Run focused tests, evaluator suite, cross-crate compilation, formatting, diff
hygiene, and Command Center typecheck. Update the scorecard from evidence.
When the exit gate passes, atomically mark program node P2 passed and expose P3
as ready through the existing roadmap auto-dispatch path.

Human intervention is reserved for a genuine blocker, externally billed spend
without an approved cap, irreversible effects, or the final release decision.
An unchanged failed verification is never repeated merely to keep a worker
busy.

## Exit evidence

- Structured verifier receipts are authoritative; plaintext success cannot
  advance either interactive or goal completion gates.
- Verification attempts/results are deduplicated, privacy-redacted, bounded,
  and monotonic once a pass is observed.
- Reply results come from per-stream typed outcomes; typed timeouts remain
  distinct from failure without matching error prose.
- Child sessions carry a real `parent_session_id`; run IDs and session IDs are
  not conflated, and child spend uses the existing Spectral/session ledger.
- Atomic roadmap materialization additionally closed the successor trust gate:
  exact project-bound approval, proposal consumption, card creation,
  dependency wiring, audit transitions, and root readiness commit together;
  dispatch occurs only after commit.

Deterministic gates passed: 142 evaluator tests, 27 CLI telemetry tests, 23
after-turn tests, 15 harness-state tests, verifier receipt tests, parent/session
lineage tests, rollback/replay/cross-project approval tests, daemon compilation,
and Command Center typecheck. No provider inference or paid benchmark was run.
