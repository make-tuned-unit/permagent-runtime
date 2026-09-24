/** @vitest-environment jsdom */
import { act, Simulate } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  goalDetail: null as { projectId: string; cardId: string } | null,
  apiFetch: vi.fn(), openGoalDetail: vi.fn(), closeGoalDetail: vi.fn(), discussDecision: vi.fn(),
  answer: vi.fn(), showAll: vi.fn(), refresh: vi.fn(), discard: vi.fn(), history: vi.fn(),
}));
vi.mock('../../../lib/api', () => ({ apiFetch: mocks.apiFetch }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (select: (state: Record<string, unknown>) => unknown) => select(mocks),
}));
vi.mock('../../../lib/notifications', () => ({ toast: vi.fn() }));
vi.mock('../../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ activeCount: 0, loaded: true }) }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./DiscussionPane', () => ({ DiscussionPane: ({ decisionId }: { decisionId: string }) => <aside>Discussion {decisionId}</aside> }));
vi.mock('./EvidenceDigest', () => ({ EvidenceDigest: () => <div>Dispatch digest</div> }));
vi.mock('./client', () => ({ decisionsClient: { dispatchEvidence: vi.fn().mockResolvedValue(null), cancelGoal: vi.fn() } }));

import { MIN_PENDING_MS } from '../../common/Button';
import { DecisionInbox } from './DecisionInbox';
import type { Decision, DecisionsResponse } from './types';
import type { useDecisions } from './useDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
function decision(id: string, patch: Partial<Decision> = {}): Decision {
  return { id, kind: 'risk_gate', tier: 2, headline: `Headline ${id}`, detail: '<b>Plain detail</b>',
    goal_id: null, project_id: null, payload: null, rank: null, status: 'open', answer: null,
    answer_note: null, answer_choice_id: null, answer_input: null, acted_by: null,
    created_at: '2026-01-01', resolved_at: null, ...patch };
}
function inbox(rows: Decision[], extra: Partial<DecisionsResponse> = {}): ReturnType<typeof useDecisions> {
  return { data: { decisions: rows, total_pending: rows.length, handled_count: 0, goals_in_flight: 0,
    goals_needing_attention: 0, attention_goals: [], oldest_pending_at: null, ...extra }, loading: false, error: false,
    answer: mocks.answer, refresh: mocks.refresh, showAll: mocks.showAll, discardStaged: mocks.discard, loadHistory: mocks.history };
}
function render(rows: Decision[], placement: 'center' | 'contained' = 'center', extra: Partial<DecisionsResponse> = {}) {
  act(() => root.render(<DecisionInbox inbox={inbox(rows, extra)} onClose={() => {}} placement={placement} />));
}
function button(label: string) {
  const found = Array.from(document.body.querySelectorAll('button')).find(b => !b.closest('[hidden]') && (b.getAttribute('aria-label') === label || b.textContent === label));
  expect(found, label).toBeTruthy();
  return found!;
}
async function click(label: string) { await act(async () => button(label).click()); }
function key(target: Element, value: string, init: KeyboardEventInit = {}) {
  act(() => target.dispatchEvent(new KeyboardEvent('keydown', { key: value, bubbles: true, ...init })));
}
function current() { return document.body.querySelector('[data-testid^="decision-"]:not([data-testid="decision-review"])')?.getAttribute('data-testid'); }
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  mocks.goalDetail = null;
  mocks.openGoalDetail.mockImplementation((projectId: string, cardId: string) => { mocks.goalDetail = { projectId, cardId }; });
  mocks.closeGoalDetail.mockImplementation(() => { mocks.goalDetail = null; });
  mocks.showAll.mockResolvedValue(undefined); mocks.refresh.mockResolvedValue(undefined); mocks.history.mockResolvedValue([]);
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.useRealTimers(); });

