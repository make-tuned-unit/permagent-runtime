/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getConfig: vi.fn(),
    upsertConfig: vi.fn(),
  },
  apiFetch: vi.fn(),
}));

// The approvals strip reuses the shared Decision-Inbox hook + overlay; mock
// both so mounting AutonomyPanel opens no WebSocket and fetches nothing.
vi.mock('../dashboard/decisions/useDecisions', () => ({
  useDecisions: vi.fn(() => ({ data: null, loading: true, error: false })),
}));
vi.mock('../dashboard/decisions/DecisionInbox', () => ({
  DecisionInbox: () => null,
}));

// The spend-cap pointer navigates to a WORKSPACE, so the store's navigator is
// what it calls. Mocked to keep this a pure render test; `true` is the normal
// case (the Financier tab is in the sidebar).
vi.mock('../../lib/store', () => ({
  useCommandCenter: Object.assign(vi.fn(() => ({})), { getState: vi.fn(() => ({})) }),
  navigateToTool: vi.fn(() => true),
}));

import { api } from '../../lib/api';
import { navigateToTool } from '../../lib/store';
import { AutonomyPanel } from './SettingsView';
import { useDecisions } from '../dashboard/decisions/useDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getConfig = vi.mocked(api.getConfig);
const upsertConfig = vi.mocked(api.upsertConfig);
const useDecisionsMock = vi.mocked(useDecisions);
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getConfig.mockReset().mockResolvedValue({ config: {}, effective_goose_mode: 'auto' } as never);
  upsertConfig.mockReset().mockResolvedValue({} as never);
  useDecisionsMock.mockReturnValue({ data: null, loading: true, error: false } as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => { root.render(<AutonomyPanel />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe('AutonomyPanel guardrail wiring', () => {
  it('persists a selectable trust level through the config endpoint', async () => {
    await mount();
    const chatBtn = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent?.includes('Chat only'),
    ) as HTMLButtonElement;
    expect(chatBtn).toBeTruthy();
    await act(async () => { chatBtn.click(); });
    expect(upsertConfig).toHaveBeenCalledWith('GOOSE_MODE', 'chat');
  });

  it('keeps the hanging per-tool modes locked (honest "Soon" guard)', async () => {
    await mount();
    const locked = Array.from(container.querySelectorAll('button')).filter(
      b => b.textContent?.includes('Ask every time') || b.textContent?.includes('Smart approve'),
    ) as HTMLButtonElement[];
    expect(locked).toHaveLength(2);
    expect(locked.every(b => b.disabled)).toBe(true);
    locked.forEach(b => b.click());
    expect(upsertConfig).not.toHaveBeenCalled();
  });

  it('shows the pending-approvals strip from the shared Decision-Inbox hook', async () => {
    useDecisionsMock.mockReturnValue({
      data: { total_pending: 3, handled_count: 0, goals_in_flight: 1, oldest_pending_at: null },
      loading: false,
      error: false,
    } as never);
    await mount();
    expect(container.textContent).toContain('Pending approvals: 3');
    expect(container.textContent).toContain('Open Decision Inbox');
  });

  it('replaced the spend-cap sliders with a link to the Financier tab', async () => {
    await mount();
    // No sliders left, and no second writer of the budget: the ceilings are set
    // in exactly one place, which is now the Financier tab.
    expect(container.querySelectorAll('input[type="range"]')).toHaveLength(0);
    const link = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent?.includes('Open the Financier'),
    ) as HTMLButtonElement;
    expect(link).toBeTruthy();
    await act(async () => { link.click(); });
    expect(navigateToTool).toHaveBeenCalledWith('financier');
  });

  it('says so when no workspace hosts the Financier tab, rather than doing nothing', async () => {
    // `navigateToTool` returns false when the sidebar has no such workspace —
    // the state an existing install is in until the startup backfill has run.
    // A button that silently no-ops is the failure this branch exists to avoid.
    vi.mocked(navigateToTool).mockReturnValueOnce(false);
    await mount();
    const link = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent?.includes('Open the Financier'),
    ) as HTMLButtonElement;
    await act(async () => { link.click(); });
    expect(container.textContent).toContain('not in your sidebar');
  });

  it('dropped the preview "Always confirm before…" toggles', async () => {
    await mount();
    expect(container.textContent).not.toContain('Always confirm before…');
  });
});
