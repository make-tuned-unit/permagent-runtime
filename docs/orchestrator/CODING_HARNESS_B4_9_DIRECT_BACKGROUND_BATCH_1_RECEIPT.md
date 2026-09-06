# B4.9 receipt — direct/background completion batch 1

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This child covers only these four genuine model-call sites:

- `Agent::create_recipe` in `agents/agent.rs`;
- `GooseClient::create_message` in `agents/mcp_client.rs` (not MCP enable-sampling);
- `generate_new_app_content` in `agents/platform_extensions/apps.rs`;
- `generate_updated_app_content` in `agents/platform_extensions/apps.rs`.

## Implementation evidence

All four paths now resolve the existing durable session and call the shared
full-model `AccountedFastCompletion::complete_accounted` boundary. Existing
prompts, selected provider/model configuration, response parsing, truncation
checks, deterministic recipe behavior, and error conversion remain intact.
MCP sampling refuses an absent or unresolvable durable session rather than
dispatching an unattributed provider call. No provider base/session naming,
inventory scanner, memory store, or paid provider was changed or invoked.

The shared boundary supplies reserve-before-dispatch, immutable provider/model
and invocation attribution, exact Spectral settlement, and fail-closed paid
error/missing-usage/cancellation handling. Existing full-model refusal and
primary-stream settlement fixtures were reused rather than creating another
accounting path.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | Focused Rust tests compiled the four changed modules; `git diff --check` passed. |
| Source-contract tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib cannot_bypass_shared_paid_dispatch_boundary -- --nocapture`: 6 passed, 0 failed (the three B4.9 guards plus existing guards matched by the shared filter). |
| No-paid-dispatch | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib accounted_full_ -- --nocapture`: 1 passed, 0 failed; missing durable task refused before provider dispatch. |
| Settlement | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib primary_stream_settles_one_authoritative_snapshot_per_invocation -- --nocapture`: 1 passed, 0 failed; exactly one Spectral ledger row and settled reservation were asserted. |
| `cargo check -p permagent --lib` | passed | `CARGO_INCREMENTAL=0 cargo check -p permagent --lib --message-format=short` completed successfully with 24 GiB free before the gate. |
| Resource/diff gate | passed | Free disk remained above the 8 GiB floor; `git diff --check` passed. |

B4 remains active. This receipt closes only direct/background batch 1 and does
not promote doctor, Financier, permission/security, or other remaining seams.
