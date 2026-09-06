# B4.8 receipt — primary Agent stream

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This child closes only the primary Agent stream seam in
`crates/goose/src/agents/reply_parts.rs` and its turn-level integration in the
existing Agent loop. No provider transport, inventory classification,
librarian/summarize, Council, UI, voice, analytics, summon/delegate, or daemon
build work was included.

## Audited contract

The Agent loop captures the selected provider/model and a fresh invocation ID,
reserves before calling `stream_split`, and does not dispatch when authorization
fails. It retains the final authoritative cumulative usage snapshot, settles it
once under that invocation identity, and marks a dispatched stream error,
missing authoritative usage, accounting failure, or cancellation as `unknown`.
The existing provider retry envelope remains inside the one reserved logical
invocation; no second untracked ledger path was introduced.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | Focused Rust tests parsed and compiled the changed source; `git diff --check` passed. |
| Primary-stream regression tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib primary_stream_ -- --nocapture`: 2 passed, 0 failed. Coverage includes paid refusal before dispatch with zero provider calls and one exact authoritative settlement row per invocation. |
| Existing retry/unknown fixtures | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib accounted_fast_ -- --nocapture`: 6 passed, 0 failed. Coverage includes missing usage → unknown, paid provider error → unknown with no regular fallback, local physical retry attribution, paid refusal, success settlement, and cancellation → unknown. |
| `cargo check -p permagent --lib` | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib --message-format=short` completed successfully with 26 GiB free before the gate and 25 GiB after verification. |
| Resource/diff gate | passed | No paid provider calls were made; free disk remained above the 8 GiB floor and `git diff --check` passed. |

B4 remains active. This receipt closes only the primary Agent stream seam; the
remaining B4 inventory and separate routing seams are not promoted here.