describe('D1 review acceptance', () => {
  it('loads all pending decisions and reaches beyond the first ten (ruling 4)', async () => {
    const rows = Array.from({ length: 12 }, (_, i) => decision(String(i).padStart(2, '0')));
    render(rows.slice(0, 10), 'center', { total_pending: 12 });
    expect(mocks.showAll).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain('Loading all pending decisions');
    expect(current()).toBeUndefined();
    render(rows);
    for (let i = 0; i < 11; i++) await click('Next decision');
    expect(current()).toBe('decision-11');
    expect(button('Next decision').disabled).toBe(true);
    expect(document.body.textContent).toContain('12 of 12');
    expect(document.body.querySelectorAll('[aria-hidden="true"] span').length).toBeLessThanOrEqual(7);
  });
  it.each(['center', 'contained'] as const)('keyboard respects editing and %s placement (rulings 2, 11)', async placement => {
    render([decision('a'), decision('b')], placement);
    const shell = document.body.querySelector('[role="dialog"]')!;
    expect(shell.getAttribute('aria-modal')).toBe(placement === 'center' ? 'true' : null);
    const note = document.body.querySelector('textarea')!;
    act(() => Simulate.change(note, { target: { value: 'j' } } as never));
    key(note, 'j'); key(note, 'ArrowRight');
    expect(current()).toBe('decision-a');
    key(shell, 'j', { isComposing: true }); key(shell, 'j', { ctrlKey: true });
    key(button('Approve'), 'j');
    expect(current()).toBe('decision-a');
    key(shell, 'j');
    expect(current()).toBe('decision-b');
    key(shell, 'k');
    expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('j');
    expect(mocks.answer).not.toHaveBeenCalled();
  });
  it('Later retains notes, never answers and leaves deferred work reachable (rulings 5, 12)', async () => {
    render([decision('a'), decision('b')]);
    act(() => Simulate.change(document.body.querySelector('textarea')!, { target: { value: 'Keep this draft' } } as never));
    await click('Later');
    expect(current()).toBe('decision-b');
    await click('Next decision');
    expect(current()).toBe('decision-a');
    expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('Keep this draft');
    expect(mocks.answer).not.toHaveBeenCalled();
    expect(mocks.discussDecision).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain('Discuss with');
  });
  it('History and parked goals stay outside the count and preserve focus (ruling 3)', async () => {
    const rows = [decision('a'), decision('b')];
    render(rows, 'center', { attention_goals: [{ id: 'g', project_id: 'p', title: 'Parked title', state_binding: 'failed', reason: 'Needs help', updated_at: '' }] });
    await click('Next decision');
    const parked = document.body.querySelector('details')!;
    expect(parked.open).toBe(false);
    expect(parked.querySelector('summary')!.textContent).toBe('Parked goals · 1');
    act(() => parked.querySelector('summary')!.click());
    expect(parked.open).toBe(true);
    expect(parked.textContent).toContain('Parked title');
    expect(parked.textContent).toContain('Needs help');
    await click('History →');
    expect(document.body.querySelector('[data-testid="decision-review"]')).toBeNull();
    await click('← Back');
    expect(current()).toBe('decision-b');
    expect(document.body.textContent).toContain('2 of 2');
  });
  it('unknown effects render record-only text and decision strings stay plain (ruling 8)', () => {
    render([decision('future', { kind: 'future_kind' })]);
    expect(document.body.textContent).toContain('Records your answer; no automatic effect');
    expect(document.body.textContent).toContain('<b>Plain detail</b>');
    expect(document.body.querySelector('[data-testid="decision-future"] b')).toBeNull();
  });
  it('opens the exact Council session endpoint with a return preserving the note (ruling 10; absent store opener)', async () => {
    mocks.apiFetch.mockResolvedValue({ report: { headline: 'Exact report', markdown: 'Session evidence' } });
    render([decision('c', { kind: 'council_action', payload: { session_id: 'session/older' } })]);
    act(() => Simulate.change(document.body.querySelector('textarea')!, { target: { value: 'Council note' } } as never));
    await click('Open council report');
    expect(mocks.apiFetch).toHaveBeenCalledWith('/api/council/session%2Folder');
    expect(document.body.textContent).toContain('Exact report');
    await click('← Back to decision');
    expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('Council note');
  });
  it('dispatch evidence opens its exact project and goal (ruling 10)', async () => {
    render([decision('d', { project_id: 'p-exact', goal_id: 'g-exact' })]);
    await click('Open goal evidence');
    expect(mocks.openGoalDetail).toHaveBeenCalledWith('p-exact', 'g-exact');
  });
  it('external resolution announces and advances using the previous ordering (ruling 5)', async () => {
    render([decision('a'), decision('b'), decision('c')]);
    await click('Next decision');
    render([decision('a'), decision('c')]);
    expect(current()).toBe('decision-c');
    expect(document.body.textContent).toContain('Headline b is no longer pending.');
    expect(document.body.textContent).toContain('Decision 2 of 2');
  });
  it('failed answers preserve the note and successful follow-through failures remain after advancement (ruling 12)', async () => {
    const rows = [decision('a'), decision('b')];
    render(rows);
    act(() => Simulate.change(document.body.querySelector('textarea')!, { target: { value: 'audit note' } } as never));
    mocks.answer.mockRejectedValueOnce(new Error('Offline'));
    await click('Approve'); await click('Confirm approve');
    expect(document.body.textContent).toContain('Offline');
    expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('audit note');
    expect(mocks.answer).toHaveBeenCalledWith('a', { answer: 'approve', note: 'audit note' });
    mocks.answer.mockResolvedValueOnce({ ok: true, decision: rows[0], effect: null, effect_error: 'Write failed' });
    // Navigation preserves the note and permits a fresh confirmation.
    await click('Next decision'); await click('Previous decision');
    await act(async () => { await vi.advanceTimersByTimeAsync(MIN_PENDING_MS + 1); });
    await click('Approve'); await click('Confirm approve');
    render([rows[1]]);
    expect(document.body.textContent).toContain('Follow-through failed: Write failed');
  });
});

