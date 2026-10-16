/** @vitest-environment jsdom
 *
 * "What Henry can use" (dagK settings pass, ST1 round 2 — ruling 2026-09-25:
 * add a Tools capability section distinct from Models, using the persona's configured
 * name). Covers the loading/empty/populated states and the name fallback.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  getExtensions: vi.fn(),
  getIdentity: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`ToolsPanel touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { ToolsPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => { root.render(<ToolsPanel goto={vi.fn()} section="tools" />); });
}

describe('ToolsPanel "What <name> can use"', () => {
  it('falls back to "your agent" before the persona name loads', async () => {
    allowed.getExtensions.mockResolvedValue({ extensions: [] });
    allowed.getIdentity.mockReturnValue(new Promise(() => {}));
    await mount();
    expect(container.textContent).toContain('What your agent can use');
  });

  it('uses the persona\'s configured name once it loads', async () => {
    allowed.getExtensions.mockResolvedValue({ extensions: [] });
    allowed.getIdentity.mockResolvedValue({
      first_name: 'Henry', last_name: null, nickname: null, display_name: 'Henry',
      traits: [], tone: '', opening_greeting: '', voice_id: null,
    });
    await mount();
    expect(container.textContent).toContain('What Henry can use');
  });

  it('says plainly when nothing is connected, distinct from a loading state', async () => {
    allowed.getExtensions.mockResolvedValue({ extensions: [{ enabled: false, type: 'stdio', name: 'brave' }] });
    allowed.getIdentity.mockResolvedValue({ first_name: '', last_name: null, nickname: null, display_name: '', traits: [], tone: '', opening_greeting: '', voice_id: null });
    await mount();
    expect(container.textContent).toContain('Nothing connected yet');
  });

  it('lists each enabled tool by its display name', async () => {
    allowed.getExtensions.mockResolvedValue({
      extensions: [
        { enabled: true, type: 'stdio', name: 'brave', display_name: 'Brave Search' },
        { enabled: false, type: 'stdio', name: 'unused', display_name: 'Unused Tool' },
      ],
    });
    allowed.getIdentity.mockResolvedValue({ first_name: 'Henry', last_name: null, nickname: null, display_name: 'Henry', traits: [], tone: '', opening_greeting: '', voice_id: null });
    await mount();
    expect(container.querySelector('[data-testid="can-use-brave"]')?.textContent).toBe('Brave Search');
    expect(container.querySelector('[data-testid="can-use-unused"]')).toBeNull();
  });
});

it('distinguishes failed reads from no tools and retries without changing connections', async () => {
  allowed.getExtensions.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce({ extensions: [] });
  allowed.getIdentity.mockResolvedValue({ first_name: null });
  await mount();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not load tools');
  const retry = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Retry')!;
  await act(async () => retry.click());
  expect(container.querySelector('[role="alert"]')).toBeNull();
  expect(allowed.getExtensions).toHaveBeenCalledTimes(2);
});
