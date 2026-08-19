/**
 * @vitest-environment jsdom
 *
 * TerminalManager reattach across a REALM RELOAD — the 2026-08-19 report.
 *
 * The existing persistence suite covers a single-commit React remount, which is
 * what the module-level cache was written for and which it handles. This is the
 * other kind of loss, and the one that made an hour of work look gone: the
 * window was minimised for ten minutes, macOS reclaimed the occluded
 * WebContent process, and the page came back with every module variable at its
 * initial value. The PTY was never touched — a `claude` process under
 * `/bin/zsh -l`, 51 minutes old, still owned by the app — but nothing in the UI
 * knew its id any more, so the manager fabricated an empty tab and `Terminal`
 * spawned a second shell over the live one.
 *
 * `__simulateRealmReloadForTests()` reproduces exactly that: it clears the
 * module cache and leaves the durable record and the backend alone.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

interface RenderedTerminal {
  sessionId: string | null;
  cwd?: string;
}
const rendered: RenderedTerminal[] = [];

vi.mock('./Terminal', () => ({
  Terminal: (props: RenderedTerminal) => {
    rendered.push({ sessionId: props.sessionId, cwd: props.cwd });
    return <div data-terminal-session={props.sessionId ?? ''} />;
  },
}));
vi.mock('../../lib/native-drag-drop', () => ({
  registerDropZone: () => () => {},
}));

// The backend's answer to "what is actually running?". `list_pty_sessions` is
// the question that did not exist before this fix.
const liveSessions = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => {
    if (cmd === 'list_pty_sessions') return Promise.resolve(liveSessions());
    return Promise.resolve(undefined);
  },
}));

// The redock listener is not what this suite is about, and a real `listen`
// needs a real Tauri bridge behind the marker we set below.
vi.mock('@tauri-apps/api/event', () => ({
  listen: () => Promise.resolve(() => {}),
}));

import {
  TerminalManager,
  __resetTerminalPersistenceForTests,
  __simulateRealmReloadForTests,
} from './TerminalManager';
import { TERMINAL_STATE_KEY } from './terminalReattach';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const CLAUDE_SESSION = {
  session_id: 'pty-claude-51min',
  cwd: '/Users/x/Documents/dev/GetLadle',
  started_at: '2026-08-19T09:00:00Z',
  alive: true,
  produced: 48120,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // The deferral that keeps a cold start from spawning before the backend has
  // been asked only applies where there IS a backend. Say so explicitly.
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  __resetTerminalPersistenceForTests();
  localStorage.clear();
  rendered.length = 0;
  liveSessions.mockReset();
  liveSessions.mockReturnValue([]);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

/** Let the reconcile effect's awaited dynamic import and invoke settle. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('a realm reload with a live PTY', () => {
  it('reattaches to the running session instead of spawning a shell over it', async () => {
    liveSessions.mockReturnValue([CLAUDE_SESSION]);

    // A cold realm: no module cache, no durable record — the state the owner's
    // window came back in.
    await act(async () => {
      root.render(<TerminalManager />);
    });
    await settle();

    const sessions = rendered.map(r => r.sessionId);
    expect(sessions).toContain('pty-claude-51min');
    // The load-bearing assertion. A `Terminal` mounted with a null sessionId is
    // precisely what calls spawn_pty_session — that is the duplicate shell.
    expect(sessions).not.toContain(null);
    expect(container.querySelector('[data-terminal-session="pty-claude-51min"]')).not.toBeNull();
  });

  it('restores the tab from the durable record when one survives', async () => {
    // A previous session wrote its tabs before the realm went away.
    localStorage.setItem(
      TERMINAL_STATE_KEY,
      JSON.stringify({
        tabs: [
          {
            id: 't1',
            label: 'GetLadle',
            sessionId: 'pty-claude-51min',
            cwd: '/Users/x/Documents/dev/GetLadle',
          },
        ],
        activeTabId: 't1',
        detachedSessionIds: [],
      }),
    );
    __simulateRealmReloadForTests();
    liveSessions.mockReturnValue([CLAUDE_SESSION]);

    await act(async () => {
      root.render(<TerminalManager />);
    });

    // Restored during the RENDER phase — before any effect, because `Terminal`
    // spawns on its own mount and an effect would already be too late.
    expect(rendered[0].sessionId).toBe('pty-claude-51min');
    await settle();
    expect(rendered.map(r => r.sessionId)).not.toContain(null);
  });

  it('writes the durable record as soon as it has tabs', async () => {
    liveSessions.mockReturnValue([CLAUDE_SESSION]);
    await act(async () => {
      root.render(<TerminalManager />);
    });
    await settle();

    const raw = localStorage.getItem(TERMINAL_STATE_KEY);
    expect(raw).toBeTruthy();
    expect(JSON.parse(raw!).tabs[0].sessionId).toBe('pty-claude-51min');
  });

  it('still opens a working terminal when nothing is running', async () => {
    liveSessions.mockReturnValue([]);
    await act(async () => {
      root.render(<TerminalManager />);
    });
    await settle();

    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered[rendered.length - 1].sessionId).toBeNull(); // a fresh shell, correctly
  });

  it('does not clear live session ids when the backend cannot be asked', async () => {
    localStorage.setItem(
      TERMINAL_STATE_KEY,
      JSON.stringify({
        tabs: [{ id: 't1', label: 'GetLadle', sessionId: 'pty-claude-51min' }],
        activeTabId: 't1',
        detachedSessionIds: [],
      }),
    );
    __simulateRealmReloadForTests();
    liveSessions.mockImplementation(() => {
      throw new Error('no bridge');
    });

    await act(async () => {
      root.render(<TerminalManager />);
    });
    await settle();

    expect(rendered.map(r => r.sessionId)).not.toContain(null);
    expect(rendered[rendered.length - 1].sessionId).toBe('pty-claude-51min');
  });
});
