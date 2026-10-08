/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { runHomeFocusTransition } from './homeFocus';
afterEach(() => {
  Reflect.deleteProperty(document, 'startViewTransition');
  delete document.documentElement.dataset.homeFocusVt;
  localStorage.removeItem('permagent-reduce-motion');
});
it('an older transition finishing cannot remove a newer transition style marker', async () => {
  localStorage.setItem('permagent-reduce-motion', 'false');
  const finish: Array<() => void> = [];
  Object.defineProperty(document, 'startViewTransition', { configurable: true, value: (cb: () => void) => {
    cb(); return { finished: new Promise<void>(resolve => finish.push(resolve)) };
  } });
  runHomeFocusTransition(() => {}); runHomeFocusTransition(() => {});
  finish[0](); await Promise.resolve();
  expect(document.documentElement.dataset.homeFocusVt).toBe('true');
  finish[1](); await Promise.resolve();
  expect(document.documentElement.dataset.homeFocusVt).toBeUndefined();
});
it('reduced motion applies the update directly without invoking browser morphing', () => {
  localStorage.setItem('permagent-reduce-motion', 'true');
  const start = vi.fn(); const update = vi.fn();
  Object.defineProperty(document, 'startViewTransition', { configurable: true, value: start });
  runHomeFocusTransition(update);
  expect(update).toHaveBeenCalledTimes(1); expect(start).not.toHaveBeenCalled();
});
it('an unavailable transition engine still applies the focus change', () => {
  const update = vi.fn();
  Object.defineProperty(document, 'startViewTransition', { configurable: true, value: () => { throw new Error('not supported'); } });
  runHomeFocusTransition(update);
  expect(update).toHaveBeenCalledTimes(1);
  expect(document.documentElement.dataset.homeFocusVt).toBeUndefined();
});
