/**
 * @vitest-environment jsdom
 *
 * SessionsList — the dagK/ux-history History rework (S2-S4).
 *
 * S2: History defaults to Conversations, with Conversations/Agent runs/System
 * chips and counts, filtering the SAME `sessions` list one shared classifier
 * (lib/sessionLabel.ts) sorts — never a second, divergent notion of "kind".
 * S3: typing filters instantly (client-side, over the already-loaded list)
 * and folds in `/api/sessions/search` matches (full transcript, debounced)
 * without ever removing a client-side match while the network call is out.
 * S4: a project badge and a goal chip appear on rows that carry them, and
 * "Fork" copies a session into a new one without ever navigating on failure.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { getSessionInsights, searchSessions, forkSession } = vi.hoisted(() => ({
  getSessionInsights: vi.fn(() => Promise.resolve({ total_sessions: 2, total_tokens: 42300 })),
  searchSessions: vi.fn(() => Promise.resolve([] as Array<{ id: string }>)),
  forkSession: vi.fn(() => Promise.resolve({ sessionId: 'forked-1' })),
}));

vi.mock('../../lib/api', () => ({
  api: {
    getSessions: vi.fn(() => Promise.resolve([])),
    createSession: vi.fn(),
    getSessionInsights,
    searchSessions,
    forkSession,
  },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { SessionsList } from './SessionsList';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

const CHAT = {
  id: 'chat-1', name: 'CLI Session', created_at: 'c', updated_at: 'u', message_count: 3,
  session_type: 'user', first_user_text: 'How do I rotate the staging keys?',
  project_id: 'project:atlas', project_name: 'Atlas',
};
const AGENT = {
  id: 'agent-1', name: 'worker', created_at: 'c', updated_at: 'u', message_count: 12,
  session_type: 'sub_agent', goal_id: 'card-9', goal_title: 'Ship the release notes',
  goal_project_id: 'project:atlas',
};
const SCHEDULED = {
  id: 'sched-1', name: 'Desktop — no battery', created_at: 'c', updated_at: 'u', message_count: 1,
  session_type: 'scheduled', recipe_title: 'Workspace Snapshot',
};

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getSessionInsights.mockClear();
  searchSessions.mockClear().mockResolvedValue([]);
  forkSession.mockClear().mockResolvedValue({ sessionId: 'forked-1' });
  useCommandCenter.setState({
    sessions: [CHAT, AGENT, SCHEDULED],
    sessionsError: false,
    loadSessions: vi.fn(),
    switchToSession: vi.fn(() => Promise.resolve()),
    deleteSession: vi.fn(() => Promise.resolve()),
    renameSession: vi.fn(() => Promise.resolve()),
    openChatDock: vi.fn(),
    setActivePanel: vi.fn(),
    openGoalDetail: vi.fn(),
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render() {
  act(() => { root.render(<SessionsList onClose={() => {}} />); });
}

function filterChip(kind: string): HTMLElement {
  const el = container.querySelector(`[data-testid="sessions-filter-${kind}"]`);
  expect(el).toBeTruthy();
  return el as HTMLElement;
}

describe('S2 — Conversations / Agent runs / System', () => {
  it('defaults to Conversations and lists only the user session', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const rows = container.querySelectorAll('[data-testid="session-row"]');
    expect(rows.length).toBe(1);
    expect(container.querySelector('[data-testid="session-row-title"]')?.textContent)
      .toBe('How do I rotate the staging keys?'); // "CLI Session" is a placeholder, repaired from the first message

    expect(filterChip('conversation').textContent).toContain('1');
    expect(filterChip('agent_run').textContent).toContain('1');
    expect(filterChip('system').textContent).toContain('1');
  });

  it('switches to Agent runs and shows the goal-linked worker', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    act(() => filterChip('agent_run').dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const rows = container.querySelectorAll('[data-testid="session-row"]');
    expect(rows.length).toBe(1);
    expect(container.querySelector('[data-testid="session-row-goal"]')?.textContent)
      .toContain('Ship the release notes');
  });

  it('switches to System and shows the scheduled run named after its job', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    act(() => filterChip('system').dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(container.querySelector('[data-testid="session-row-title"]')?.textContent)
      .toBe('Workspace Snapshot');
  });

  it('an empty filter shows the filter-specific empty state, not the global one', async () => {
    useCommandCenter.setState({ sessions: [SCHEDULED] });
    render();
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-testid="sessions-filter-empty"]')?.textContent)
      .toBe('No conversations yet.');
  });
});

describe('S3 — search', () => {
  it('filters client-side instantly over the label and first message', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    act(() => filterChip('agent_run').dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const search = container.querySelector('[data-testid="sessions-search"]') as HTMLInputElement;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
      setter.call(search, 'release notes');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(container.querySelectorAll('[data-testid="session-row"]').length).toBe(1);
  });

  it('folds in a debounced full-transcript match the client-side filter misses', async () => {
    vi.useFakeTimers();
    searchSessions.mockResolvedValue([{ id: 'sched-1' }]);
    render();
    await act(async () => { await Promise.resolve(); });
    act(() => filterChip('system').dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const search = container.querySelector('[data-testid="sessions-search"]') as HTMLInputElement;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
      setter.call(search, 'a word only inside a later message');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    // Before the debounced search resolves: the client-side filter alone finds nothing.
    expect(container.querySelectorAll('[data-testid="session-row"]').length).toBe(0);
    await act(async () => { vi.advanceTimersByTime(300); await Promise.resolve(); await Promise.resolve(); });
    expect(searchSessions).toHaveBeenCalledWith('a word only inside a later message');
    expect(container.querySelectorAll('[data-testid="session-row"]').length).toBe(1);
    vi.useRealTimers();
  });
});

describe('S3 — insights header', () => {
  it('shows the total session count and a compact token total', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const insights = container.querySelector('[data-testid="sessions-insights"]');
    expect(insights?.textContent).toContain('2');
    expect(insights?.textContent).toMatch(/42\.3K/);
  });
});

describe('S4 — project badge, goal chip, fork', () => {
  it('shows the project chip on a conversation that carries one', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-testid="session-row-project"]')?.textContent).toBe('Atlas');
  });

  it('Fork copies the session and opens the new one, never the original on failure', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const fork = container.querySelector('[data-testid="session-row-fork"]') as HTMLButtonElement;
    await act(async () => {
      fork.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      await Promise.resolve(); await Promise.resolve();
    });
    expect(forkSession).toHaveBeenCalledWith('chat-1');
    expect(useCommandCenter.getState().switchToSession).toHaveBeenCalledWith('forked-1');
  });

  it('a failed fork never switches sessions', async () => {
    forkSession.mockRejectedValueOnce(new Error('daemon down'));
    render();
    await act(async () => { await Promise.resolve(); });
    const fork = container.querySelector('[data-testid="session-row-fork"]') as HTMLButtonElement;
    const switchToSession = useCommandCenter.getState().switchToSession;
    await act(async () => {
      fork.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      await Promise.resolve(); await Promise.resolve();
    });
    expect(switchToSession).not.toHaveBeenCalled();
  });
});

describe('R4 — keyboard: a session row is Enter-activatable', () => {
  it('Enter on a focused row opens it, same as a click', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const row = container.querySelector('[data-testid="session-row"]') as HTMLElement;
    expect(row.getAttribute('role')).toBe('button');
    expect(row.getAttribute('tabindex')).toBe('0');
    act(() => row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })));
    expect(useCommandCenter.getState().switchToSession).toHaveBeenCalledWith('chat-1');
  });
});

describe('rename — 2026-09-25 coordinator finding: a single click must never start it', () => {
  it('a single click on the title selects/opens it, exactly like clicking anywhere else on the row', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const title = container.querySelector('[data-testid="session-row-title-text"]') as HTMLElement;
    act(() => title.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    // It opened the row (the click bubbled through, unobstructed) — it did
    // NOT turn the title into a rename input.
    expect(useCommandCenter.getState().switchToSession).toHaveBeenCalledWith('chat-1');
    expect(container.querySelector('input:not([type="search"])')).toBeNull();
  });

  it('a double-click on the title starts a rename', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const title = container.querySelector('[data-testid="session-row-title-text"]') as HTMLElement;
    act(() => title.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    const input = container.querySelector('input:not([type="search"])') as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.value).toBe('How do I rotate the staging keys?');
  });

  it('F2 on a focused row starts a rename without opening it', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const row = container.querySelector('[data-testid="session-row"]') as HTMLElement;
    act(() => row.dispatchEvent(new KeyboardEvent('keydown', { key: 'F2', bubbles: true, cancelable: true })));
    expect(container.querySelector('input:not([type="search"])')).toBeTruthy();
    expect(useCommandCenter.getState().switchToSession).not.toHaveBeenCalled();
  });
});
