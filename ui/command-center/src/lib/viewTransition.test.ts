/**
 * @vitest-environment jsdom
 *
 * runViewTransition — no-op fallback when the API is missing (jsdom, an
 * older WKWebView) or reduced motion is on; otherwise defers to the native
 * API, which itself calls back synchronously.
 */
import { afterEach, expect, it } from 'vitest';
import { runViewTransition } from './viewTransition';
import { setReduceMotion } from '../styles/tokens';

// Cast through `Record<string, unknown>` rather than `Document & { startViewTransition?: ... }`:
// current TS DOM lib types `startViewTransition` as a required member returning
// a full `ViewTransition`, which is stricter than jsdom's own reality (it has
// no such member at all) and than a minimal test double needs to satisfy.
const doc = document as unknown as Record<string, unknown>;

afterEach(() => {
  setReduceMotion(false);
  delete doc.startViewTransition;
});

it('runs the update synchronously when startViewTransition is unavailable (jsdom has none)', () => {
  let ran = false;
  runViewTransition(() => { ran = true; });
  expect(ran).toBe(true);
});

it('uses document.startViewTransition when present', () => {
  let calledWith: (() => void) | undefined;
  doc.startViewTransition = (cb: () => void) => {
    calledWith = cb;
    cb();
    return { ready: Promise.resolve() };
  };
  let ran = false;
  runViewTransition(() => { ran = true; });
  expect(calledWith).toBeDefined();
  expect(ran).toBe(true);
});

it('falls back to a synchronous update under reduced motion even if the API exists', () => {
  setReduceMotion(true);
  let usedApi = false;
  doc.startViewTransition = () => {
    usedApi = true;
    return { ready: Promise.resolve() };
  };
  let ran = false;
  runViewTransition(() => { ran = true; });
  expect(usedApi).toBe(false);
  expect(ran).toBe(true);
});
