/**
 * Reattaching to PTYs that are still running.
 *
 * Reported 2026-08-19: after minimising the window for ten minutes, the
 * terminal pane showed a FRESH shell at `~` and an hour of work in a `claude`
 * session appeared to be gone. It was not gone — the PTY was verified alive at
 * the time: a `claude` process under `/bin/zsh -l`, 51 minutes old, parented by
 * the Permagent app, on ttys006. Nothing had killed it (`kill_pty` is only
 * reachable from an explicit close).
 *
 * What was lost was the FRONTEND'S MEMORY of the session id. `persistedTabs` is
 * a module-level variable: it survives a React unmount, which is what it was
 * written for, but not a re-evaluation of the JS realm — and macOS reclaiming an
 * occluded WebContent process is exactly that. With the ids gone, nothing could
 * address the live session; the manager made a new tab and spawned a second
 * shell on top of it.
 *
 * These tests pin the two things that were missing: a durable record, and the
 * ability to adopt a live session the record still lost.
 */
import { describe, it, expect } from 'vitest';
import {
  TERMINAL_STATE_KEY,
  labelForCwd,
  readStoredState,
  reconcileTabs,
  resolveActiveTabId,
  writeStoredState,
  type PtySessionInfo,
} from './terminalReattach';
import type { TerminalTab } from './TerminalManager';

const LIVE_CLAUDE: PtySessionInfo = {
  session_id: 'pty-claude-1',
  cwd: '/Users/x/Documents/dev/GetLadle',
  started_at: '2026-08-19T09:00:00Z',
  alive: true,
  produced: 48120,
};

let counter = 0;
const makeTab = (): TerminalTab => ({
  id: `fresh-${++counter}`,
  label: `Terminal ${counter}`,
  sessionId: null,
});

/** A localStorage stand-in; jsdom's is per-file global, this is per-test. */
function memoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (k: string) => map.get(k) ?? null,
    key: (i: number) => [...map.keys()][i] ?? null,
    removeItem: (k: string) => void map.delete(k),
    setItem: (k: string, v: string) => void map.set(k, v),
  } as Storage;
}

describe('adopting a live session the UI forgot', () => {
  it('reattaches to the running shell instead of burying it under a new one', () => {
    // The realm was re-evaluated: no tabs at all, but the PTY is still there.
    const tabs = reconcileTabs({
      tabs: [],
      live: [LIVE_CLAUDE],
      listed: true,
      adopt: true,
      detachedSessionIds: [],
      makeTab,
    });

    expect(tabs).toHaveLength(1);
    expect(tabs[0].sessionId).toBe('pty-claude-1');
    expect(tabs[0].cwd).toBe('/Users/x/Documents/dev/GetLadle');
    expect(tabs[0].label).toBe('GetLadle');
  });

  /**
   * The old behaviour, stated so the difference is visible: with no way to ask
   * the backend anything, a forgotten realm produced one empty tab, and
   * `Terminal` spawns whenever `sessionId` is null — a second shell over a live
   * one.
   */
  it('the old path fabricated an empty tab, which is what spawned the duplicate shell', () => {
    const oldBehaviour = (persisted: TerminalTab[] | null) => persisted ?? [makeTab()];
    const tabs = oldBehaviour(null);
    expect(tabs).toHaveLength(1);
    expect(tabs[0].sessionId).toBeNull(); // -> spawn_pty_session, over the live PTY
  });

  it('does not adopt a session a detached pane window owns', () => {
    const tabs = reconcileTabs({
      tabs: [],
      live: [LIVE_CLAUDE],
      listed: true,
      adopt: true,
      detachedSessionIds: ['pty-claude-1'],
      makeTab,
    });
    expect(tabs).toHaveLength(1);
    expect(tabs[0].sessionId).toBeNull(); // a fresh tab, not a second view of one PTY
  });

  it('does not adopt on a warm remount — an unclaimed session is somebody else’s', () => {
    const existing: TerminalTab[] = [
      { id: 't1', label: 'proj', sessionId: 'pty-a', cwd: '/Users/x/proj' },
    ];
    const tabs = reconcileTabs({
      tabs: existing,
      live: [{ ...LIVE_CLAUDE, session_id: 'pty-a' }, LIVE_CLAUDE],
      listed: true,
      adopt: false,
      detachedSessionIds: [],
      makeTab,
    });
    expect(tabs.map(t => t.sessionId)).toEqual(['pty-a']);
  });

  it('never adopts a session that has exited', () => {
    const tabs = reconcileTabs({
      tabs: [],
      live: [{ ...LIVE_CLAUDE, alive: false }],
      listed: true,
      adopt: true,
      detachedSessionIds: [],
      makeTab,
    });
    expect(tabs[0].sessionId).toBeNull();
  });
});

