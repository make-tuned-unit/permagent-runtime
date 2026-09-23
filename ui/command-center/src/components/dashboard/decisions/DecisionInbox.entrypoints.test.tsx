/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ inbox: {} as unknown, openDecisionInbox: vi.fn(() => false) }));
vi.mock('./useDecisions', () => ({ useDecisions: () => state.inbox }));
vi.mock('../../../lib/api', () => ({
  api: { getCouncilLatest: vi.fn().mockResolvedValue({ report: { headline: 'Weekly report', markdown: 'Report' }, positions: [], openActions: 2 }) },
  apiFetch: vi.fn().mockResolvedValue({ items: [{ id: 'event', ts: '2026-09-22', kind: 'decision_created', actor: 'agent', title: 'Review journal decision', ref_kind: 'decision', ref_id: 'a' }] }),
}));
vi.mock('../../../lib/store', () => ({
  navigateToTool: vi.fn(),
  useCommandCenter: (select: (s: Record<string, unknown>) => unknown) => select({
    pendingDecisionInbox: false, clearPendingDecisionInbox: vi.fn(), openDecisionInbox: state.openDecisionInbox,
    openGoalDetail: vi.fn(), closeGoalDetail: vi.fn(), goalDetail: null,
  }),
}));
vi.mock('../../../hooks/useBrowserNavigate', () => ({ useBrowserNavigate: () => vi.fn() }));
vi.mock('../../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ activeCount: 0, loaded: true }) }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./DiscussionPane', () => ({ DiscussionPane: () => <aside>Isolated discussion</aside> }));
vi.mock('./DecisionItem', () => ({ DecisionActions: () => <button>Approve</button>, badgeFor: () => ({ label: 'approval' }) }));
vi.mock('./EvidenceDigest', () => ({ EvidenceDigest: () => null }));
vi.mock('./client', () => ({ decisionsClient: {} }));

import { DecisionsCard } from '../cards/DecisionsCard';
import { CouncilCard } from '../cards/CouncilCard';
import { TimelineCard } from '../cards/TimelineCard';
import { ApprovalsStrip } from '../../settings/SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  state.inbox = {
    data: { decisions: ['a', 'b'].map(id => ({ id, tier: 2, kind: 'future', headline: `Decision ${id}`, detail: 'Details', created_at: '2026-01-01', payload: null })),
      total_pending: 2, handled_count: 0, goals_in_flight: 0, goals_needing_attention: 0, attention_goals: [], oldest_pending_at: null },
    loading: false, error: false, showAll: vi.fn().mockResolvedValue(undefined), refresh: vi.fn(), loadHistory: vi.fn(),
  };
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

it.each(['Home', 'Council', 'Timeline', 'Settings fallback'])('every entry point opens the same one-at-a-time review: %s (ruling 2)', async entry => {
  await act(async () => root.render(entry === 'Home' ? <DecisionsCard /> : entry === 'Council' ? <CouncilCard /> : entry === 'Timeline' ? <TimelineCard /> : <ApprovalsStrip />));
  const opener = entry === 'Home' ? container.querySelector('[aria-label="Open the decision inbox"]')
    : entry === 'Timeline' ? Array.from(container.querySelectorAll('div')).find(el => el.textContent === 'Review journal decision')
    : Array.from(container.querySelectorAll('button')).find(el => el.textContent?.includes(entry === 'Council' ? 'open actions' : 'Open Decisions on Home'));
  expect(opener).toBeTruthy();
  await act(async () => (opener as HTMLElement).click());
  expect(container.querySelectorAll('[data-testid="decision-review"]').length).toBe(1);
  expect(container.querySelectorAll('[data-testid="decision-a"]').length).toBe(1);
  expect(container.querySelector('[data-testid="decision-b"]')).toBeNull();
  expect(container.textContent).toContain('1 of 2');
});
