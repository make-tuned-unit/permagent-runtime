/** @vitest-environment jsdom */
/**
 * A held answer survives the app quitting inside its Undo window. Cmd+Q ends
 * the process with no pagehide, no visibilitychange and no React unmount, so
 * nothing in the page can send it: the hold is stored when it is made, and the
 * next launch sends it once the daemon answers a read.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  list: vi.fn(), answer: vi.fn(), toast: vi.fn(), retire: vi.fn(), emit: vi.fn(), firstPageAnswer: vi.fn(),
}));
vi.mock('../../../lib/api', () => ({ apiFetch: vi.fn(), eventsWsUrl: async () => 'ws://localhost/events' }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: Object.assign(
    (select: (state: Record<string, unknown>) => unknown) => select({ goalDetail: null }),
    { getState: () => ({ bumpPeople: () => {} }) },
  ),
}));
vi.mock('../../../lib/notifications', () => ({ toast: mocks.toast, retireToastAction: mocks.retire }));
vi.mock('../../../lib/emitActivity', () => ({ emitActivity: mocks.emit }));
vi.mock('../../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ activeCount: 0, loaded: true }) }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./DiscussionPane', () => ({ DiscussionPane: () => null }));
vi.mock('./EvidenceDigest', () => ({ EvidenceDigest: () => null }));
vi.mock('./client', () => ({
  decisionsClient: { list: mocks.list, answer: mocks.answer, dispatchEvidence: async () => null, cancelGoal: vi.fn() },
}));

import {
  HELD_STORE_KEY, ORPHAN_AFTER_MS, forgetHold, nextOrphanDueIn, rememberHold, reportOrphan, sendOrphanedHolds,
  type OrphanOutcome, type StoredHold,
} from './DecisionReview.heldStore';
import type { Decision, DecisionsResponse } from './types';
import type { useDecisions } from './useDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class FakeWebSocket {
  onmessage: unknown = null;
  onerror: unknown = null;
  onclose: unknown = null;
  close() {}
}

function decision(id: string): Decision {
  return { id, kind: 'risk_gate', tier: 2, headline: `Headline ${id}`, detail: '', goal_id: null, project_id: null,
    payload: { action_class: 'cc_shell', description: 'Run a script' }, rank: null, status: 'open', answer: null,
    answer_note: null, answer_choice_id: null, answer_input: null, acted_by: null, created_at: '2026-01-01', resolved_at: null };
}
function response(rows: Decision[], total = rows.length): DecisionsResponse {
  return { decisions: rows, total_pending: total, handled_count: 0, goals_in_flight: 0, goals_needing_attention: 0,
    attention_goals: [], oldest_pending_at: null };
}
const stored = () => JSON.parse(localStorage.getItem(HELD_STORE_KEY) ?? '[]') as StoredHold[];
/** A hold a page that is gone left behind, `age` ms ago. */
function leftBehind(id: string, age: number, extra: Partial<StoredHold> = {}): StoredHold {
  return { id, body: { answer: 'approve' }, label: 'Allowed', title: `Headline ${id}`, owner: 'gone-page', heldAt: Date.now() - age, ...extra };
}
const sent = (id: string) => ({ ok: true as const, decision: decision(id), effect: null, effect_error: null });

let realWebSocket: unknown;
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  localStorage.removeItem(HELD_STORE_KEY);
  realWebSocket = (globalThis as { WebSocket?: unknown }).WebSocket;
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeWebSocket;
});
afterEach(() => {
  (globalThis as { WebSocket?: unknown }).WebSocket = realWebSocket;
  vi.useRealTimers();
});

