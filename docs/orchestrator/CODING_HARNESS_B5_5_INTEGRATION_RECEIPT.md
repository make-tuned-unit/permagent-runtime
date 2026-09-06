# B5.5 integration evidence — 2026-09-05

Status: deterministic producer/consumer restart contract passed; installed
runtime fan-in remains open. No model calls or paid inference used.

## Actual shared boundary

`b6_restart_reconciles_unknown_hold_and_blocks_replay` creates a real temporary
session store, authorizes a paid reservation, reopens/reconciles it and asserts
the entire serialized projection against
`scripts/testdata/budget_projection_v1.json`. Only generated task/root IDs and
the snapshot timestamp are normalized. The retained hold is unknown0.75,
used0.75 and remaining0.25, not free/zero or dispatch permission.

`CostStatusline.test.tsx` imports that same file, substitutes valid fixed
identities/time, and drives the production event parser/store/component.
It then clears client state and drives real hydration handling using the same
projection at the mocked HTTP boundary. Rendered text and canonical store
projection must match through both paths, including unknown and remaining.
The backend fixture is not independently retyped in TypeScript.

## Executed evidence

- Rust `cargo test -p permagent --lib b6_ -- --nocapture`:6passed,0failed;
  command details and infrastructure history in runtime recovery receipt.
- UI `npm test -- --run src/lib/costMeter.test.ts src/lib/livenessSync.test.ts
  src/components/build/CostStatusline.test.tsx src/hooks/useAppNavigate.replay.test.ts`:
  72passed,0failed (19+29+5+19), root execution.
- `npm run typecheck`: exit0, root execution.
- Prior daemon projection helper tests:7passed after generated-dylib signature
  recovery. These are pure helper tests, not actual HTTP integration.

## Remaining boundary

The UI HTTP method is mocked here. A fresh installed daemon/CLI request and
actual route response after restart are not established by these tests.
The running daemon reports an older SHA, and must not be relabelled as the
current source. B5 whole-program promotion must explicitly account for that
runtime boundary rather than call this shared-fixture test a live end-to-end
journey. B6 adjacent approval/verification/dispatch regression checks remain
separate from its six budget fixtures.
