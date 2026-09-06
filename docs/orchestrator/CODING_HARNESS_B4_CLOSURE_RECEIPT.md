# B4 closure receipt — paid dispatch enforcement

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed; B5 activated

## Proven boundary

Every production model invocation, retry/fallback, Council call, background
completion, supervised worker, and external-provider transport identified by
`dispatch-inventory.v2` now has either:

1. the shared reserve-before-dispatch and exact Spectral settlement wrapper;
   or
2. a typed ownership/exclusion marker whose deterministic test proves the
   reservation belongs to an enclosing caller or the subprocess is local,
   deterministic, or separately supervised.

Ambiguous paid work without a durable task identity refuses before dispatch.
Local/subscription usage remains metered without being relabelled paid. Unknown
post-dispatch usage remains a budget-consuming hold. Session naming no longer
spends a model call.

## Exit evidence

- Strict production inventory: 26 seams, 3 wrapped, 23 typed exclusions,
  **0 unwrapped**.
- `permagent-eval`: 159 library tests + 18 CLI tests passed.
- Orchestrator integration: 124 passed, 0 failed after B4.11 bound all four
  stale fixtures to authoritative Spectral session/task identities.
- B4 accounting regression filter: 7 passed, 0 failed.
- Provider-base/local-title filter: 18 passed, 0 failed.
- Every B4.4–B4.11 receipt records focused tests and no paid provider/CLI call.
- `git diff --check` passed.

The broad `permagent` library run remains a known dirty-environment baseline:
4,258 passed, 65 failed, 6 ignored. Listener/network tests cannot bind under
this restricted sandbox; host-state and prompt/identity snapshot clusters are
retained for their owning later defect DAGs. The four B4-relevant failures from
that run were isolated and repaired; this receipt does not claim the unrelated
65-result aggregate is globally green.

## Successor

B5 projection/recovery is active. P4 remains active until B5, B6, and the B7
transition receipt pass.
