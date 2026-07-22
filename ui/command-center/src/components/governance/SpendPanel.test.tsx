/**
 * @vitest-environment jsdom
 *
 * Governance → Spend wiring. Pins that the panel is REAL, not decorative:
 *   1. it renders the running total + tokens from GET /api/governance/spend,
 *   2. it lists per-project and per-session spend from that same payload,
 *   3. saving the budget round-trips to POST /api/governance/budget.
 *
 * `../../lib/api` is mocked (the DataPanel.consent pattern) so mounting touches
 * no network.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getSpend: vi.fn(),
    setBudget: vi.fn(),
  },
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { SpendPanel } from './SpendPanel';
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getSpendMock = vi.mocked(api.getSpend);
const setBudgetMock = vi.mocked(api.setBudget);

const SNAPSHOT = {
  runningTotalUsd: 5.25,
  totalTokens: 3400,
  sessionCount: 2,
  budget: {
    session: { soft: 10, gate: 25, hard: 50 },
    task: { soft: 2, gate: 5, hard: 10 },
  },
  sessions: [
    { id: 's1', name: 'build session', workingDir: '/proj/a', sessionType: 'user', costUsd: 5.0, tokens: 3000, updatedAt: '2026-07-22T00:00:00Z', band: 'ok' },
    { id: 's2', name: 'chat session', workingDir: '/proj/b', sessionType: 'user', costUsd: 0.25, tokens: 400, updatedAt: '2026-07-22T00:00:00Z', band: 'ok' },
  ],
  projects: [
    { path: '/proj/a', label: 'Alpha', costUsd: 5.0, tokens: 3000, sessionCount: 1 },
    { path: '/proj/b', label: 'b', costUsd: 0.25, tokens: 400, sessionCount: 1 },
  ],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getSpendMock.mockReset();
  setBudgetMock.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe('SpendPanel', () => {
  it('renders running total, tokens, projects, and sessions from the real spend read', async () => {
    getSpendMock.mockResolvedValue(SNAPSHOT as never);
    await act(async () => { root.render(<SpendPanel />); });
    await flush();

    expect(getSpendMock).toHaveBeenCalled();
    const text = container.textContent ?? '';
    expect(text).toContain('$5.25');   // running total
    expect(text).toContain('3.4k');    // total tokens, compacted
    expect(text).toContain('Alpha');   // per-project label
    expect(text).toContain('build session'); // per-session name
  });

  it('saves the budget through POST /api/governance/budget', async () => {
    getSpendMock.mockResolvedValue(SNAPSHOT as never);
    setBudgetMock.mockResolvedValue({
      session: { soft: 8, gate: 25, hard: 50 },
      task: { soft: 2, gate: 5, hard: 10 },
    } as never);
    await act(async () => { root.render(<SpendPanel />); });
    await flush();

    // Change the soft input, then click "Save cap".
    const softInput = container.querySelector('input[aria-label="Session Warn (soft) ceiling in USD"]') as HTMLInputElement;
    expect(softInput).toBeTruthy();
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
      setter.call(softInput, '8');
      softInput.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const saveBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Save cap')!;
    expect(saveBtn).toBeTruthy();
    await act(async () => { saveBtn.click(); });
    await flush();

    expect(setBudgetMock).toHaveBeenCalledTimes(1);
    const patch = setBudgetMock.mock.calls[0][0];
    expect(patch.session?.soft).toBe(8);
  });
});
