# B4.5 receipt — initiative draft and meeting write-up

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This child covers the two remaining `complete_fast` production sites owned by
this seam:

- `initiative/draft.rs` cheap proposal drafting; and
- the `ViaSession` branch of `meeting_writeup.rs`.

The meeting mesh generation path and summon/delegate routing are explicitly
outside this child.

## Implementation evidence

Both paths now:

1. resolve a durable hidden session in the existing SessionManager/Spectral
   store;
2. call `AccountedFastCompletion::complete_fast_accounted` rather than
   `Provider::complete_fast` directly;
3. snapshot the selected provider/model and assign a new invocation ID for
   each physical fast/full attempt; and
4. inherit the shared reserve-before-dispatch, exact-settlement, and
   paid-error/missing-usage/cancellation-unknown behavior.

The durable hidden session has no automatically invented task budget. A paid
provider therefore fails closed before dispatch unless a real durable task is
bound; local/subscription work remains attributable without a paid hold. Two
deterministic source-contract guards assert that these modules use the shared
entry point and contain no direct `.complete_fast(` bypass.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | Rust source was parsed by the existing formatter check; `git diff --check` passed for the touched implementation files. Workspace-wide formatting still reports pre-existing ordering/test-format deltas in the shared `reply_parts.rs`; no broad formatter rewrite was applied. |
| Focused runtime tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib shared_paid_dispatch_boundary -- --nocapture`: 2 passed, 0 failed. |
| `cargo check -p permagent --lib` | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib --message-format=short` completed successfully after the resource gate recovered. |
| Static dispatch inventory | passed for audit execution | `target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json` reports 44 seams / 41 strict failures / 3 wrapped; neither target module contains a direct provider completion seam. |

B4 overall remains active. This receipt does not promote the remaining 41
unwrapped production seams.
