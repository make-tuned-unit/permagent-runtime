/**
 * @vitest-environment jsdom
 *
 * SessionsList render honesty (C6) + rename-Escape containment (C5).
 *
 * C6: when loadSessions failed, the overlay must render the inline error +
 * Retry — never the lying "No sessions yet" empty state — and Retry must
 * re-drive loadSessions.
 *
 * C5: Escape inside the rename input must cancel ONLY the rename; without
 * stopPropagation the overlay's window-level Escape handler also fired and
 * closed the whole surface on the same keypress.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { getSessions: vi.fn(() => Promise.resolve([])), createSession: vi.fn() },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
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

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(ui: React.ReactElement) {
  act(() => root.render(ui));
}

describe('C6 — failed load renders the inline error, not the empty-state lie', () => {
  it('shows the error + Retry and hides "No sessions yet" when sessionsError is set', () => {
    const loadSessions = vi.fn();
    useCommandCenter.setState({ sessions: [], sessionsError: true, loadSessions });
    render(<SessionsList onClose={() => {}} />);

    expect(container.textContent).toContain("Couldn't load sessions");
    expect(container.textContent).not.toContain('No sessions yet');

    const retry = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Retry');
    expect(retry).toBeTruthy();
    const callsBefore = loadSessions.mock.calls.length; // mount effect also loads
    act(() => retry!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(loadSessions.mock.calls.length).toBe(callsBefore + 1);
  });

  it('still shows the honest empty state when the daemon really has no sessions', () => {
    useCommandCenter.setState({ sessions: [], sessionsError: false, loadSessions: vi.fn() });
    render(<SessionsList onClose={() => {}} />);
    expect(container.textContent).toContain('No sessions yet');
    expect(container.textContent).not.toContain("Couldn't load sessions");
  });
});

describe('C5 — Escape in the rename input cancels the rename only', () => {
  const session = { id: 'sess-1234567890', name: 'My chat', created_at: 'c', updated_at: undefined, message_count: 2 };

  function startRenaming(): HTMLInputElement {
    // EditableName's own span (its wrapper has the same textContent — clicking
    // the wrapper would select the row instead of starting the rename).
    const name = container.querySelector('span.truncate.cursor-pointer');
    expect(name).toBeTruthy();
    act(() => name!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const input = container.querySelector('input');
    expect(input).toBeTruthy();
    return input as HTMLInputElement;
  }

  it('one Escape closes the input but does NOT close the overlay', () => {
    const onClose = vi.fn();
    useCommandCenter.setState({ sessions: [session], sessionsError: false, loadSessions: vi.fn() });
    render(<SessionsList onClose={onClose} />);

    const input = startRenaming();
    act(() => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    });

    expect(container.querySelector('input')).toBeNull(); // rename cancelled
    expect(onClose).not.toHaveBeenCalled(); // overlay survived the keypress
  });

  it('Escape outside the rename still closes the overlay (the window handler is live)', () => {
    const onClose = vi.fn();
    useCommandCenter.setState({ sessions: [session], sessionsError: false, loadSessions: vi.fn() });
    render(<SessionsList onClose={onClose} />);

    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
