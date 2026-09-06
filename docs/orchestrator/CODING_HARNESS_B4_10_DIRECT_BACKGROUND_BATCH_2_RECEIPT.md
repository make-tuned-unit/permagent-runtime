# B4.10 receipt — direct/background completion batch 2

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This bounded child covers only genuine model-call sites in:

- `doctor.rs` provider health checks and fallback probes;
- `financier_close.rs` `judge_with_opus`;
- `permission/permission_judge.rs` read-only classification;
- `security/adversary_inspector.rs` tool-call review.

Council, provider implementations, sovereign guard, external CLI/GoalTask
paths, browser/UI, and inventory scanner code were not changed.

## Implementation evidence

Each site now resolves a durable Spectral session and routes the full-model call
through `AccountedFastCompletion::complete_accounted`. Doctor carries the real
interactive session through every provider/model fallback probe. Financier uses
the durable hidden `financier-close` session. Permission judging and adversary
inspection use the request's durable session and fail closed to their existing
safe behavior when it cannot be loaded.

Prompts, provider/model selection, parsing, fallback behavior, and failure
semantics remain intact. Paid or ambiguous calls without a task identity are
refused before provider dispatch; local/subscription calls remain attributed
and metered without fabricated paid spend. No parallel memory or accounting
store was introduced, and no provider call was made during verification.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | `git diff --check` passed; changed modules compiled in the focused test target. |
| Source-contract tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib cannot_bypass_shared_paid_dispatch_boundary -- --nocapture`: 10 passed, 0 failed (four B4.10 guards plus existing batch guards matched by the shared filter). |
| No-paid-dispatch / settlement proof | passed | Existing shared fixtures `accounted_full_refuses_paid_dispatch_without_a_durable_task` and `primary_stream_settles_one_authoritative_snapshot_per_invocation` passed; the former asserted zero provider calls and the latter asserted one ledger row plus a settled reservation. |
| `cargo check -p permagent --lib` | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib --message-format=short` completed successfully with 23 GiB free before the gate. |
| Inventory snapshot | passed for audit execution | `target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json` reported 26 seams: 3 wrapped and 23 explicitly excluded; none of the four B4.10 modules appeared as an unwrapped completion seam. |
| Resource/diff gate | passed | Free disk remained above the 8 GiB floor; `git diff --check` passed. |

B4 remains active. This receipt closes only direct/background batch 2 and does
not promote unrelated Council, provider, CLI, or UI/voice work.
