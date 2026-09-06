# B4.8D receipt — provider delegates and local session naming

**Date:** 2026-09-04 (America/Halifax)  
**Status:** passed for this bounded seam; B4 integration remains active

## Scope and decision

- `AnthropicProvider::stream` delegates to its own `stream_split`; it is a
  provider-internal transport beneath the primary Agent reservation, so a
  second reservation there would double-account one physical request.
- `SovereignGuard::stream_split` delegates to its inner provider after its
  egress check; it is the same caller-owned physical stream, not another
  invocation.
- Session naming already produced a deterministic local title from usable user
  text. The empty/attachment-only edge case nevertheless fell through to
  `Provider::complete_fast`, creating avoidable background spend outside an
  explicit user request. It now returns the deterministic `New Session`
  fallback locally. Spectral/session remains the only durable substrate.

The two transport delegates carry typed `permagent-dispatch` exclusions with
their accounting authority. No provider call was made.

## Verification

| Gate | Result | Evidence |
|---|---|---|
| Local title behavior and no-dispatch source contract | passed | `CARGO_INCREMENTAL=0 cargo test -p permagent --lib providers::base::tests:: -- --nocapture`: 18 passed, 0 failed. |
| Production inventory | passed for these seams | `target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json`: both delegates are typed exclusions; the former session-name completion is absent. The converged tree reported 26 total seams and zero unwrapped seams. |
| Formatting and diff | passed | targeted `rustfmt` completed and `git diff --check` passed in the converged worker tree. |

## Integrated-gate honesty

The one full `permagent` library run completed with 4,258 passed, 65 failed,
and 6 ignored. Most failures require listener/network or host-state privileges
that this restricted test environment does not provide. Four isolated
orchestrator fixtures were also red because they did not create/bind the
authoritative durable session/task state now required by B4. Those four reopen
the B4 integration gate and are tracked as B4.11; this receipt does not claim
the overall B4 or P4 gate passed.