describe('a tab whose PTY has gone', () => {
  it('gets a working shell rather than a pane wired to nothing', () => {
    const tabs = reconcileTabs({
      tabs: [{ id: 't1', label: 'proj', sessionId: 'pty-dead' }],
      live: [],
      listed: true,
      adopt: false,
      detachedSessionIds: [],
      makeTab,
    });
    expect(tabs).toHaveLength(1);
    expect(tabs[0].id).toBe('t1'); // same tab, same label
    expect(tabs[0].sessionId).toBeNull(); // will spawn
  });

  /**
   * The dangerous case. If the listing itself failed, "nothing is running" is
   * not a fact — and acting on it would clear every id and respawn every shell,
   * which is the damage this whole change exists to prevent.
   */
  it('a FAILED listing changes nothing at all', () => {
    const live: TerminalTab[] = [
      { id: 't1', label: 'proj', sessionId: 'pty-a' },
      { id: 't2', label: 'notes', sessionId: 'pty-b' },
    ];
    const tabs = reconcileTabs({
      tabs: live,
      live: [],
      listed: false,
      adopt: true,
      detachedSessionIds: [],
      makeTab,
    });
    expect(tabs).toBe(live);
    expect(tabs.map(t => t.sessionId)).toEqual(['pty-a', 'pty-b']);
  });

  it('never produces an empty pane', () => {
    const tabs = reconcileTabs({
      tabs: [],
      live: [],
      listed: true,
      adopt: true,
      detachedSessionIds: [],
      makeTab,
    });
    expect(tabs).toHaveLength(1);
  });
});

describe('the durable record', () => {
  it('round-trips tabs, the active id and the detached set', () => {
    const storage = memoryStorage();
    const state = {
      tabs: [{ id: 't1', label: 'GetLadle', sessionId: 'pty-claude-1', cwd: '/x/GetLadle' }],
      activeTabId: 't1',
      detachedSessionIds: ['pty-other'],
    };
    writeStoredState(storage, state);
    expect(readStoredState(storage)).toEqual(state);
  });

  it('survives a module-level reset — that is the entire point', () => {
    const storage = memoryStorage();
    writeStoredState(storage, {
      tabs: [{ id: 't1', label: 'GetLadle', sessionId: 'pty-claude-1' }],
      activeTabId: 't1',
      detachedSessionIds: [],
    });
    // A realm re-evaluation zeroes every module variable. Storage does not care.
    const afterReload = readStoredState(storage);
    expect(afterReload?.tabs[0].sessionId).toBe('pty-claude-1');
  });

  it('degrades to a cold start on corrupt or absent data, never throwing', () => {
    const storage = memoryStorage();
    expect(readStoredState(storage)).toBeNull();
    storage.setItem(TERMINAL_STATE_KEY, 'not json');
    expect(readStoredState(storage)).toBeNull();
    storage.setItem(TERMINAL_STATE_KEY, JSON.stringify({ tabs: 'nope' }));
    expect(readStoredState(storage)).toBeNull();
    storage.setItem(TERMINAL_STATE_KEY, JSON.stringify({ tabs: [] }));
    expect(readStoredState(storage)).toBeNull();
    expect(readStoredState(null)).toBeNull();
  });

  it('a storage that throws is not worth breaking the pane over', () => {
    const hostile = {
      getItem: () => {
        throw new Error('denied');
      },
      setItem: () => {
        throw new Error('quota');
      },
    } as unknown as Storage;
    expect(readStoredState(hostile)).toBeNull();
    expect(() =>
      writeStoredState(hostile, { tabs: [], activeTabId: null, detachedSessionIds: [] }),
    ).not.toThrow();
  });
});

describe('small things that have to be right', () => {
  it('labels an adopted tab by its working directory', () => {
    expect(labelForCwd('/Users/x/Documents/dev/GetLadle')).toBe('GetLadle');
    expect(labelForCwd('/Users/x/Documents/dev/GetLadle/')).toBe('GetLadle');
    expect(labelForCwd('/')).toBe('Terminal');
    expect(labelForCwd('')).toBe('Terminal');
  });

  it('keeps the active tab when it survived, and falls back when it did not', () => {
    const tabs: TerminalTab[] = [
      { id: 'a', label: 'a', sessionId: null },
      { id: 'b', label: 'b', sessionId: null },
    ];
    expect(resolveActiveTabId(tabs, 'b')).toBe('b');
    expect(resolveActiveTabId(tabs, 'gone')).toBe('a');
    expect(resolveActiveTabId(tabs, null)).toBe('a');
    expect(resolveActiveTabId([], 'x')).toBe('');
  });
});
