/**
 * @vitest-environment jsdom
 *
 * `loadBoard` here used to `catch { /* silently fail *\/ }`, so a board that
 * failed to load rendered "No tasks yet." — the same words as a project that
 * genuinely has none. Its sibling `ProjectKanban` fetches the same two
 * endpoints one lens away and has always handled this correctly.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const switchToSession = vi.fn(() => Promise.resolve());
const openChatDock = vi.fn();
vi.mock('../../lib/store', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../lib/store');
  return {
    ...actual,
    useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
      sel({ openCardOnBoard: vi.fn(), openGoalDetail: vi.fn(), growProject: vi.fn(), switchToSession, openChatDock, identityRev: 0 }),
    navigateToTool: vi.fn(),
  };
});
// TasksPanel's provenance marker names the agent via usePersona, which calls
// `api.getIdentity()` — stubbed so the marker's label is deterministic rather
// than whatever the real (absent, in a test) daemon would answer.
vi.mock('../../lib/api', () => ({ api: { getIdentity: () => Promise.resolve({ display_name: 'Nova', first_name: 'Nova', traits: [], tone: '' }) } }));

import { TasksPanel } from './ProjectOverview';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

function render(props: Record<string, unknown>) {
  return act(() => {
    root.render(
      <TasksPanel
        columns={[]}
        cards={[]}
        loading={false}
        error={false}
        onRetry={() => {}}
        onOpenGoal={() => {}}
        onOpenCard={() => {}}
        {...props}
      />,
    );
  });
}

it('a board that failed to load is an error with a retry, not "no tasks yet"', () => {
  const onRetry = vi.fn();
  render({ error: true, onRetry });

  expect(container.textContent).toMatch(/Couldn't load/i);
  expect(container.textContent).not.toMatch(/No tasks yet/i);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Try again/i.test(b.textContent ?? ''))!;
  expect(retry).toBeTruthy();
  act(() => { retry.click(); });
  expect(onRetry).toHaveBeenCalled();
});

it('says it is loading before the first fetch lands', () => {
  render({ loading: true });
  expect(container.textContent).toMatch(/Loading/i);
  expect(container.textContent).not.toMatch(/No tasks yet/i);
});

it('a project that really has no cards still reads as empty', () => {
  render({});
  expect(container.textContent).toMatch(/No tasks yet/i);
  expect(container.textContent).not.toMatch(/Couldn't load/i);
});

const COLS = [{ id: 'c1', projectId: 'p', name: 'To do', position: 0, columnKind: 'todo', stateBinding: 'ready', wipLimit: null }];
function taskCard(id: string, title: string, cardType = 'task') {
  return {
    id, projectId: 'p', cardType, title, description: '', columnId: 'c1', position: 0,
    createdBy: 'user', assignedTo: null, metadataJson: {}, createdAt: '2026-09-01T00:00:00Z',
    updatedAt: '2026-09-01T00:00:00Z', archivedAt: null,
  };
}

// QA walk 2026-09-24: "Week -4: Finalize blog voice…" looked like a row and
// did nothing — only goal cards were wired. A task opens its card detail.
it('clicking a plain task opens its card detail, and Enter does too', () => {
  const onOpenCard = vi.fn();
  const onOpenGoal = vi.fn();
  render({ columns: COLS, cards: [taskCard('t1', 'Week -4: Finalize blog voice and tone'), taskCard('g1', 'Ship pricing', 'goal')], onOpenCard, onOpenGoal });
  const rows = Array.from(container.querySelectorAll<HTMLElement>('[data-testid="overview-task-row"]'));
  const task = rows.find(r => r.textContent?.includes('Finalize blog voice'))!;
  expect(task.getAttribute('role')).toBe('button');
  expect(task.tabIndex).toBe(0);
  act(() => { task.click(); });
  expect(onOpenCard).toHaveBeenCalledWith('t1');
  onOpenCard.mockClear();
  act(() => { task.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); });
  expect(onOpenCard).toHaveBeenCalledWith('t1');
  // Goals still open the goal modal, not the card one.
  const goal = rows.find(r => r.textContent?.includes('Ship pricing'))!;
  act(() => { goal.click(); });
  expect(onOpenGoal).toHaveBeenCalledWith('g1');
  expect(onOpenCard).toHaveBeenCalledTimes(1);
});

// UX_BENCHMARK_RESEARCH.md §7.2: a task the agent created/moved on its own
// used to look identical to one a human did. QA-style regression: the row
// must name who, and — only when the card carries which run — open it
// without also opening the card's own detail underneath.
describe('agent provenance marker (§7.2)', () => {
  async function flushPersona() {
    // usePersona's `api.getIdentity()` resolves a microtask after mount.
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
  }

  it('a user-created task carries no provenance marker', async () => {
    render({ columns: COLS, cards: [taskCard('t1', 'Written by a person')] });
    await flushPersona();
    expect(container.querySelector('[data-testid="task-provenance"]')).toBeNull();
  });

  it('an agent-created task with no run id shows a static "by <agent>" label', async () => {
    render({ columns: COLS, cards: [{ ...taskCard('t1', 'Filed automatically'), createdBy: 'agent' }] });
    await flushPersona();
    const marker = container.querySelector('[data-testid="task-provenance"]')!;
    expect(marker).not.toBeNull();
    expect(marker.textContent).toBe('by Nova');
    expect(marker.tagName).not.toBe('BUTTON');
  });

  it('an agent-created task WITH a run id opens that run, not the card, and only that', async () => {
    const onOpenCard = vi.fn();
    render({
      columns: COLS,
      cards: [{
        ...taskCard('t1', 'Filed automatically'), createdBy: 'agent',
        metadataJson: { created_by_session_id: 'sess-42' },
      }],
      onOpenCard,
    });
    await flushPersona();
    const marker = container.querySelector<HTMLElement>('[data-testid="task-provenance"]')!;
    expect(marker.tagName).toBe('BUTTON');
    act(() => { marker.click(); });
    expect(switchToSession).toHaveBeenCalledWith('sess-42');
    expect(openChatDock).toHaveBeenCalled();
    // The click must not also bubble to the row and open the card detail.
    expect(onOpenCard).not.toHaveBeenCalled();
  });
});