describe('the stored hold', () => {
  it("this page's own holds are never recovered here: its timer, close and pagehide own them", async () => {
    rememberHold('a', { answer: 'approve' }, 'Allowed', 'Headline a');
    vi.setSystemTime(Date.now() + ORPHAN_AFTER_MS * 2);
    const send = vi.fn();
    await sendOrphanedHolds({ open: new Set(['a']), complete: true }, send, vi.fn());
    expect(send).not.toHaveBeenCalled();
    expect(nextOrphanDueIn()).toBeNull();
    forgetHold('a');
    expect(localStorage.getItem(HELD_STORE_KEY)).toBeNull();
  });

  it("another page's hold still inside its own send time is left alone, and says when it is due", async () => {
    localStorage.setItem(HELD_STORE_KEY, JSON.stringify([leftBehind('a', 1_000)]));
    const send = vi.fn();
    await sendOrphanedHolds({ open: new Set(['a']), complete: true }, send, vi.fn());
    expect(send).not.toHaveBeenCalled();
    expect(nextOrphanDueIn()).toBe(ORPHAN_AFTER_MS - 1_000);
  });

  it('a gone page\'s hold is sent exactly once, even when two readers find it together', async () => {
    localStorage.setItem(HELD_STORE_KEY, JSON.stringify([leftBehind('a', ORPHAN_AFTER_MS)]));
    let settle!: (value: ReturnType<typeof sent>) => void;
    const send = vi.fn(() => new Promise<ReturnType<typeof sent>>(resolve => { settle = resolve; }));
    const report = vi.fn();
    const first = sendOrphanedHolds({ open: new Set(['a']), complete: true }, send, report);
    const second = sendOrphanedHolds({ open: new Set(['a']), complete: true }, send, report);
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith('a', { answer: 'approve' });
    // Claimed: another window reading now sees it as freshly held, not due.
    expect(stored()[0].heldAt).toBe(Date.now());
    settle(sent('a'));
    await Promise.all([first, second]);
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(expect.objectContaining({ kind: 'sent' }));
    expect(localStorage.getItem(HELD_STORE_KEY)).toBeNull();
  });

  it('one whose decision is no longer open already landed: dropped quietly, no second POST', async () => {
    localStorage.setItem(HELD_STORE_KEY, JSON.stringify([leftBehind('a', ORPHAN_AFTER_MS)]));
    const send = vi.fn();
    const report = vi.fn();
    await sendOrphanedHolds({ open: new Set(['b']), complete: true }, send, report);
    expect(send).not.toHaveBeenCalled();
    expect(report).toHaveBeenCalledWith(expect.objectContaining({ kind: 'landed' }));
    expect(localStorage.getItem(HELD_STORE_KEY)).toBeNull();
  });

  it('past a truncated queue it is sent anyway; a 409 then means it landed, a 409 while listed means refused', async () => {
    localStorage.setItem(HELD_STORE_KEY, JSON.stringify([leftBehind('a', ORPHAN_AFTER_MS), leftBehind('b', ORPHAN_AFTER_MS)]));
    const send = vi.fn(async () => ({ ok: false as const, conflict: true as const }));
    const outcomes: OrphanOutcome[] = [];
    await sendOrphanedHolds({ open: new Set(['b']), complete: false }, send, o => outcomes.push(o));
    expect(send).toHaveBeenCalledTimes(2);
    expect(Object.fromEntries(outcomes.map(o => [o.hold.id, o.kind]))).toEqual({ a: 'landed', b: 'refused' });
  });

  it('a send that fails is reported, and the stored copy goes (the decision is back in the queue)', async () => {
    localStorage.setItem(HELD_STORE_KEY, JSON.stringify([leftBehind('a', ORPHAN_AFTER_MS)]));
    const report = vi.fn();
    await sendOrphanedHolds({ open: new Set(['a']), complete: true }, async () => { throw new Error('daemon down'); }, report);
    expect(report).toHaveBeenCalledWith(expect.objectContaining({ kind: 'failed', stillOpen: true }));
    expect(localStorage.getItem(HELD_STORE_KEY)).toBeNull();
  });

  it('the user is told which way a recovered answer went', () => {
    const hold = leftBehind('a', 0);
    reportOrphan({ hold, kind: 'landed' });
    expect(mocks.toast).not.toHaveBeenCalled();
    reportOrphan({ hold, kind: 'sent', result: sent('a') });
    expect(mocks.toast).toHaveBeenLastCalledWith('Sent your earlier answer', expect.stringContaining('Headline a: Allowed.'), undefined, { force: true });
    reportOrphan({ hold, kind: 'refused' });
    expect(mocks.toast).toHaveBeenLastCalledWith("Your earlier answer wasn't accepted", expect.stringContaining('back in your queue'), undefined, { force: true });
    reportOrphan({ hold, kind: 'failed', error: new Error('daemon down'), stillOpen: true });
    expect(mocks.toast).toHaveBeenLastCalledWith("Couldn't send your earlier answer", 'daemon down. Headline a: Allowed. It is back in your queue.', undefined, { tone: 'danger', force: true });
  });

  it('unreadable storage reads as empty instead of throwing', async () => {
    localStorage.setItem(HELD_STORE_KEY, '{not json');
    expect(nextOrphanDueIn()).toBeNull();
    await expect(sendOrphanedHolds({ open: new Set(), complete: true }, vi.fn(), vi.fn())).resolves.toBeUndefined();
  });
});

