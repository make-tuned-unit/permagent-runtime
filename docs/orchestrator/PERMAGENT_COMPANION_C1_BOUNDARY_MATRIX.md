# C1 boundary and fault inventory

Owner: root integration, 2026-09-05. C1.1 source inventory only; not a C1
runtime pass. All fixtures below use existing stores/controllers. No authority
comes from a model's reputation, previous success, or a manifest status.

| Caller / effect | Existing boundary and fault | Evidence mapped | Remaining acceptance |
|---|---|---|---|
| HTTP app/device → control route | Bearer identity, absent/wrong token refusal, audit attribution | `trust_boundary.rs::the_token_gates_the_control_plane_and_the_audit_records_the_use` exercises configured router, master/device attribution and real response status | Execute isolated integration target; token admission is not proof of task approval or granular device privilege |
| Agent → scoped extension | Malformed scope fails; empty scope denies; display names normalize; external engines cannot claim in-process enforcement | `dispatch_scope.rs` six existing fixtures | Execute focused filter; preserve external-worker permission boundary |
| Concurrent workers → filesystem | Unsafe path rejected; overlapping/unknown writers serialized; only explicit disjoint/read-only work parallelized | `write_scope.rs` six existing fixtures | Execute focused filter and consume actual worker-dispatch evidence; metadata alone is not OS sandbox enforcement |
| Request → cancellation / reconnect | Empty active-request preamble reconciles idle; unknown cancellation false; live token cancellation true | `stream_lifecycle.rs::streaming_lifecycle_truth_signals` | Execute isolated target; this unauthed handler fixture does not replace auth tests or prove voice STT cancellation |
| Events → client liveness | Seven existing event constructors reach actual local WebSocket; payload envelope and content minimization | `liveness_wire.rs::liveness_lanes_emit_to_real_websocket` | Execute bounded 10-second/500-frame target; no production event/log data |
| Worker → completion / restart | Running heartbeat freezes at terminal; process identity serialized; rebind/stale classification | `execution_receipt.rs` fixtures; runtime receipt records orphan restart 1/1 | Execute affected focused filters on final snapshot; daemon startup ordering test queued |
| Provider → spend | Atomic claim, unknown hold retained after restart, duplicate completion deduped, mixed billing attribution | B6.1–B6.6 six executed fixtures in runtime recovery receipt | Consume B6.7 integration, do not rerun unchanged fixtures or infer paid-provider quality |
| Approved review → successor | Existing Decision Inbox effect outbox plus program bridge; exact approved manifest and gate evidence required | Bridge seven historical tests passed; new registration adversarial cases under repair | New approval provenance/transaction/gate-specific evidence/retry tests must execute; generic verdict cannot pass every named gate |
| Voice capture → inference | PCM memory/duration cap; generation invalidation and one final required | Capture-limit daemon test queued; optional streaming fake tests prepared | Execute route producer→wire cancellation tests and actual local model/device acceptance |

## Ordered remaining C1 work

1. Finish the active bridge/voice boundary repairs and obtain one coordinated
   compiling snapshot. Do not launch redundant Cargo jobs during shared edits.
2. Execute the existing scoped core filters and daemon integration targets via
   the runtime owner. Preflight the 8 GiB disk floor before linking. Infrastructure
   failures remain distinct from assertion failures; no repeated unchanged retry.
3. Reuse B5/B6 spend receipts, then verify the approved task → exact worker →
   failure/restart → typed terminal receipt → UI projection journey. Require
   consistent task/session/invocation identity and no automatic replay of unknown
   external effects.
4. Independent review consumes this matrix, source diff identity and executed
   results. C1 does not advance C2 merely because all named tests exist.

No new scheduler, permission framework, memory table, model call or production
mutation was performed for this inventory. Final native build, physical voice,
and spend-authorized held-out evaluations remain separate program gates.
