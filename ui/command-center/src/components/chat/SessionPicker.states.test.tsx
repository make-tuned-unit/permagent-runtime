/** @vitest-environment jsdom
 *
 * R2 (dagK UX standard, 2026-09-25): SessionPicker's dropdown had only a
 * populated state — a slow or unreachable daemon drew the same empty-looking
 * list as an agent with no other sessions, and a failed "+ New session"
 * silently closed the dropdown with nothing to explain why nothing happened.
 * This pins the three states it was missing: loading, error+retry and a
 * truthful empty state, plus the create-failure staying open and saying so.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { getSessions, createSession } = vi.hoisted(() => ({
  getSessions: vi.fn(),
  createSession: vi.fn(),
}));

vi.mock('../../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/api')>();
  return { ...actual, api: { ...actual.api, getSessions, createSession } };
});

import { useCommandCenter } from '../../lib/store';
import { SessionPicker } from './SessionPicker';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

function openPicker() {
  const toggle = container.querySelector('[aria-label="Choose chat session"]') as HTMLButtonElement;
  act(() => { toggle.click(); });
}

beforeEach(() => {
  getSessions.mockReset();
  createSession.mockReset();
  useCommandCenter.setState({ sessions: [], sessionsError: false, chatSessionId: 'sess-1', agentName: 'Henry' });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('SessionPicker states', () => {
  it('shows a loading state while the list is in flight, then the populated list', async () => {
    let resolve!: (v: unknown) => void;
    getSessions.mockReturnValueOnce(new Promise(r => { resolve = r; }));
    act(() => { root.render(<SessionPicker />); });
    openPicker();
    expect(container.textContent).toContain('Loading sessions');

    await act(async () => { resolve([{ id: 's2', name: 'Other', message_count: 3, updated_at: new Date().toISOString() }]); await flush(); });
    expect(container.textContent).not.toContain('Loading sessions');
    expect(container.textContent).toContain('Other');
  });

  it('shows a truthful empty state, not a blank dropdown, when there really are no other sessions', async () => {
    getSessions.mockResolvedValueOnce([]);
    act(() => { root.render(<SessionPicker />); });
    openPicker();
    await flush();
    expect(container.textContent).toContain('No other sessions yet');
  });

  it('shows an error with retry when the daemon is unreachable, distinct from empty', async () => {
    getSessions.mockRejectedValueOnce(new Error('network'));
    act(() => { root.render(<SessionPicker />); });
    openPicker();
    await flush();
    expect(container.textContent).toContain("Couldn't reach the daemon");
    expect(container.textContent).not.toContain('No other sessions yet');

    getSessions.mockResolvedValueOnce([{ id: 's3', name: 'Recovered', message_count: 1, updated_at: new Date().toISOString() }]);
    const retry = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Retry') as HTMLButtonElement;
    act(() => { retry.click(); });
    await flush();
    expect(container.textContent).toContain('Recovered');
  });

  it('keeps the dropdown open and reports a failed "New session" instead of closing silently', async () => {
    getSessions.mockResolvedValueOnce([]);
    createSession.mockRejectedValueOnce(new Error('no daemon'));
    act(() => { root.render(<SessionPicker />); });
    openPicker();
    await flush();

    const newSessionBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === '+ New session') as HTMLButtonElement;
    await act(async () => { newSessionBtn.click(); await flush(); });

    // Still open (the dropdown DOM is present) and the failure is named.
    expect(container.textContent).toContain("Couldn't start a new session");
    expect(container.querySelector('[aria-label="Choose chat session"]')?.getAttribute('aria-expanded')).toBe('true');
  });
});
