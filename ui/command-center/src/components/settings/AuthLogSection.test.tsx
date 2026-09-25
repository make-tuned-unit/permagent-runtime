/** @vitest-environment jsdom
 *
 * Wiring test for Privacy → control-plane auth log (ST3, dagK settings pass):
 * `/api/security/auth-log` had no UI. Covers loading/empty/error/populated and
 * that the raw route-class enum is worded in plain language (R3).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  getAuthLog: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`AuthLogSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { AuthLogSection } from './AuthLogSection';

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
  await act(async () => { root.render(<AuthLogSection />); });
}

describe('AuthLogSection', () => {
  it('shows a loading state before the log resolves', async () => {
    allowed.getAuthLog.mockReturnValue(new Promise(() => {}));
    await mount();
    expect(container.textContent).toContain('Loading auth log');
  });

  it('shows an honest error state, not empty', async () => {
    allowed.getAuthLog.mockRejectedValue(new Error('down'));
    await mount();
    expect(container.textContent).toContain("Couldn't read the auth log");
  });

  it('shows the empty state when nothing has been recorded', async () => {
    allowed.getAuthLog.mockResolvedValue([]);
    await mount();
    expect(container.textContent).toContain('No events recorded yet');
  });

  it('renders a row in plain language, not the raw route-class enum', async () => {
    allowed.getAuthLog.mockResolvedValue([{
      id: 'e1', ts: '2026-09-25T09:00:00Z', outcome: 'admitted', principal: 'device-abc',
      credential: 'device_token', class: 'execute', method: 'POST', path: '/api/agent/run', status: 200, peer: '127.0.0.1',
    }]);
    await mount();
    expect(container.querySelector('[data-testid="auth-log-row"]')).toBeTruthy();
    expect(container.textContent).toContain('ran code');
    expect(container.textContent).not.toContain('"execute"');
    expect(container.textContent).toContain('device-abc');
  });

  it('refreshes on demand', async () => {
    allowed.getAuthLog.mockResolvedValue([]);
    await mount();
    const btn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Refresh') as HTMLButtonElement;
    await act(async () => { btn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.getAuthLog).toHaveBeenCalledTimes(2);
  });
});
