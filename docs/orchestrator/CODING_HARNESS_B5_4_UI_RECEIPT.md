# B5.4 receipt — Build budget projection recovery and rendering

Date: 2026-09-05 (America/Halifax)

Status: UI lane passed root review and focused regression execution;
B5 overall remains pending B5.5 ledger/restart-to-consumer integration.

Root found and repaired one additional child-rollup rendering race: the prior
session's subagent cost stayed visible while the new session request loaded.
Rollup state now carries its source session identity and is filtered before
rendering. The new regression failed before the fix and passed afterward.
On 2026-09-05 the three cost/liveness/render files executed52 tests, and the
separate useAppNavigate.replay suite executed19 tests (71 total). Typecheck
and scoped diff checks passed. These are UI/unit integration assertions, not
an installed desktop or live daemon journey.

## Contract delivered

The Command Center now models the daemon's `budget-projection.v1` payload as a
strict runtime-validated type. Version, root-session identity, timestamps,
finite/non-negative numeric optionals, cap ordering, completeness/band values,
billing evidence, and provenance are checked before a projection can enter the
store. A numeric zero remains distinct from `null` (unavailable/invalid).

The Build statusline renders canonical session and task settled, held,
unknown, effective-used, remaining, cap, billing, and provenance evidence.
Unknown and unavailable states render as explicit labels rather than `$0.00`.
The harness projection is authoritative when present; chat `TokenState` is not
used to fill an active harness projection.

Active harness views are hydrated once at startup and once per global-events
reconnect from `GET /api/coding-sessions/harness-runs`; there is no polling
path in this recovery seam. A hydration response cannot erase a newer live
spend frame. Replayed frames, stale projection timestamps, late non-terminal
frames after a terminal frame, malformed projections, and identity mismatches
are ignored. History is exposed through the matching API client method for
future detail surfaces. Initial/read failures are represented as explicit
unavailable state without inventing a session id or falling back to chat
spend. Overlapping hydration requests use a generation guard; an empty active
TTL response clears only non-terminal active state, preserving terminal
evidence as terminal rather than converting it to zero.

Cross-session malformed data is isolated: a bad B projection produces an
unavailable B identity with null amounts while valid A remains only as
last-known evidence. A valid B projection can then recover even when its
provider timestamp predates A; timestamp ordering is scoped to one session.

## Verification

```text
ui/command-center: npm run typecheck
passed

ui/command-center: npm test -- --run \
  src/lib/costMeter.test.ts \
  src/lib/livenessSync.test.ts \
  src/components/build/CostStatusline.test.tsx
70 passed; 0 failed

git diff --check -- <owned UI files>
passed
```

Vitest emitted the existing sandbox `listen EPERM` warning for Vite's HMR
socket; the focused tests completed successfully. No Rust builds or provider
calls were used.

## Gate ledger

| Node | State | Evidence |
|---|---|---|
| B5.0/B5.1 | passed | canonical projection contract and Spectral query seam receipt |
| B5.2 | focused runtime passed | seven daemon projection/helper assertions in runtime recovery receipt |
| B5.3 | focused serialization passed | one executed core event assertion; broader CLI boundary remains in B5.5 |
| B5.4 | UI lane passed | root review and71 focused UI/reconnect assertions; B5.5 still required |
| B5.5 | pending | adversarial integrated verification remains |
