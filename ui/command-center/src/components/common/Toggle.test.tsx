/**
 * @vitest-environment jsdom
 *
 * Toggle — the contract a settings switch owes the user.
 *
 * A switch bound to a server-persisted setting is a promise-returning action
 * wearing a switch's clothes. The old one had no busy phase and no failure
 * path at all, so six call sites had each hand-rolled their own optimistic
 * flip and their own revert, and the call sites that had not simply lied about
 * what the daemon had stored. What is asserted here is the whole of it:
 * optimistic flip, a visible in-flight phase, revert-and-say-so on failure,
 * and a disabled state that can carry its reason.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Toggle } from './Toggle';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function sw(): HTMLButtonElement {
  return container.querySelector('button') as HTMLButtonElement;
}

async function flush() {
  await act(async () => { for (let i = 0; i < 6; i += 1) await Promise.resolve(); });
}

/** A promise the test settles by hand, so the in-flight window is observable. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

it('is a switch, not an anonymous button', () => {
  act(() => { root.render(<Toggle on label="Remote access" />); });
  expect(sw().getAttribute('role')).toBe('switch');
  expect(sw().getAttribute('aria-checked')).toBe('true');
  expect(sw().getAttribute('aria-label')).toBe('Remote access');
});

it('flips to the requested position before the write lands', async () => {
  const d = deferred<void>();
  act(() => { root.render(<Toggle on={false} onChange={() => d.promise} />); });
  await act(async () => { sw().click(); });
  // The prop still says off — the user sees on, because that is what they asked
  // for and the app believes it will happen.
  expect(sw().getAttribute('aria-checked')).toBe('true');
  await act(async () => { d.resolve(); await d.promise; });
});

it('says it is busy while the write is in flight, and refuses a second flip', async () => {
  const d = deferred<void>();
  const onChange = vi.fn(() => d.promise);
  act(() => { root.render(<Toggle on={false} onChange={onChange} />); });
  await act(async () => { sw().click(); });
  expect(sw().getAttribute('aria-busy')).toBe('true');
  expect(sw().getAttribute('data-pending')).toBe('true');
  await act(async () => { sw().click(); });
  expect(onChange).toHaveBeenCalledTimes(1);
  await act(async () => { d.resolve(); await d.promise; });
  await flush();
  expect(sw().getAttribute('data-pending')).toBeNull();
});

it('reverts to where it was and says so when the write throws', async () => {
  const d = deferred<void>();
  act(() => { root.render(<Toggle on={false} onChange={() => d.promise} />); });
  await act(async () => { sw().click(); });
  expect(sw().getAttribute('aria-checked')).toBe('true');
  await act(async () => { d.reject(new Error('daemon unreachable')); await d.promise.catch(() => {}); });
  await flush();
  expect(sw().getAttribute('aria-checked')).toBe('false');
  expect(container.textContent).toContain('daemon unreachable');
});

it('treats a resolved `false` as a failure, the way Button does', async () => {
  act(() => { root.render(<Toggle on={false} onChange={() => Promise.resolve(false)} />); });
  await act(async () => { sw().click(); });
  await flush();
  expect(sw().getAttribute('aria-checked')).toBe('false');
  expect(container.textContent?.toLowerCase()).toContain("couldn't save");
});

it('hands the position back to the prop once the write lands', async () => {
  const d = deferred<void>();
  act(() => { root.render(<Toggle on={false} onChange={() => d.promise} />); });
  await act(async () => { sw().click(); });
  await act(async () => { d.resolve(); await d.promise; });
  await flush();
  // The caller re-renders with the value the daemon confirmed — which here is
  // NOT what was asked for. The switch must follow the daemon, not its own
  // optimism, or it goes on claiming a setting that was never stored.
  act(() => { root.render(<Toggle on={false} onChange={() => d.promise} />); });
  expect(sw().getAttribute('aria-checked')).toBe('false');
});

it('has no busy phase for a purely local setting', async () => {
  const onChange = vi.fn();
  act(() => { root.render(<Toggle on={false} onChange={onChange} />); });
  await act(async () => { sw().click(); });
  expect(onChange).toHaveBeenCalledWith(true);
  expect(sw().getAttribute('data-pending')).toBeNull();
});

it('carries the reason it cannot be pressed', () => {
  const onChange = vi.fn();
  act(() => {
    root.render(<Toggle on={false} disabled disabledReason="Tailscale is not installed." onChange={onChange} />);
  });
  expect(sw().disabled).toBe(true);
  expect(sw().title).toBe('Tailscale is not installed.');
  act(() => { sw().click(); });
  expect(onChange).not.toHaveBeenCalled();
});

it('renders a caller-owned message in the same place as its own', () => {
  act(() => { root.render(<Toggle on error="Saved, but could not re-read it." />); });
  expect(container.textContent).toContain('Saved, but could not re-read it.');
});
