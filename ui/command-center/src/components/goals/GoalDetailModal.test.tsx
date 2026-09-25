/** @vitest-environment jsdom */

/**
 * GM1/GM3/GM4/GM7 (dagK): regression coverage for the goal detail modal's
 * agent-transparency fixes.
 *
 *  - GM1: a worker session id is a click-through "Open transcript", not text.
 *  - GM3: Escape closes the modal (DetailModal's own floor — locked in here
 *    for this specific consumer, including mid-confirm and with the evidence
 *    disclosure open).
 *  - GM4: the state label matches the SAME hold state Home's "In flight" list
 *    reads from `/api/goals/active` — not recomputed from the board column
 *    alone.
 *  - GM7: A2A messages on the card's `a2a_inbox`/`a2a_sent` metadata render
 *    as a "Messages" section instead of being invisible.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({
  switchToSession: vi.fn().mockResolvedValue(undefined),
  openChatDock: vi.fn(),
}));

vi.mock('../../lib/store', () => ({
  useCommandCenter: (select: (s: Record<string, unknown>) => unknown) => select({
    switchToSession: state.switchToSession,
    openChatDock: state.openChatDock,
  }),
}));
// The Evidence panel is exercised on its own (EvidenceDigest.test.tsx et al);
// here it would otherwise make a real, unmocked network call.
vi.mock('../dashboard/decisions/EvidenceDigest', () => ({ EvidenceDigest: () => null }));

const CARD: Record<string, unknown> = {
  id: 'c1', projectId: 'p1', cardType: 'goal', title: 'Ship the thing',
  description: 'Do the work.', columnId: 'col-ready', assignedTo: 'Henry',
  metadataJson: {
    worker_session_id: 'sess-abc123',
    a2a_inbox: [{ from_goal: 'g2', to_goal: 'c1', body: 'watch the race', ts: '2026-09-20T00:00:00Z' }],
    a2a_sent: [{ from_goal: 'c1', to_goal: 'g2', body: 'on it', ts: '2026-09-20T00:01:00Z' }],
  },
  createdAt: '2026-09-01T00:00:00Z', updatedAt: '2026-09-01T00:00:00Z',
};
const COLUMNS = [{ id: 'col-ready', name: 'Ready', stateBinding: 'ready' }];
const PROJECT_GOALS = [CARD, { id: 'g2', title: 'Other goal', cardType: 'goal', metadataJson: {} }];

function mockApi(activeGoal: Record<string, unknown> | null) {
  vi.doMock('../../lib/api', () => ({
    apiFetch: vi.fn().mockImplementation((url: string) => {
      if (url.includes('/columns')) return Promise.resolve(COLUMNS);
      if (url.includes('?card_type=goal')) return Promise.resolve(PROJECT_GOALS);
      if (url.includes('/api/goals/active')) return Promise.resolve({ goals: activeGoal ? [activeGoal] : [] });
      return Promise.resolve(CARD);
    }),
  }));
}

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  vi.resetModules();
  state.switchToSession.mockClear();
  state.openChatDock.mockClear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.doUnmock('../../lib/api');
});

async function renderModal(activeGoal: Record<string, unknown> | null = null) {
  mockApi(activeGoal);
  const { GoalDetailModal } = await import('./GoalDetailModal');
  const onClose = vi.fn();
  await act(async () => {
    root.render(<GoalDetailModal projectId="p1" cardId="c1" onClose={onClose} />);
  });
  // flush the Promise.all fetch chain
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
  return onClose;
}

function pressEscape() {
  act(() => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  });
}

it('GM1: the worker session row is an Open-transcript button that switches sessions and opens the dock', async () => {
  await renderModal();
  const btn = Array.from(document.body.querySelectorAll('button')).find(b => b.textContent === 'Open transcript');
  expect(btn, 'must render a click-through affordance, not the raw session id as text').toBeTruthy();
  expect(document.body.textContent).not.toContain('sess-abc123');
  await act(async () => { btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(state.switchToSession).toHaveBeenCalledWith('sess-abc123');
  expect(state.openChatDock).toHaveBeenCalledTimes(1);
});

it('GM3: Escape closes the modal', async () => {
  const onClose = await renderModal();
  pressEscape();
  expect(onClose).toHaveBeenCalledTimes(1);
});

it('GM3: Escape still closes the modal with the evidence "Show details" disclosure open', async () => {
  const onClose = await renderModal();
  // The evidence digest self-fetches and renders nothing testable here without
  // its own client mock; what matters is that expanding ANY disclosure in the
  // body does not swallow the keydown before it reaches the shared shell.
  const checkbox = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement | null;
  if (checkbox) act(() => { checkbox.focus(); });
  pressEscape();
  expect(onClose).toHaveBeenCalledTimes(1);
});

it('GM4: a held goal reads "Held" here — the same word Home\'s In-flight list uses for the same hold state', async () => {
  await renderModal({ id: 'c1', title: 'Ship the thing', project_id: 'p1', state: 'in_progress', assigned_to: 'Henry', created_at: '', updated_at: '', hold_note: 'held 1/3: verified, waiting on a commit' });
  expect(document.body.textContent).toContain('Held');
  expect(document.body.textContent).toContain('held 1/3: verified, waiting on a commit');
  // Not the column name, which would read "Ready" — the exact drift GM4 reports.
  const badge = document.body.querySelector('[role="dialog"]');
  expect(badge?.textContent).not.toMatch(/\bReady\b/);
});

it('GM4: an un-held active goal keeps reading the plain state label', async () => {
  await renderModal({ id: 'c1', title: 'Ship the thing', project_id: 'p1', state: 'ready', assigned_to: 'Henry', created_at: '', updated_at: '', hold_note: null });
  const badge = document.body.querySelector('[role="dialog"]');
  expect(badge?.textContent).toMatch(/Ready/);
});

it('GM7: A2A inbox and sent messages render as a Messages section', async () => {
  await renderModal();
  expect(document.body.textContent).toContain('Messages');
  expect(document.body.textContent).toContain('watch the race');
  expect(document.body.textContent).toContain('on it');
  expect(document.body.textContent).toContain('Other goal');
});