describe('quitting the app inside the Undo window', () => {
  it('the next launch sends the held answer and says so', async () => {
    // First launch: answer a decision; the toast says Allowed.
    const { DecisionInbox } = await import('./DecisionInbox');
    mocks.firstPageAnswer.mockImplementation(async (id: string) => sent(id));
    const firstInbox = {
      data: response([decision('a'), decision('b')]), loading: false, error: false,
      answer: mocks.firstPageAnswer, refresh: vi.fn(), showAll: vi.fn(), discardStaged: vi.fn(), loadHistory: vi.fn(),
    } as unknown as ReturnType<typeof useDecisions>;
    const firstContainer = document.createElement('div');
    document.body.appendChild(firstContainer);
    const firstRoot: Root = createRoot(firstContainer);
    act(() => firstRoot.render(<DecisionInbox inbox={firstInbox} onClose={() => {}} />));
    const body = document.body.querySelector('[data-testid="inbox-body"]')!;
    act(() => { body.dispatchEvent(new KeyboardEvent('keydown', { key: '1', bubbles: true, cancelable: true })); });
    expect(stored().map(h => h.id)).toEqual(['a']);

    // Cmd+Q at 3 s: the process ends. No page event, no unmount, no timer.
    await act(async () => { await vi.advanceTimersByTimeAsync(3_000); });
    const quitAt = Date.now();
    vi.clearAllTimers(); // (it also rewinds the fake clock)
    vi.setSystemTime(quitAt);
    expect(mocks.firstPageAnswer).not.toHaveBeenCalled();

    // Relaunch: a fresh page reads the inbox.
    vi.resetModules();
    const { useDecisions: relaunched } = await import('./useDecisions');
    mocks.list.mockImplementation(async () => response([decision('a'), decision('b')]));
    mocks.answer.mockImplementation(async (id: string) => ({ decision: decision(id), effect: null, effect_error: null }));
    function Probe() { relaunched(); return null; }
    const container = document.createElement('div');
    document.body.appendChild(container);
    const root = createRoot(container);
    await act(async () => { root.render(<Probe />); });
    await act(async () => { await vi.advanceTimersByTimeAsync(ORPHAN_AFTER_MS); });

    expect(mocks.answer).toHaveBeenCalledTimes(1);
    expect(mocks.answer.mock.calls[0].slice(0, 2)).toEqual(['a', { answer: 'approve' }]);
    expect(localStorage.getItem(HELD_STORE_KEY)).toBeNull();
    expect(mocks.toast).toHaveBeenCalledWith('Sent your earlier answer', expect.stringContaining('Headline a: Allowed.'), undefined, { force: true });

    act(() => root.unmount());
    container.remove();
    act(() => firstRoot.unmount());
    firstContainer.remove();
  });
});
