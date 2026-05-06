import { useState, useCallback, useRef, useEffect } from 'react';
import { FiPlus, FiX, FiTerminal } from 'react-icons/fi';
import { Terminal } from './Terminal';

interface TerminalTab {
  id: string;
  label: string;
  sessionId: string | null;
  cwd?: string;
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

export function TerminalManager() {
  const [tabs, setTabs] = useState<TerminalTab[]>(() => {
    if (persistedTabs) return persistedTabs;
    return [createTab()];
  });
  const [activeTabId, setActiveTabId] = useState<string>(() => {
    return persistedActiveTabId || tabs[0].id;
  });
  const [closingTabId, setClosingTabId] = useState<string | null>(null);
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

  // Persist state on unmount so terminal survives workspace switches
  useEffect(() => {
    return () => {
      persistedTabs = tabsRef.current;
      persistedActiveTabId = activeTabIdRef.current;
    };
  }, []);

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
    <div className="flex h-full flex-col bg-[#0A0E17]">
      <div className="flex items-center border-b border-dark-border bg-[#0B1120]">
        <div className="flex flex-1 items-center overflow-x-auto">
          {tabs.map(tab => (
            <button
              key={tab.id}
              onClick={() => setActiveTabId(tab.id)}
              className={`group flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-mono border-r border-dark-border transition-colors shrink-0 ${
                tab.id === activeTabId
                  ? 'bg-[#0A0E17] text-accent'
                  : 'text-dark-muted hover:text-dark-text hover:bg-white/5'
              }`}
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
              isVisible={tab.id === activeTabId}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
