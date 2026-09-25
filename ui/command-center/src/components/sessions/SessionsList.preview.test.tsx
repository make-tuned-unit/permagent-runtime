/**
 * @vitest-environment jsdom
 *
 * SessionsList — the ≥1600px list/preview split (coordinator ruling,
 * 2026-09-25): a list column + a preview pane for the selected/hovered row,
 * fetched lazily via GET /api/sessions/{id} and cached; keyboard ↑/↓ moves
 * selection (never opens), Enter opens. Below the breakpoint the split
 * doesn't render at all — SessionsList.filters.test.tsx covers that layout.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { getSession } = vi.hoisted(() => ({
  getSession: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: {
    getSessions: vi.fn(() => Promise.resolve([])),
    createSession: vi.fn(),
    getSessionInsights: vi.fn(() => Promise.resolve({ total_sessions: 2, total_tokens: 100 })),
    searchSessions: vi.fn(() => Promise.resolve([])),
    forkSession: vi.fn(() => Promise.resolve({ sessionId: 'forked-1' })),
    getSession,
  },
  extractText: (m: { content: Array<{ type: string; text?: string }> }) =>
    m.content.filter(c => c.type === 'text').map(c => c.text).join('\n\n'),
  apiFetch: vi.fn(),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

import { SessionsList } from './SessionsList';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function setWidth(px: number) {
  Object.defineProperty(window, 'innerWidth', { value: px, writable: true, configurable: true });
}

let container: HTMLDivElement;
let root: Root;

const ROWS = [
  { id: 'row-1', name: 'Plan the Atlas launch', session_type: 'user', created_at: 'c', updated_at: 'u', message_count: 8 },
  { id: 'row-2', name: 'Fix the login bug', session_type: 'user', created_at: 'c', updated_at: 'u', message_count: 3 },
  { id: 'row-3', name: 'Ship the release notes', session_type: 'user', created_at: 'c', updated_at: 'u', message_count: 1 },
];

beforeEach(() => {
  setWidth(1800); // ≥1600: the split renders
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getSession.mockReset();
  getSession.mockResolvedValue({
    id: 'row-1', name: 'Plan the Atlas launch', message_count: 8, total_tokens: 4200,
    conversation: [
      { role: 'user', content: [{ type: 'text', text: 'Let’s plan the Atlas launch.' }], metadata: { userVisible: true, agentVisible: true } },
      { role: 'assistant', content: [{ type: 'text', text: 'Here is a draft plan.' }], metadata: { userVisible: true, agentVisible: true } },
    ],
  });
  useCommandCenter.setState({
    sessions: ROWS,
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
  setWidth(1024);
});

function render() {
  act(() => { root.render(<SessionsList onClose={() => {}} />); });
}

describe('below 1600px: no split', () => {
  it('renders a single list and no preview pane', async () => {
    setWidth(1200);
    render();
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-testid="session-preview"]')).toBeNull();
    expect(container.querySelectorAll('[data-testid="session-row"]').length).toBe(3);
  });
});

describe('≥1600px: list/preview split', () => {
  it('shows the empty preview state until a row is selected or hovered', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-testid="session-preview"]')).toBeNull();
    expect(container.textContent).toContain('Select a session to preview');
  });

  it('clicking a row opens it AND populates the preview (loading -> populated)', async () => {
    render();
    await act(async () => { await Promise.resolve(); });
    const row = container.querySelectorAll('[data-testid="session-row"]')[0] as HTMLElement;
    act(() => { row.dispatchEvent(new MouseEvent('click', { bubbles: true })); row.focus(); });
    // loading: a skeleton, not a spinner in the void
    expect(container.querySelector('[data-testid="session-preview-skeleton"]')).toBeTruthy();
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(getSession).toHaveBeenCalledWith('row-1');
    expect(container.querySelector('[data-testid="session-preview-title"]')?.textContent).toBe('Plan the Atlas launch');
    expect(container.textContent).toContain('Let’s plan the Atlas launch.'); // first message
    expect(container.textContent).toContain('Here is a draft plan.'); // last reply
    expect(container.textContent).toContain('4.2K tokens');
  });

  it('an error fetching the detail shows a retry, not a silent empty pane', async () => {
    getSession.mockReset();
    getSession.mockRejectedValueOnce(new Error('down')).mockResolvedValueOnce({ id: 'row-1', message_count: 1, conversation: [] });
    render();
    await act(async () => { await Promise.resolve(); });
    const row = container.querySelectorAll('[data-testid="session-row"]')[0] as HTMLElement;
    // jsdom does not move focus on a synthetic dispatchEvent click the way a
    // real browser does on a genuine click — call .focus() too, same as a
    // real click on a focusable role="button" row would.
    act(() => { row.dispatchEvent(new MouseEvent('click', { bubbles: true })); row.focus(); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(container.textContent).toContain("Couldn't load this session");
    const retry = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Try again');
    expect(retry).toBeTruthy();
    act(() => retry!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(getSession).toHaveBeenCalledTimes(2);
    expect(container.textContent).not.toContain("Couldn't load this session");
  });

  it('hovering a different row peeks at it after a delay, and reverts on mouseleave', async () => {
    vi.useFakeTimers();
    render();
    await act(async () => { await Promise.resolve(); });
    const rows = container.querySelectorAll('[data-testid="session-row"]');
    act(() => { (rows[0] as HTMLElement).dispatchEvent(new MouseEvent('click', { bubbles: true })); (rows[0] as HTMLElement).focus(); });
    await act(async () => { await Promise.resolve(); });

    // React derives onMouseEnter/onMouseLeave from native 'mouseover'/'mouseout'
    // (mouseenter/mouseleave don't bubble, so React doesn't listen for them
    // directly) — dispatch what the browser actually sends.
    act(() => { (rows[1] as HTMLElement).dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
    // Before the delay elapses, the preview still shows the selected row.
    expect(getSession).not.toHaveBeenCalledWith('row-2');
    act(() => { vi.advanceTimersByTime(320); });
    await act(async () => { await Promise.resolve(); });
    expect(getSession).toHaveBeenCalledWith('row-2');

    act(() => { (rows[1] as HTMLElement).dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
    await act(async () => { await Promise.resolve(); });
    // Reverted to the selected row (row-1, already cached from the click above).
    expect(container.querySelector('[data-testid="session-preview-title"]')?.textContent).toBe('Plan the Atlas launch');
    vi.useRealTimers();
  });

  it('↑/↓ move the preview selection without opening a session; Enter opens it', async () => {
    const switchToSession = useCommandCenter.getState().switchToSession;
    render();
    await act(async () => { await Promise.resolve(); });
    const rows = Array.from(container.querySelectorAll('[data-testid="session-row"]')) as HTMLElement[];
    act(() => rows[0].focus());
    act(() => rows[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(rows[1]);
    expect(switchToSession).not.toHaveBeenCalled(); // arrow alone never opens
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-testid="session-preview"]')).toBeTruthy(); // selection followed focus

    act(() => rows[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })));
    expect(switchToSession).toHaveBeenCalledWith('row-2');
  });
});
