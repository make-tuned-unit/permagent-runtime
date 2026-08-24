/**
 * Consume-once guard for a queued Build-tab PTY launch (store.ts
 * `pendingTerminalLaunch`).
 *
 * BuildView's effect that consumes the pending launch and calls
 * `createProjectTab` is not idempotent on its own — it opens a tab as a side
 * effect and only clears the store afterward, asynchronously. If BuildView
 * mounts twice before that clear lands (React StrictMode's dev double-invoke,
 * or a genuine remount from `navigateToTool('build')` racing the same tick),
 * the second run still sees the same launch and opens a second tab. Reported:
 * pressing "Send to Claude" on a Grow action opened two terminal tabs.
 *
 * The fix is a claim, not a lock: each launch carries an id minted once per
 * press (store.ts), and the first caller to claim an id wins — every later
 * claim of the same id is told "already taken" and does nothing. This has to
 * live at module scope, not in a component ref: a ref is exactly what a
 * remount forgets, and forgetting is the bug.
 *
 * TerminalManager.tsx builds its own claim set from the same `createClaimSet`
 * factory (lib/claimSet.ts) as an independent second layer of defence — its
 * own Set, never shared with this one, since each layer must be able to claim
 * on its own.
 */

import { createClaimSet } from '../../lib/claimSet';

const buildViewClaims = createClaimSet(50);

export function claimLaunch(id: string): boolean {
  return buildViewClaims.claim(id);
}

export function resetClaimedLaunches(): void {
  buildViewClaims.reset();
}