it('a 409 refresh distinguishes a still-open verification hold and retains the note (ruling 12)', async () => {
  render([decision('held')]);
  act(() => Simulate.change(document.body.querySelector('textarea')!, { target: { value: 'hold note' } } as never));
  mocks.answer.mockResolvedValueOnce({ ok: false, conflict: true });
  await click('Approve'); await click('Confirm approve');
  expect(mocks.refresh).toHaveBeenCalled();
  expect(document.body.textContent).toContain('Decision is still open. Verification may not be ready');
  expect(document.body.textContent).not.toContain('Someone already answered');
  expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('hold note');
});

it('missing effects and pause episode evidence are explicit (rulings 8–10)', () => {
  render([decision('paused', { kind: 'unblock', payload: { target: { type: 'scheduled_job', job_id: 'job-7' }, pause_episode_id: 'episode-9' } })]);
  expect(document.body.textContent).toContain('missing:');
  expect(document.body.textContent).toContain('Job job-7 · pause episode episode-9 — no viewer yet.');
});

it('container width, rather than viewport width, controls the two columns (rulings 2, 11)', () => {
  let resize: ResizeObserverCallback | undefined;
  const observer = { observe: vi.fn(), unobserve: vi.fn(), disconnect: vi.fn() };
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe = observer.observe;
    unobserve = observer.unobserve;
    disconnect = observer.disconnect;
  });
  try {
    render([decision('a')], 'contained');
    const width = (value: number) => act(() => resize!([{ contentRect: { width: value } } as ResizeObserverEntry], observer as ResizeObserver));
    const grid = () => Array.from(document.body.querySelectorAll<HTMLElement>('div')).find(el => el.style.display === 'grid')!;
    width(900);
    expect(grid().style.gridTemplateColumns).toBe('minmax(0, 7fr) minmax(0, 5fr)');
    width(400);
    expect(grid().style.gridTemplateColumns).toBe('minmax(0, 1fr)');
  } finally { vi.unstubAllGlobals(); }
});

it.each(['center', 'contained'] as const)('goal evidence suspends the %s inbox focus trap and returning keeps focus and notes (rulings 10, 11)', async placement => {
  const rows = [decision('a'), decision('b', { project_id: 'p', goal_id: 'g' })];
  render(rows, placement);
  await click('Next decision');
  expect(current()).toBe('decision-b');
  act(() => Simulate.change(document.body.querySelector('textarea')!, { target: { value: 'proof note' } } as never));
  await click('Open goal evidence');
  expect(mocks.openGoalDetail).toHaveBeenCalledWith('p', 'g');
  // The selector mock has no subscription; render the actual store transition.
  render(rows, placement);
  const shell = document.body.querySelector('[role="dialog"]')!;
  expect(shell.getAttribute('aria-modal')).toBeNull();
  expect((shell.parentElement as HTMLElement).style.display).toBe('none');
  // GoalDetailModalHost closes evidence through this store action.
  act(() => mocks.closeGoalDetail());
  render(rows, placement);
  expect(current()).toBe('decision-b');
  expect((document.body.querySelector('textarea') as HTMLTextAreaElement).value).toBe('proof note');
  expect(document.body.querySelector('[role="dialog"]')!.getAttribute('aria-modal')).toBe(placement === 'center' ? 'true' : null);
});
