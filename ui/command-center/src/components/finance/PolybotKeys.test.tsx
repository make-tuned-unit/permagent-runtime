/**
 * @vitest-environment jsdom
 *
 * Keys stay a status list. Opening all six password fields at once is what
 * made the Finance Polybot card unusable.
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

import { PolybotKeys } from './PolybotKeys';
import { MIN_PENDING_MS } from '../common/Button';
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const readSecret = vi.mocked(api.readSecretConfig);
const upsert = vi.mocked(api.upsertConfig);

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function typeInto(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

beforeEach(() => {
  readSecret.mockReset();
  upsert.mockReset();
  readSecret.mockImplementation(async (key: string) => (
    key === 'POLYMARKET_API_KEY' ? { maskedValue: '01a03932****' } : null
  ));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('shows status rows and no password fields until Add or Replace', async () => {
  await act(async () => { root.render(<PolybotKeys />); });
  await flush();
  expect(container.querySelectorAll('[data-testid="polybot-key-row"]')).toHaveLength(6);
  expect(container.querySelectorAll('[data-testid="polybot-key-editor"]')).toHaveLength(0);
  expect(container.querySelectorAll('input[type="password"]')).toHaveLength(0);
  expect(container.textContent).toMatch(/1 of 5 required keys/);
  expect(container.textContent).toMatch(/01a03932/);
});

it('opens a single editor on Replace', async () => {
  await act(async () => { root.render(<PolybotKeys />); });
  await flush();
  const replace = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Replace');
  expect(replace).toBeTruthy();
  await act(async () => { replace!.click(); });
  expect(container.querySelectorAll('[data-testid="polybot-key-editor"]')).toHaveLength(1);
  expect(container.querySelectorAll('input[type="password"]')).toHaveLength(1);
});

it('surfaces a failed key save at the control and never ticks', async () => {
  vi.useFakeTimers();
  await act(async () => { root.render(<PolybotKeys />); });
  await flush();
  upsert.mockRejectedValue(new Error('keychain locked'));

  const replace = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Replace');
  await act(async () => { replace!.click(); });
  const input = container.querySelector('input[type="password"]') as HTMLInputElement;
  await act(async () => { typeInto(input, 'secret'); });
  const save = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Save');
  await act(async () => { save!.click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(container.textContent).toMatch(/keychain locked/);
  expect(container.querySelector('[data-state="success"]')).toBeNull();
});

it('says a row is still being checked rather than reporting "not set"', async () => {
  let release!: () => void;
  const gate = new Promise<void>((res) => { release = res; });
  readSecret.mockImplementation(async (key: string) => {
    await gate;
    return key === 'POLYMARKET_API_KEY' ? { maskedValue: '01a03932****' } : null;
  });
  await act(async () => { root.render(<PolybotKeys />); });

  expect(container.textContent).toMatch(/Checking/);
  expect(container.textContent).not.toMatch(/not set/);

  await act(async () => { release(); await Promise.resolve(); });
  await flush();
  expect(container.textContent).toMatch(/not set/);
});
