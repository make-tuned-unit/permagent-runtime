/**
 * @vitest-environment jsdom
 *
 * Saving a credential is the highest-consequence "did it work?" question on
 * the Finance tab. A save that failed must never look like one that worked,
 * and "not set" must not double as "still checking".
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    readSecretConfig: vi.fn(),
    upsertConfig: vi.fn(),
    removeConfig: vi.fn(),
  },
}));

import { FundamentalsKey } from './FundamentalsKey';
import { MIN_PENDING_MS } from '../common/Button';
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const readSecret = vi.mocked(api.readSecretConfig);
const upsert = vi.mocked(api.upsertConfig);

let container: HTMLDivElement;
let root: Root;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function buttonNamed(label: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.trim() === label);
}

function typeInto(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

beforeEach(() => {
  vi.useFakeTimers();
  readSecret.mockReset();
  upsert.mockReset();
  readSecret.mockResolvedValue(null);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('says it is still checking rather than reporting "no key" before the read lands', async () => {
  const gate = deferred<{ maskedValue: string } | null>();
  readSecret.mockReturnValue(gate.promise as ReturnType<typeof api.readSecretConfig>);
  await act(async () => { root.render(<FundamentalsKey />); });

  expect(container.textContent).toMatch(/Checking/i);
  expect(container.textContent).not.toMatch(/No key/i);

  await act(async () => { gate.resolve(null); });
  await settle();
  expect(container.textContent).toMatch(/No key/i);
});

it('surfaces a failed save at the control and never ticks', async () => {
  await act(async () => { root.render(<FundamentalsKey />); });
  await settle();
  upsert.mockRejectedValue(new Error('keychain locked'));

  await act(async () => { buttonNamed('Add key')!.click(); });
  const input = container.querySelector('input[type="password"]') as HTMLInputElement;
  await act(async () => { typeInto(input, 'abc123'); });
  await act(async () => { buttonNamed('Save')!.click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(container.textContent).toMatch(/keychain locked/);
  expect(container.querySelector('[data-state="success"]')).toBeNull();
  // The editor stays open so the value the user typed is not lost.
  expect(container.querySelector('input[type="password"]')).toBeTruthy();
});

it('holds the pending phase and ticks on a save that lands', async () => {
  await act(async () => { root.render(<FundamentalsKey />); });
  await settle();
  upsert.mockResolvedValue(undefined as never);

  await act(async () => { buttonNamed('Add key')!.click(); });
  const input = container.querySelector('input[type="password"]') as HTMLInputElement;
  await act(async () => { typeInto(input, 'abc123'); });
  await act(async () => { buttonNamed('Save')!.click(); });

  expect(container.querySelector('[data-pending="true"]')).toBeTruthy();
  await advance(MIN_PENDING_MS + 50);
  expect(container.querySelector('[data-state="success"]')).toBeTruthy();
});
