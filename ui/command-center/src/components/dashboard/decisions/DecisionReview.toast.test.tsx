/** @vitest-environment jsdom */
/**
 * The Undo toast against the REAL notification store (DecisionReview.test
 * mocks it). The toast pauses its own lifetime on hover and focus while the
 * hold does not, so it could still show Undo after the answer had been sent,
 * and pressing it then did nothing. Once the hold ends, the same toast item
 * must carry no action at all.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  answer: vi.fn(), showAll: vi.fn(), refresh: vi.fn(), discard: vi.fn(), history: vi.fn(),
}));
vi.mock('../../../lib/api', () => ({ apiFetch: vi.fn(), eventsWsUrl: async () => 'ws://localhost/events' }));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (select: (state: Record<string, unknown>) => unknown) => select({ goalDetail: null }),
}));
vi.mock('../../../lib/useLiveGoals', () => ({ useLiveGoals: () => ({ activeCount: 0, loaded: true }) }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: { display_name: 'Nova' } }) }));
vi.mock('./DiscussionPane', () => ({ DiscussionPane: () => null }));
vi.mock('./EvidenceDigest', () => ({ EvidenceDigest: () => null }));
vi.mock('./client', () => ({ decisionsClient: { dispatchEvidence: async () => null, cancelGoal: vi.fn() } }));

import { useNotifications, type AppNotification } from '../../../lib/notifications';
import { DecisionInbox } from './DecisionInbox';
import { UNDO_MS } from './DecisionReview.undo';
import type { Decision } from './types';
import type { useDecisions } from './useDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class FakeWebSocket {
  onmessage: unknown = null;
  onclose: unknown = null;
  close() {}
}

let root: Root;
let container: HTMLDivElement;
let items: AppNotification[] = [];
let realWebSocket: unknown;

function Probe() {
  items = useNotifications().items;
  return null;
}

function decision(id: string): Decision {
  return { id, kind: 'risk_gate', tier: 2, headline: `Headline ${id}`, detail: '', goal_id: null, project_id: null,
    payload: { action_class: 'cc_shell', description: 'Run a script' }, rank: null, status: 'open', answer: null,
    answer_note: null, answer_choice_id: null, answer_input: null, acted_by: null, created_at: '2026-01-01', resolved_at: null };
}

function render(rows: Decision[]) {
  const inbox = { data: { decisions: rows, total_pending: rows.length, handled_count: 0, goals_in_flight: 0,
    goals_needing_attention: 0, attention_goals: [], oldest_pending_at: null }, loading: false, error: false,
    answer: mocks.answer, refresh: mocks.refresh, showAll: mocks.showAll, discardStaged: mocks.discard, loadHistory: mocks.history,
  } as ReturnType<typeof useDecisions>;
  act(() => root.render(<><DecisionInbox inbox={inbox} onClose={() => {}} /><Probe /></>));
}

function press(key: string) {
  const target = document.body.querySelector('[data-testid="inbox-body"]')!;
  act(() => { target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })); });
}

/** The toast this hold raised (the store is module-wide: match its body). */
function toastFor(title: string) {
  return items.find(i => i.body === title);
}

beforeEach(() => {
  vi.useFakeTimers();
  realWebSocket = (globalThis as { WebSocket?: unknown }).WebSocket;
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeWebSocket;
  mocks.answer.mockImplementation(async (id: string) => ({ ok: true, decision: decision(id), effect: null, effect_error: null }));
  mocks.showAll.mockResolvedValue(undefined);
  mocks.refresh.mockResolvedValue(undefined);
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  (globalThis as { WebSocket?: unknown }).WebSocket = realWebSocket;
  vi.useRealTimers();
});

it('after the hold is sent, the same toast stays but offers no Undo', async () => {
  render([decision('sent')]);
  press('1');
  const before = toastFor('Headline sent')!;
  expect(before.action?.label).toBe('Undo');
  await act(async () => { await vi.advanceTimersByTimeAsync(UNDO_MS); });
  expect(mocks.answer).toHaveBeenCalledWith('sent', { answer: 'approve' });
  const after = items.find(i => i.id === before.id)!;
  expect(after.action).toBeUndefined();
  expect(after.body).toBe('Headline sent: sending now, so it can no longer be undone.');
});

it('after Z, the toast says Undone and offers no Undo', () => {
  render([decision('back')]);
  press('1');
  const before = toastFor('Headline back')!;
  press('z');
  const after = items.find(i => i.id === before.id)!;
  expect(after.action).toBeUndefined();
  expect(after.title).toBe('Undone');
});
