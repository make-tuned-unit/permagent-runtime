/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../../lib/useLiveGoals', () => ({
  useLiveGoals: () => ({ activeCount: 0, loaded: true }),
}));
vi.mock('../../settings/useSettings', () => ({
  usePersona: () => ({ data: null }),
}));
vi.mock('./DecisionItem', () => ({
  DecisionItem: () => <div data-testid="decision-item" />,
}));
vi.mock('./client', () => ({
  decisionsClient: { cancelGoal: vi.fn() },
}));

import { getThemedColors } from '../../../styles/tokens';
import { DecisionInbox } from './DecisionInbox';
import type { useDecisions } from './useDecisions';
import type { DecisionsResponse } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

type Inbox = ReturnType<typeof useDecisions>;

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

function makeInbox(overrides: Partial<Inbox> = {}): Inbox {
  return {
    data: null,
    loading: false,
    error: false,
    refresh: vi.fn().mockResolvedValue(undefined),
    showAll: vi.fn().mockResolvedValue(undefined),
    answer: vi.fn(),
    discardStaged: vi.fn().mockResolvedValue(undefined),
    loadHistory: vi.fn().mockResolvedValue([]),
    ...overrides,
  } as Inbox;
}

function render(inbox: Inbox) {
  act(() => root.render(<DecisionInbox inbox={inbox} onClose={vi.fn()} />));
}

function asRendered(color: string): string {
  const probe = document.createElement('span');
  probe.style.color = color;
  return probe.style.color;
}

const emptyInbox: DecisionsResponse = {
  decisions: [],
  total_pending: 0,
  handled_count: 0,
  goals_in_flight: 0,
  goals_needing_attention: 0,
  attention_goals: [],
  oldest_pending_at: null,
};

describe('DecisionInbox explanatory copy', () => {
  it('uses readable muted copy for a cold connection failure and keeps retry actionable', async () => {
    const refresh = vi.fn().mockResolvedValue(undefined);
    const inbox = makeInbox({ error: true, refresh });
    render(inbox);

    const detail = Array.from(container.querySelectorAll('div')).find(
      el => el.textContent === 'This is a connection problem, not an empty inbox.',
    ) as HTMLDivElement | undefined;
    expect(detail).toBeDefined();
    expect(detail!.style.color).toBe(asRendered(getThemedColors().textMuted));
    expect(detail!.style.color).not.toBe(asRendered(getThemedColors().textDim));

    const retry = Array.from(container.querySelectorAll('button')).find(
      button => button.textContent === 'Retry',
    ) as HTMLButtonElement | undefined;
    expect(retry).toBeDefined();
    await act(async () => retry!.click());
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('uses readable muted copy for the empty-state activity explanation', () => {
    render(makeInbox({ data: emptyInbox }));

    const detail = Array.from(container.querySelectorAll('div')).find(
      el => el.textContent === '0 goals in flight.',
    ) as HTMLDivElement | undefined;
    expect(detail).toBeDefined();
    expect(detail!.style.color).toBe(asRendered(getThemedColors().textMuted));
  });

  it('keeps the history action on the readable semantic without changing navigation', async () => {
    const loadHistory = vi.fn().mockResolvedValue([]);
    const inbox = makeInbox({
      data: { ...emptyInbox, total_pending: 1 },
      loadHistory,
    });
    render(inbox);

    const history = Array.from(container.querySelectorAll('button')).find(
      button => button.textContent === 'History →',
    ) as HTMLButtonElement | undefined;
    expect(history).toBeDefined();
    expect(history!.style.getPropertyValue('--pa-btn-fg')).toBe(getThemedColors().textMuted);
    await act(async () => { history!.click(); });
    expect(loadHistory).toHaveBeenCalledTimes(1);
  });
});
