/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

// Every api method ModelsPanel is allowed to touch. Anything else — notably
// the retired getWorkers / GET /api/agent/workers — throws on access, so a
// regression that re-adds the duplicate roster fails here.
const allowed = vi.hoisted(() => ({
  getConfig: vi.fn(),
  readConfig: vi.fn(),
  upsertConfig: vi.fn(),
  getOllamaStatus: vi.fn(),
  getLibrarianSchedule: vi.fn(),
  setLibrarianSchedule: vi.fn(),
  runLibrarianNow: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`ModelsPanel touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { ModelsPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  allowed.getConfig.mockReset().mockResolvedValue({ config: {}, effective_goose_mode: 'auto' } as never);
  allowed.readConfig.mockReset().mockResolvedValue(null as never);
  allowed.upsertConfig.mockReset().mockResolvedValue({} as never);
  allowed.getOllamaStatus.mockReset().mockResolvedValue({ reachable: false } as never);
  allowed.getLibrarianSchedule.mockReset().mockResolvedValue(null as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(goto: (key: string) => void) {
  await act(async () => { root.render(<ModelsPanel goto={goto} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe('ModelsPanel roster pointer', () => {
  it('points at Settings → Agents instead of fetching its own worker roster', async () => {
    const goto = vi.fn();
    await mount(goto);

    const text = container.textContent ?? '';
    expect(text).not.toContain('Loading roster');
    expect(text).not.toContain('No workers configured');

    const open = container.querySelector('[data-testid="models-open-agents"]') as HTMLButtonElement;
    expect(open).toBeTruthy();
    await act(async () => { open.click(); });
    expect(goto).toHaveBeenCalledWith('agents');
  });
});
