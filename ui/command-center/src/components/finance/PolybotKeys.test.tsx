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
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const readSecret = vi.mocked(api.readSecretConfig);

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

beforeEach(() => {
  readSecret.mockReset();
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
