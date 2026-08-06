import { useState, useCallback, useRef, useEffect, useImperativeHandle, forwardRef } from 'react';
import { FiPlus, FiX, FiTerminal, FiFilePlus, FiExternalLink } from 'react-icons/fi';
import { Terminal } from './Terminal';
import { useTheme } from '../../styles/useTheme';
import { registerDropZone } from '../../lib/native-drag-drop';
import { resolvePtyInjection } from './terminalDrop';
import { font } from '../../styles/tokens';
import { CycleTabsButton } from '../build/CycleTabsButton';
import { nextPaneTabId, usePaneTabCycling } from '../build/paneTabCycling';

export interface TerminalManagerHandle {
  createProjectTab: (cwd: string, label: string, initialCommand?: string, supervisedSessionId?: string) => void;
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

/** Test-only: clear the module-level persistence between test cases. */
export function __resetTerminalPersistenceForTests() {
  persistedTabs = null;
  persistedActiveTabId = null;
}

interface TerminalManagerProps { initialTab?: TerminalTab | null; detached?: boolean }

export const TerminalManager = forwardRef<TerminalManagerHandle, TerminalManagerProps>(function TerminalManager({ initialTab, detached = false }, ref) {
  const { colors } = useTheme();
  const [tabs, setTabs] = useState<TerminalTab[]>(() => {
    if (initialTab) return [initialTab];
    if (!detached && persistedTabs) return persistedTabs;
    return [createTab()];
  });
  const [activeTabId, setActiveTabId] = useState<string>(() => {
    return initialTab?.id || (!detached ? persistedActiveTabId : null) || tabs[0].id;
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
    if (!detached) {
      persistedTabs = tabs;
      persistedActiveTabId = activeTabId;
    }
  }, [tabs, activeTabId, detached]);

  useEffect(() => {
    if (detached || !('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    import('@tauri-apps/api/event').then(({ listen }) => listen<{ kind: string; tab: TerminalTab }>('pane_redock', e => {
      if (e.payload.kind !== 'terminal') return;
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

  const createProjectTab = useCallback((cwd: string, label: string, initialCommand?: string, supervisedSessionId?: string) => {
    const tab: TerminalTab = {
      ...createTab(cwd),
      label,
      initialCommand,
      supervisedSessionId,
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

  const popOutActive = useCallback(async () => {
    const tab = tabsRef.current.find(t => t.id === activeTabIdRef.current);
    if (!tab || detached) return;
    try {
      const { createPaneWindow } = await import('../../lib/paneWindows');
      await createPaneWindow('terminal', tab);
      setTabs(prev => {
        const next = prev.filter(t => t.id !== tab.id);
        if (next.length) { setActiveTabId(next[0].id); return next; }
        const replacement = createTab();
        setActiveTabId(replacement.id);
        return [replacement];
      });
    } catch (err) { console.error('[terminal] pop-out failed:', err); }
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
            <button
              key={tab.id}
              onClick={() => setActiveTabId(tab.id)}
              className={`group flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-mono border-r border-dark-border transition-colors shrink-0 ${
                tab.id === activeTabId
                  ? 'text-accent'
                  : 'text-dark-muted hover:text-dark-text hover:bg-white/5'
              }`}
              style={tab.id === activeTabId ? { backgroundColor: colors.bg } : undefined}
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
            </button>
          ))}
        </div>
        <CycleTabsButton pane="terminal" onCycle={() => cycleTabs()} />
        {!detached && <button onClick={popOutActive} className="px-2 py-1.5 text-dark-muted hover:text-accent transition-colors" title="Pop out active terminal"><FiExternalLink size={13} /></button>}
        <button
          onClick={handleNewTab}
          className="px-2.5 py-1.5 text-dark-muted hover:text-accent transition-colors"
          title="New terminal (Cmd+T)"
        >
          <FiPlus size={13} />
        </button>
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
              isVisible={tab.id === activeTabId}
            />
          </div>
        ))}
      </div>
    </div>
  );
});
