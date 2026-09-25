/**
 * runViewTransition, called from a plain node-environment test (no
 * `@vitest-environment jsdom`) — the shape `store.ts`'s own tests run under,
 * where `document` itself is undefined, not merely `document.
 * startViewTransition`. `decisionInbox.store.test.ts` caught this for real:
 * `navigateToTool` now routes through `runViewTransition`, and referencing
 * `document` unconditionally crashed every node-environment test that calls
 * it (openDecisionInbox → navigateToTool → runViewTransition).
 */
import { afterEach, expect, it, vi } from 'vitest';
import { runViewTransition } from './viewTransition';

afterEach(() => { vi.unstubAllGlobals(); });

it('runs the update with no error when `document` does not exist at all', () => {
  // `vi.stubGlobal` rather than relying on the default (node) environment
  // actually lacking `document`: a jsdom-environment sibling test file run
  // in the same worker can leave a global `document` behind, which made this
  // assertion order-dependent when it merely checked `typeof document`.
  vi.stubGlobal('document', undefined);
  let ran = false;
  expect(() => runViewTransition(() => { ran = true; })).not.toThrow();
  expect(ran).toBe(true);
});
