/**
 * @vitest-environment jsdom
 *
 * A fresh profile gets exactly one terminal tab. Smoke 2026-10-12 (F7) showed
 * two tabs, both titled after the home folder, on a profile that had never
 * opened a terminal. This drives the cold start against a fake backend that
 * behaves like the real one: a mounted Terminal with no session spawns one and
 * reports it, and `list_pty_sessions` answers with what was spawned.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { useEffect } from 'react';

const backend = vi.hoisted(() => ({ sessions: [] as Array<{ session_id: string; cwd: string; started_at: string; alive: boolean; produced: number }>, n: 0 }));

vi.mock('./Terminal', () => ({
  Terminal: (props: { sessionId: string | null; onSessionSpawned: (s: string) => void }) => {
    useEffect(() => {
      if (props.sessionId) return undefined;
      const id = `pty-${++backend.n}`;
      backend.sessions.push({ session_id: id, cwd: '/Users/j', started_at: 'x', alive: true, produced: 1 });
      // Real spawn is an IPC round trip: the id arrives a tick later, and
      // only if this instance is still mounted.
      let live = true;
      setTimeout(() => { if (live) props.onSessionSpawned(id); }, 0);
      return () => { live = false; };
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    return <div data-terminal-pane="1" data-terminal-session={props.sessionId ?? ''} />;
  },
}));
vi.mock('../../lib/native-drag-drop', () => ({ registerDropZone: () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({
  transformCallback: () => 0,
  invoke: (cmd: string) => {
    if (cmd === 'list_pty_sessions') return Promise.resolve(backend.sessions.map(s => ({ ...s })));
    return Promise.resolve(undefined);
  },
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));

import { TerminalManager, __resetTerminalPersistenceForTests } from './TerminalManager';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(undefined) };
  __resetTerminalPersistenceForTests();
  localStorage.clear();
  backend.sessions.length = 0;
  backend.n = 0;
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

async function settle() {
  for (let i = 0; i < 4; i++) {
    await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
  }
}
const panes = () => container.querySelectorAll('[data-terminal-pane]').length;

describe('fresh profile terminal tabs', () => {
  it('opens exactly one tab on a plain cold start', async () => {
    await act(async () => { root.render(<TerminalManager />); });
    await settle();
    expect(panes()).toBe(1);
    expect(backend.sessions).toHaveLength(1);
  });

  it('still opens exactly one tab when the pane remounts before the first reconcile settles', async () => {
    await act(async () => { root.render(<TerminalManager key="a" />); });
    await act(async () => { root.render(<TerminalManager key="b" />); });
    await settle();
    expect(panes()).toBe(1);
  });

  it('still opens exactly one tab when the pane remounts after the first tab spawned', async () => {
    await act(async () => { root.render(<TerminalManager key="a" />); });
    await settle();
    await act(async () => { root.render(<TerminalManager key="b" />); });
    await settle();
    expect(panes()).toBe(1);
  });
});

describe('fresh profile terminal tabs, slow spawn', () => {
  it('one tab when the pane remounts while the first shell is still spawning, then a realm reload', async () => {
    await act(async () => { root.render(<TerminalManager key="a" />); });
    await settle();
    await act(async () => { root.render(<TerminalManager key="b" />); });
    await act(async () => { await new Promise(r => setTimeout(r, 5)); });
    await settle();
    expect(panes()).toBe(1);
  });
});

/**
 * Native explanation of the F7 screenshot (2026-10-12). The "fresh profile"
 * smoke ran the bundle `ai.permagent.record-review-20261007`, whose WebKit
 * localStorage is per bundle id, not per daemon profile. Its
 * `permagent.terminal.state.v1` held TWO tabs, both labelled "j" (folder of
 * /Users/j), with ids stamped 2026-10-08 14:30 and 14:40 — created in earlier
 * review sessions, long before the smoke. So the two tabs were the restored
 * client record, not a default plus a restore and not a double create. These
 * cases pin that a returning client gets exactly its saved tabs back.
 */
describe('a returning client restores exactly its saved tabs', () => {
  const saved = {
    tabs: [
      { id: 'tab-1791481242199-1', label: 'j', sessionId: 'pty-old-1', cwd: '/Users/j' },
      { id: 'tab-1791480600668-1', label: 'j', sessionId: 'pty-old-2', cwd: '/Users/j' },
    ],
    activeTabId: 'tab-1791480600668-1',
    detachedSessionIds: [],
  };
  const tabIdsInStorage = () => JSON.parse(localStorage.getItem('permagent.terminal.state.v1')!).tabs.map((t: { id: string }) => t.id);

  it('two saved tabs whose shells died come back as two tabs, no extra default', async () => {
    localStorage.setItem('permagent.terminal.state.v1', JSON.stringify(saved));
    await act(async () => { root.render(<TerminalManager />); });
    await settle();
    expect(panes()).toBe(2);
    expect(tabIdsInStorage()).toEqual(saved.tabs.map(t => t.id));
  });

  it('a live shell no saved tab claims is not adopted as a third tab', async () => {
    localStorage.setItem('permagent.terminal.state.v1', JSON.stringify(saved));
    backend.sessions.push({ session_id: 'pty-stray', cwd: '/Users/j', started_at: 'x', alive: true, produced: 1 });
    await act(async () => { root.render(<TerminalManager />); });
    await settle();
    expect(panes()).toBe(2);
  });

  it('one saved tab restores as one tab', async () => {
    localStorage.setItem('permagent.terminal.state.v1', JSON.stringify({ ...saved, tabs: [saved.tabs[0]], activeTabId: saved.tabs[0].id }));
    await act(async () => { root.render(<TerminalManager />); });
    await settle();
    expect(panes()).toBe(1);
  });
});
