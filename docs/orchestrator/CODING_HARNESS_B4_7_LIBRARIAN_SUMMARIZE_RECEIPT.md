# B4.7 receipt — librarian atoms and summarize

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This child covers only these two provider completion seams:

- `agents/platform_extensions/librarian_atoms.rs` strong-model atom generation;
- `agents/platform_extensions/summarize.rs` file-summary generation.

Summon/delegate, analytics, Council, external CLIs, and other background
completion seams remain outside this child.

## Implementation evidence

Both paths now call the shared full-model
`AccountedFastCompletion::complete_accounted` entry point. This preserves the
librarian's actor/lead model selection and the summarize extension's provider
model while adding immutable provider/model attribution, a distinct invocation
ID, reserve-before-dispatch, exact Spectral settlement, and fail-closed
paid-error/missing-usage/cancellation handling. Librarian atoms retain their
existing extractive fallback when the provider is absent or fails; summarize
retains its existing tool error response. No parallel memory or accounting
store was introduced.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | Focused Rust tests compiled the changed modules; `git diff --check` passed. |
| Focused source-contract tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib bypass_shared_paid_dispatch_boundary -- --nocapture`: the two B4.7 guards passed (the existing playbook guard also matched); 3 passed, 0 failed. |
| No-paid-dispatch fixtures | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib refuses_paid_dispatch_without_a_durable_task -- --nocapture`: both fast and full shared-accounting fixtures passed; 2 passed, 0 failed, with zero provider calls. |
| `cargo check -p permagent --lib` | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib --message-format=short` completed successfully with 25 GiB free before the link-heavy gate. |
| Static dispatch inventory | passed for audit execution | `target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json` reports 42 seams / 39 strict failures / 3 wrapped; neither target module remains in the inventory. |
| Resource/diff gate | passed | Free disk remained 25 GiB after verification; `git diff --check` passed. |

B4 remains active. This receipt promotes only the two named seams and does not
claim that the remaining 39 production inventory seams are safe.
