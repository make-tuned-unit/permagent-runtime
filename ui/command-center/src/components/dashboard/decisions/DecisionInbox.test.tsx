/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../../lib/useLiveGoals', () => ({
  useLiveGoals: () => ({ activeCount: 0, loaded: true }),
}));
vi.mock('../../settings/useSettings', () => ({
  // A neutral fixture name: the inbox renders the persona seam, never a literal.
  usePersona: () => ({ data: { display_name: 'Nova' } }),
}));
vi.mock('./client', () => ({
  decisionsClient: { cancelGoal: vi.fn() },
}));

import { getThemedColors } from '../../../styles/tokens';
import { DecisionInbox } from './DecisionInbox';
import type { useDecisions } from './useDecisions';
import type { DecisionsResponse, HistoryItem } from './types';

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

describe('DecisionInbox states (R2)', () => {
  it('a cold connection failure is an error StateBlock with retry — never the all-clear', async () => {
    const refresh = vi.fn().mockResolvedValue(undefined);
    render(makeInbox({ error: true, refresh }));
    expect(document.body.textContent).toContain("Couldn't reach the decision inbox.");
    expect(document.body.textContent).toContain('This is a connection problem, not an empty inbox.');
    expect(document.body.textContent).not.toContain('Nothing needs you');
    const retry = Array.from(document.body.querySelectorAll('button')).find(b => b.textContent === 'Try again');
    await act(async () => retry!.click());
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('a cold load is a skeleton in the final shape, not text in the void', () => {
    render(makeInbox({ loading: true }));
    expect(document.body.querySelector('[data-testid="decision-loading"]')).not.toBeNull();
    expect(document.body.textContent).not.toContain('Checking with');
  });

  it('empty is a calm StateBlock with the agent\'s handled summary and a way to review it', async () => {
    const loadHistory = vi.fn().mockResolvedValue([]);
    const since = new Date(); since.setHours(0, 0, 0, 0);
    render(makeInbox({ data: { ...emptyInbox, handled_count: 40, handled: { count: 4, since: since.toISOString() } }, loadHistory }));
    expect(document.body.textContent).toContain('Nothing needs you');
    expect(document.body.textContent).toContain('Nova handled 4 items since 12 AM.');
    expect(document.body.textContent).not.toMatch(/overnight/);
    const review = Array.from(document.body.querySelectorAll('button')).find(b => b.textContent === 'Review what Nova did')!;
    await act(async () => { review.click(); });
    expect(loadHistory).toHaveBeenCalledTimes(1);
  });

  // The legacy total counts the user's own answers (and expiries) too; it
  // used to read "Nova has handled 4 items so far".
  it('empty never credits the agent with the any-actor legacy total', () => {
    render(makeInbox({ data: { ...emptyInbox, handled_count: 4 } }));
    expect(document.body.textContent).not.toContain('Nova has handled');
    expect(Array.from(document.body.querySelectorAll('button')).some(b => b.textContent === 'Review what Nova did')).toBe(false);
    expect(document.body.textContent).toContain('0 goals in flight.');
  });

  it('with nothing handled, empty says how much is in flight', () => {
    render(makeInbox({ data: emptyInbox }));
    const detail = Array.from(document.body.querySelectorAll('div')).find(el => el.textContent === '0 goals in flight.');
    expect(detail).toBeDefined();
    expect(detail!.style.color).toBe(asRendered(getThemedColors().textMuted));
  });
});

describe('the header handled link (U18)', () => {
  it('names the daemon window and opens History', async () => {
    const loadHistory = vi.fn().mockResolvedValue([]);
    const since = new Date(); since.setHours(0, 0, 0, 0);
    render(makeInbox({ data: { ...emptyInbox, handled_count: 50, handled: { count: 3, since: since.toISOString() } }, loadHistory }));
    const link = document.body.querySelector('[data-testid="handled-link"]') as HTMLButtonElement;
    expect(link.textContent).toMatch(/^3 handled since 12 AM ›$/);
    await act(async () => { link.click(); });
    expect(loadHistory).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain('What Nova did');
  });

  // Was "7 handled ›" opening "What Nova did" (agent-only rows): the count
  // and the list it opened disagreed whenever the user had answered anything.
  it('falls back to "N resolved" with no window, and opens everything', async () => {
    const mine = { id: 'mine', seq: 1, headline: 'Answered by you', acted_by: 'user', tier: 2, answer: 'approve', created_at: '2026-01-01', outcome: '' } as unknown as HistoryItem;
    const loadHistory = vi.fn().mockResolvedValue([mine]);
    render(makeInbox({ data: { ...emptyInbox, handled_count: 7 }, loadHistory }));
    const link = document.body.querySelector('[data-testid="handled-link"]') as HTMLButtonElement;
    expect(link.textContent).toBe('7 resolved ›');
    await act(async () => { link.click(); });
    expect(document.body.textContent).not.toContain('What Nova did');
    expect(document.body.textContent).toContain('Answered by you');
  });

  it('the windowed agent view lists what the agent did inside the window it counted', async () => {
    const since = new Date(); since.setHours(0, 0, 0, 0);
    const row = (id: string, resolved: Date) => ({
      id, seq: 1, headline: `Row ${id}`, acted_by: 'system', tier: 2, answer: 'approve',
      created_at: resolved.toISOString(), resolved_at: resolved.toISOString(), audit_created_at: resolved.toISOString(), outcome: '',
    }) as unknown as HistoryItem;
    const before = new Date(since.getTime() - 3_600_000);
    const after = new Date(since.getTime() + 60_000);
    const loadHistory = vi.fn().mockResolvedValue([row('old', before), row('new', after)]);
    render(makeInbox({ data: { ...emptyInbox, handled_count: 9, handled: { count: 1, since: since.toISOString() } }, loadHistory }));
    await act(async () => { (document.body.querySelector('[data-testid="handled-link"]') as HTMLButtonElement).click(); });
    expect(document.body.textContent).toContain('What Nova did');
    expect(document.body.textContent).toContain('Row new');
    expect(document.body.textContent).not.toContain('Row old');
  });

  it('a failed history read is an error, not an empty record', async () => {
    const loadHistory = vi.fn().mockRejectedValue(new Error('down'));
    render(makeInbox({ data: emptyInbox, loadHistory }));
    const link = document.body.querySelector('[data-testid="handled-link"]') as HTMLButtonElement;
    expect(link.textContent).toBe('History ›');
    await act(async () => { link.click(); });
    expect(document.body.textContent).toContain("Couldn't load the decision history.");
  });
});
