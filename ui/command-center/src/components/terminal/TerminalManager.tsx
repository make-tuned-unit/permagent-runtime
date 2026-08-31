import { useState, useCallback, useRef, useEffect, useImperativeHandle, forwardRef, type CSSProperties } from 'react';
import { FiPlus, FiX, FiTerminal, FiFilePlus, FiExternalLink } from 'react-icons/fi';
import { Terminal } from './Terminal';
import { useTheme } from '../../styles/useTheme';
import { registerDropZone } from '../../lib/native-drag-drop';
import { resolvePtyInjection } from './terminalDrop';
import { font, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { CycleTabsButton } from '../build/CycleTabsButton';
import { nextPaneTabId, usePaneTabCycling } from '../build/paneTabCycling';
import {
  TERMINAL_STATE_KEY,
  readStoredState,
  writeStoredState,
  reconcileTabs,
  resolveActiveTabId,
  type PtySessionInfo,
} from './terminalReattach';
import { createClaimSet } from '../../lib/claimSet';

export interface TerminalManagerHandle {
  createProjectTab: (
    cwd: string,
    label: string,
    initialCommand?: string,
    supervisedSessionId?: string,
    extras?: {
      followUpInput?: string;
      growthAction?: { projectId: string; actionId: string };
      /** Second layer of the double-tab defence (see pendingLaunch.ts) — when
       *  present and already claimed by this manager, the tab is not created. */
      launchId?: string;
    },
  ) => void;
  getActiveTab: () => TerminalTab;
  /** Every tab this manager owns — used by a detached pane window to tear
   *  down all of its PTYs when the window is genuinely closed (not redocked). */
  getAllTabs: () => TerminalTab[];
  /** Kill the PTY behind a tab. Detached windows call this on real close so a
   *  closed window does not leave an orphaned shell running. */
  killTab: (sessionId: string) => Promise<void>;
}

export interface TerminalTab {
  id: string;
  label: string;
  sessionId: string | null;
  cwd?: string;
  initialCommand?: string;
  /** S2 (#428): supervised loop session id — makes the spawned PTY tee its
   *  output to the daemon's gate parser. */
  supervisedSessionId?: string;
  followUpInput?: string;
  growthAction?: { projectId: string; actionId: string };
}

let tabCounter = 0;

function createTab(cwd?: string): TerminalTab {
  tabCounter++;
  return {
    id: `tab-${Date.now()}-${tabCounter}`,
    label: `Terminal ${tabCounter}`,
    sessionId: null,
    cwd,
  };
}

// ── Module-level state persists across workspace switches (mount/unmount) ──
//
// WHY THIS IS MODULE-LEVEL (do not refactor into React state):
// When the user switches workspaces (e.g., Build → Home → Build), React
// unmounts TerminalManager entirely. If tab state lived only in React
// state, every workspace switch would kill all terminal tabs and spawn
// new PTY sessions — losing scrollback, working directory, running
// processes, and shell history. Module-level vars survive the unmount
// and restore the exact tab state on remount, reconnecting to the
// still-alive PTY sessions on the Tauri backend.
//
// Same pattern used by Browser.tsx for the same reason.
let persistedTabs: TerminalTab[] | null = null;
let persistedActiveTabId: string | null = null;

// Sessions handed to a detached pane window. That window keeps its own copy in
// its own realm, so the docked manager must never adopt one back — two panes
// writing into one PTY is a worse outcome than an orphan.
let detachedSessionIds: string[] = [];

// Second layer of the double-tab defence (see pendingLaunch.ts): BuildView
// claims a launch id first, but if it somehow reached here twice anyway (a
// second manager instance, or a caller bypassing BuildView), this manager
// refuses to open a second tab for an id it has already seen. A DEDICATED
// set — never shared with BuildView's — because both layers must be able to
// claim independently.
const projectTabLaunchClaims = createClaimSet(50);

// ── …and a DURABLE copy, because module-level state is not durable ──────────
//
// The module variables above survive a React unmount. They do not survive
// re-evaluation of the JS realm, and that is what actually happened on
// 2026-08-19: the window was minimised for ten minutes, macOS reclaimed the
// occluded WebContent process, the page came back freshly evaluated, and this
// file's `persistedTabs` was `null` again. The manager then made one empty tab,
// `Terminal` saw no `sessionId` and spawned a new shell at `$HOME`, and a live
// `claude` session — still running, still owned by the app — became reachable
// by nothing. The owner reasonably believed an hour of work was gone.
//
// So the record is mirrored into localStorage, which does survive, and the
// backend is asked what is actually running (`list_pty_sessions`) so anything
// the record still lost can be adopted rather than buried.
function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * Test-only: a genuinely cold start — module cache AND durable record cleared.
 * The durable half matters: leaving it behind would leak one case's tabs into
 * the next, which is precisely the class of confusion this record introduces.
 */
export function __resetTerminalPersistenceForTests() {
  persistedTabs = null;
  persistedActiveTabId = null;
  detachedSessionIds = [];
  projectTabLaunchClaims.reset();
  try {
    storage()?.removeItem(TERMINAL_STATE_KEY);
  } catch {
    /* no storage in this environment */
  }
}

/**
 * Test-only: drop the module-level cache the way a realm re-evaluation does,
 * WITHOUT touching the durable record. This is the shape of the reported bug —
 * the process kept its PTYs, the page forgot their ids.
 */
export function __simulateRealmReloadForTests() {
  persistedTabs = null;
  persistedActiveTabId = null;
}

interface TerminalManagerProps { initialTab?: TerminalTab | null; detached?: boolean }

export const TerminalManager = forwardRef<TerminalManagerHandle, TerminalManagerProps>(function TerminalManager({ initialTab, detached = false }, ref) {
  const { colors } = useTheme();
  // Read the durable record ONCE, during the render phase, the same way the
  // module cache is read — a later effect would be too late, because `Terminal`
  // spawns on its own mount.
  const storedRef = useRef<ReturnType<typeof readStoredState> | undefined>(undefined);
  if (storedRef.current === undefined) {
    storedRef.current = detached || initialTab ? null : readStoredState(storage());
  }
  const stored = storedRef.current;

  // A COLD start: neither the module cache nor the durable record knows
  // anything. That is exactly the state a re-evaluated realm comes back in, and
  // the only state in which an unclaimed live PTY means "the UI forgot me".
  // Start with NO tabs so nothing spawns before the backend has been asked; the
  // reconcile effect below fills the pane a tick later. Every other path keeps
  // its tabs immediately, so an ordinary pane toggle is unchanged.
  //
  // Only when there IS a backend to ask. Without the Tauri bridge (a browser
  // preview, a unit test) there is no question to wait for and no PTY to
  // collide with, so deferring would just be an empty pane for no reason.
  const coldRef = useRef<boolean | undefined>(undefined);
  if (coldRef.current === undefined) {
    const hasBridge = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    coldRef.current = hasBridge && !detached && !initialTab && !persistedTabs && !stored;
  }
  const coldStart = coldRef.current;

  const [tabs, setTabs] = useState<TerminalTab[]>(() => {
    if (initialTab) return [initialTab];
    if (!detached && persistedTabs) return persistedTabs;
    if (stored) return stored.tabs;
    return coldStart ? [] : [createTab()];
  });
  const [activeTabId, setActiveTabId] = useState<string>(() => {
    return (
      initialTab?.id ||
      (!detached ? persistedActiveTabId || stored?.activeTabId : null) ||
      tabs[0]?.id ||
      ''
    );
  });
  const [closingTabId, setClosingTabId] = useState<string | null>(null);
  // Drop-to-CC-terminal (#557): visual state while a file is dragged over the pane.
  const [dropActive, setDropActive] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const killPtyRef = useRef<(sessionId: string) => Promise<void>>();

  if (!killPtyRef.current) {
    killPtyRef.current = async (sessionId: string) => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('kill_pty', { sessionId });
      } catch {
        // ignore
      }
    };
  }

  const activeTabIdRef = useRef(activeTabId);
  activeTabIdRef.current = activeTabId;
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;

  const cycleTabs = useCallback((backwards = false) => {
    const nextId = nextPaneTabId(
      tabsRef.current.map(tab => tab.id),
      activeTabIdRef.current,
      backwards,
    );
    if (nextId) setActiveTabId(nextId);
  }, []);
  const selectPane = usePaneTabCycling('terminal', rootRef, cycleTabs);

  // Persist state CONTINUOUSLY, not only on unmount. A pane toggle in
  // BuildView remounts this manager in a SINGLE React commit (the Group's key
  // changes), and React runs the NEW instance's useState initializer during
  // the render phase — BEFORE the old instance's cleanup runs in the commit
  // phase. An unmount-only persist therefore handed the new instance stale
  // (or null) state: a fresh single tab over still-live PTYs, with the real
  // tabs only reappearing on the NEXT remount (reported 2026-08-06). Keeping
  // the module vars current on every change makes the initializer read the
  // truth no matter how the remount is scheduled.
  useEffect(() => {
    if (detached) return;
    persistedTabs = tabs;
    persistedActiveTabId = activeTabId;
    // …and durably, so the ids outlive the realm that is holding them.
    // Deliberately skipped while the cold start is still empty: writing `[]`
    // would erase the very record the next reload needs.
    if (tabs.length > 0) {
      writeStoredState(storage(), { tabs, activeTabId, detachedSessionIds });
    }
  }, [tabs, activeTabId, detached]);

  // ── Reconcile against the PTYs that actually exist ────────────────────────
  //
  // Runs once per mount. Two jobs, both of which used to be impossible because
  // nothing could ask the backend anything:
  //
  //   * a tab whose session has exited loses its id, so it gets a working shell
  //     instead of a pane wired to a session that is not there;
  //   * on a cold start, a live session no tab claims is ADOPTED — that is the
  //     session the user was working in, and burying it under a fresh shell is
  //     the reported bug.
  //
  // A FAILED listing changes nothing. Treating "I could not ask" as "nothing is
  // running" would clear every id and respawn every shell.
  useEffect(() => {
    if (detached) return;
    let cancelled = false;
    (async () => {
      let live: PtySessionInfo[] = [];
      let listed = false;
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        live = (await invoke('list_pty_sessions')) as PtySessionInfo[];
        listed = Array.isArray(live);
      } catch {
        listed = false;
      }
      if (cancelled) return;
      setTabs(prev => {
        const next = reconcileTabs({
          tabs: prev,
          live: listed ? live : [],
          listed,
          adopt: coldStart,
          detachedSessionIds,
          makeTab: () => createTab(),
        });
        setActiveTabId(id => resolveActiveTabId(next, id || null));
        return next;
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [detached, coldStart]);

  useEffect(() => {
    if (detached || !('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    import('@tauri-apps/api/event').then(({ listen }) => listen<{ kind: string; tab: TerminalTab }>('pane_redock', e => {
      if (e.payload.kind !== 'terminal') return;
      const sid = e.payload.tab.sessionId;
      if (sid) detachedSessionIds = detachedSessionIds.filter(id => id !== sid);
      setTabs(prev => [...prev.filter(t => t.id !== e.payload.tab.id), e.payload.tab]);
      setActiveTabId(e.payload.tab.id);
    })).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [detached]);

  // Drop-to-CC-terminal (#557): inject a dropped file's path into the PTY of the
  // active tab's session as if typed — a running Claude Code session then
  // receives the path in its input (no newline; the path is inserted, not
  // submitted). Depends on the scoped drop routing from #550: this zone claims
  // drops over its own bounds at a higher priority than the app-level chat zone.
  const injectPaths = useCallback(async (paths: string[]) => {
    const target = resolvePtyInjection(tabsRef.current, activeTabIdRef.current, paths);
    if (!target) return; // no live session, or nothing to inject
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('write_to_pty', { sessionId: target.sessionId, data: target.data });
    } catch (err) {
      console.error('[terminal] drop-to-CC path injection failed:', err);
    }
  }, []);

  useEffect(() => {
    return registerDropZone({
      id: 'build-terminal',
      getElement: () => rootRef.current,
      priority: 10, // win over the window-wide chat fallback (#550)
      onEnter: () => setDropActive(true),
      onLeave: () => setDropActive(false),
      onDrop: (paths) => {
        setDropActive(false);
        void injectPaths(paths);
      },
    });
  }, [injectPaths]);

  const handleNewTab = useCallback(() => {
    const tab = createTab();
    setTabs(prev => [...prev, tab]);
    setActiveTabId(tab.id);
  }, []);

  const handleCloseTab = useCallback((tabId: string, e?: React.MouseEvent) => {
    e?.stopPropagation();

    setClosingTabId(prev => {
      if (prev === tabId) {
        setTabs(prevTabs => {
          const tab = prevTabs.find(t => t.id === tabId);
          if (tab?.sessionId) {
            killPtyRef.current?.(tab.sessionId);
          }
          const next = prevTabs.filter(t => t.id !== tabId);
          if (next.length === 0) {
            const newTab = createTab();
            setActiveTabId(newTab.id);
            return [newTab];
          }
          if (tabId === activeTabIdRef.current) {
            const idx = prevTabs.findIndex(t => t.id === tabId);
            const nextActive = next[Math.min(idx, next.length - 1)];
            setActiveTabId(nextActive.id);
          }
          return next;
        });
        return null;
      }
      setTimeout(() => setClosingTabId(p => p === tabId ? null : p), 2000);
      return tabId;
    });
  }, []);

  const handleSessionSpawned = useCallback((tabId: string, sessionId: string) => {
    setTabs(prev =>
      prev.map(t => t.id === tabId ? { ...t, sessionId } : t)
    );
  }, []);

  const handleTitleChange = useCallback((tabId: string, title: string) => {
    setTabs(prev => {
      let label = title.trim();
      const pathMatch = label.match(/[~\/]([^:]+)$/);
      if (pathMatch) {
        // Title contains a path — extract last segment as folder name
        const segments = pathMatch[1].split('/').filter(Boolean);
        label = segments[segments.length - 1] || label;
      } else if (label === 'zsh' || label === 'bash' || label === 'sh' || !label) {
        // Shell name — only use if tab has no label yet
        const tab = prev.find(t => t.id === tabId);
        if (tab && tab.cwd) return prev;
        label = 'Terminal';
      } else {
        // Process names like "Claude Code" — don't overwrite
        return prev;
      }
      return prev.map(t => t.id === tabId ? { ...t, label } : t);
    });
  }, []);

  const handleCwdChange = useCallback((tabId: string, cwdPath: string) => {
    // cwdPath is like "/Users/jesse/dev/Canon" — extract last segment
    const segments = cwdPath.split('/').filter(Boolean);
    const folder = segments[segments.length - 1] || 'Terminal';
    setTabs(prev =>
      prev.map(t => t.id === tabId ? { ...t, label: folder, cwd: cwdPath } : t)
    );
  }, []);

  const createProjectTab = useCallback((
    cwd: string,
    label: string,
    initialCommand?: string,
    supervisedSessionId?: string,
    extras?: {
      followUpInput?: string;
      growthAction?: { projectId: string; actionId: string };
      launchId?: string;
    },
  ) => {
    // Second-layer guard (see pendingLaunch.ts) — BuildView already claims
    // the id before calling here; this refuses a duplicate call for the same
    // id regardless of how it arrived.
    if (extras?.launchId && !projectTabLaunchClaims.claim(extras.launchId)) return;
    const tab: TerminalTab = {
      ...createTab(cwd),
      label,
      initialCommand,
      supervisedSessionId,
      followUpInput: extras?.followUpInput,
      growthAction: extras?.growthAction,
    };
    setTabs(prev => [...prev, tab]);
    setActiveTabId(tab.id);
  }, []);

  useImperativeHandle(ref, () => ({
    createProjectTab,
    getActiveTab: () => tabsRef.current.find(t => t.id === activeTabIdRef.current) || tabsRef.current[0],
    getAllTabs: () => tabsRef.current,
    killTab: async (sessionId: string) => { await killPtyRef.current?.(sessionId); },
  }), [createProjectTab]);

  // Returns whether the pop-out actually happened: this is a `Button` onClick,
  // and the primitive ticks on a resolved promise unless it resolves `false`.
  // The catch below swallows its own error, so without this a failed pop-out
  // would confirm itself.
  const popOutActive = useCallback(async () => {
    const tab = tabsRef.current.find(t => t.id === activeTabIdRef.current);
    if (!tab || detached) return false;
    try {
      const { createPaneWindow } = await import('../../lib/paneWindows');
      await createPaneWindow('terminal', tab);
      // The pane window owns this session now. Remember that, or a cold start
      // here would adopt it back and wire two panes to one PTY.
      if (tab.sessionId && !detachedSessionIds.includes(tab.sessionId)) {
        detachedSessionIds = [...detachedSessionIds, tab.sessionId];
      }
      setTabs(prev => {
        const next = prev.filter(t => t.id !== tab.id);
        if (next.length) { setActiveTabId(next[0].id); return next; }
        const replacement = createTab();
        setActiveTabId(replacement.id);
        return [replacement];
      });
      return true;
    } catch (err) { console.error('[terminal] pop-out failed:', err); return false; }
  }, [detached]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) {
        if (e.key === 't') {
          e.preventDefault();
          handleNewTab();
        } else if (e.key === 'w') {
          e.preventDefault();
          handleCloseTab(activeTabIdRef.current);
        }
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [handleNewTab, handleCloseTab]);

  return (
    <div ref={rootRef} onFocusCapture={selectPane} className="relative flex h-full flex-col" style={{ backgroundColor: colors.bg }}>
      {/* Drop-to-CC-terminal overlay (#557): shown while a file is dragged over
          the pane. pointer-events-none so it never intercepts the native drop. */}
      {dropActive && (
        <div
          className="absolute inset-0 z-40 flex flex-col items-center justify-center rounded-xl m-2 pointer-events-none"
          style={{ backgroundColor: colors.bg, opacity: 0.93, border: `2px dashed ${colors.cyan}80` }}
        >
          <FiFilePlus size={32} className="mb-2" style={{ color: `${colors.cyan}99` }} />
          <span className="text-sm" style={{ fontFamily: font.mono, color: `${colors.cyan}CC` }}>
            Drop to add file path to the terminal
          </span>
        </div>
      )}
      <div className="flex items-center border-b border-dark-border" style={{ backgroundColor: colors.surface }}>
        <div className="flex flex-1 items-center overflow-x-auto">
          {tabs.map(tab => (
            <Button
              key={tab.id}
              colors={colors}
              variant="bare"
              onClick={() => setActiveTabId(tab.id)}
              // `group` stays: the close affordance reveals on group-hover.
              className="group shrink-0"
              style={{
                '--pa-btn-bg': tab.id === activeTabId ? colors.bg : 'transparent',
                '--pa-btn-fg': tab.id === activeTabId ? colors.cyan : colors.textMuted,
                '--pa-btn-border': colors.border,
                '--pa-btn-bg-hover': tab.id === activeTabId ? colors.bg : 'rgba(255,255,255,0.05)',
                '--pa-btn-fg-hover': tab.id === activeTabId ? colors.cyan : colors.text,
                '--pa-btn-bg-active': tab.id === activeTabId ? colors.bg : 'rgba(255,255,255,0.05)',
                '--pa-btn-border-hover': colors.border,
                '--pa-btn-pad': '6px 12px',
                '--pa-btn-radius': '0',
                // `.pa-btn` carries a border on all four edges; a tab has one
                // only on its right. Widths here, colour still from the vars
                // above, so the hover rule keeps working.
                borderWidth: '0 1px 0 0',
                fontFamily: font.mono,
                fontSize: textSize.micro,
                gap: 6,
              } as CSSProperties}
            >
              <FiTerminal size={11} />
              <span className="truncate max-w-[140px]">{tab.label}</span>
              <span
                onClick={(e) => handleCloseTab(tab.id, e)}
                className={`ml-1 rounded p-0.5 transition-colors ${
                  closingTabId === tab.id
                    ? 'bg-red-500/20 text-red-400'
                    : 'opacity-0 group-hover:opacity-100 hover:bg-white/10 text-dark-muted'
                }`}
              >
                <FiX size={10} />
              </span>
            </Button>
          ))}
        </div>
        <CycleTabsButton pane="terminal" onCycle={() => cycleTabs()} />
        {!detached && (
          <Button
            colors={colors}
            variant="bare"
            onClick={popOutActive}
            title="Pop out active terminal"
            aria-label="Pop out active terminal"
            style={{
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-fg-hover': colors.cyan,
              '--pa-btn-pad': '6px 8px',
            } as CSSProperties}
          >
            <FiExternalLink size={13} />
          </Button>
        )}
        <Button
          colors={colors}
          variant="bare"
          onClick={handleNewTab}
          title="New terminal (Cmd+T)"
          aria-label="New terminal"
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.cyan,
            '--pa-btn-pad': '6px 10px',
          } as CSSProperties}
        >
          <FiPlus size={13} />
        </Button>
      </div>

      <div className="flex-1 min-h-0 relative">
        {tabs.map(tab => (
          <div
            key={tab.id}
            className="absolute inset-0"
            style={{ display: tab.id === activeTabId ? 'block' : 'none' }}
          >
            <Terminal
              sessionId={tab.sessionId}
              onSessionSpawned={(sid) => handleSessionSpawned(tab.id, sid)}
              onTitleChange={(title) => handleTitleChange(tab.id, title)}
              onCwdChange={(cwd) => handleCwdChange(tab.id, cwd)}
              cwd={tab.cwd}
              initialCommand={tab.initialCommand}
              supervisedSessionId={tab.supervisedSessionId}
              followUpInput={tab.followUpInput}
              growthAction={tab.growthAction}
              isVisible={tab.id === activeTabId}
            />
          </div>
        ))}
      </div>
    </div>
  );
});
