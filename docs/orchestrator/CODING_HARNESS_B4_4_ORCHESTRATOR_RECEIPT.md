# B4.4 receipt — orchestrator summaries and roadmap decomposition

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed for this seam; B4 overall remains active  
**Ledger authority:** existing Spectral session/cost ledger only

## Scope

This child DAG covers only the orchestrator's provider completions:

- `view_session(mode="summarize")` conversation summaries;
- the first roadmap-decomposition attempt; and
- the strict JSON retry after an unparseable decomposition response.

No new budget store, process-local spend counter, or transcript-derived task
identity was introduced.

## Implementation evidence

All three physical attempts now call the shared
`AccountedFastCompletion::complete_fast_accounted` entry point. That seam:

1. snapshots the selected provider and model before dispatch;
2. assigns a fresh invocation ID per physical fast/full attempt;
3. reserves paid work before `Provider::complete`;
4. settles authoritative usage through the existing Spectral transaction; and
5. marks paid errors, missing usage, and cancellation as `unknown` rather than
   releasing or guessing the hold.

Local/subscription fast-to-full fallback remains separately attributed. The
existing deterministic no-provider fixture now exercises this shared entry
point and proves that a paid call without a durable task is refused before the
provider is called (`calls == 0`). The existing paid missing-usage, paid
failure, local fallback, and cancellation fixtures continue to cover the
failure contract.

The static source inventory currently reports 47 provider/process seams, with
3 wrapped and 44 intentionally remaining for later B4 children. The
orchestrator's three direct completion sites are no longer bypasses because
they route through the shared wrapper.

## Verification ladder

| Gate | Result | Evidence |
|---|---|---|
| Parser/diff | passed | `git diff --check` is clean; `cargo check -p permagent --lib --message-format=short` parsed and typechecked the implementation. Workspace-wide `cargo fmt --check` still reports pre-existing import/test formatting in the heavily modified shared file; no broad formatter rewrite was applied. |
| Focused no-provider/accounting tests | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib accounted_fast -- --nocapture`: 6 passed, 0 failed. This includes refusal before provider dispatch when no durable task exists. |
| `cargo check -p permagent --lib` | passed | Completed in 48.12s with one linker warning only. |
| Static inventory | passed for audit execution | `target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json` reported 47 seams / 44 strict failures / 3 wrapped. |

This is not a B4 overall pass. B4.4 is complete for the orchestrator seam;
B4 remains active for the other unwrapped production seams.
