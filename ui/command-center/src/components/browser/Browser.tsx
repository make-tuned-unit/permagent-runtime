import { useState, useCallback, useRef, useEffect } from 'react';
import { useCommandCenter } from '../../lib/store';
import { useBrowserContentBridge } from '../../hooks/useBrowserContentBridge';
import {
  FiArrowLeft,
  FiArrowRight,
  FiRefreshCw,
  FiLock,
  FiAlertTriangle,
  FiShield,
  FiGlobe,
} from 'react-icons/fi';
import { BrowserTabs, type BrowserTab } from './BrowserTabs';

// ── Tauri API loader (cached, no module-level mutation) ──

interface TauriApi {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (event: string, handler: (e: { payload: unknown }) => void) => Promise<() => void>;
}

let cachedApi: TauriApi | null = null;
let apiPromise: Promise<TauriApi | null> | null = null;

function getTauriApi(): Promise<TauriApi | null> {
  if (cachedApi) return Promise.resolve(cachedApi);
  if (!apiPromise) {
    apiPromise = (async () => {
      try {
        const core = await import('@tauri-apps/api/core');
        const event = await import('@tauri-apps/api/event');
        cachedApi = { invoke: core.invoke, listen: event.listen };
        return cachedApi;
      } catch {
        return null;
      }
    })();
  }
  return apiPromise;
}

// TODO: Frontend-driven emission is transitional. Lifecycle hooks should move
// to Rust-owned surfaces in Phase 2.5 (see docs/architecture/PHASE_2_5_TAURI_REFACTOR.md).
function emitActivity(api: TauriApi | null, eventType: string, payload: Record<string, unknown>) {
  if (!api) return;
  api.invoke('emit_activity', {
    event_type: eventType,
    source_surface: 'browser',
    payload,
    session_id: null,
    project_id: null,
  }).catch((err: unknown) => console.debug('[activity] emit failed:', err));
}

let tabCounter = 0;

function createTab(): BrowserTab {
  tabCounter++;
  return {
    id: `btab-${Date.now()}-${tabCounter}`,
    label: 'New Tab',
    webviewId: null,
    url: '',
    loading: false,
  };
}

function getUrlProtocol(url: string): 'https' | 'http' | 'other' {
  if (url.startsWith('https://')) return 'https';
  if (url.startsWith('http://')) return 'http';
  return 'other';
}

function normalizeUrl(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return '';
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  if (/^[a-zA-Z0-9-]+\.[a-zA-Z]{2,}/.test(trimmed)) return `https://${trimmed}`;
  return `https://www.google.com/search?q=${encodeURIComponent(trimmed)}`;
}

// ── Module-level state persists across workspace switches (mount/unmount) ──
let persistedTabs: BrowserTab[] | null = null;
let persistedActiveTabId: string | null = null;

export function Browser() {
  const overlayBlocking = useCommandCenter(s => s.overlayBlockingBrowser);

  const [tabs, setTabs] = useState<BrowserTab[]>(() => {
    if (persistedTabs) return persistedTabs;
    return [createTab()];
  });
  const [activeTabId, setActiveTabId] = useState<string>(() => {
    return persistedActiveTabId || tabs[0].id;
  });
  const [closingTabId, setClosingTabId] = useState<string | null>(null);
  const [urlInput, setUrlInput] = useState('');
  const [api, setApi] = useState<TauriApi | null>(null);
  const [zoomLevel, setZoomLevel] = useState(1.0);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Refs that always hold the latest value (for callbacks and cleanup)
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeTabIdRef = useRef(activeTabId);
  activeTabIdRef.current = activeTabId;
  const apiRef = useRef(api);
  apiRef.current = api;

  const activeTab = tabs.find((t) => t.id === activeTabId);

  // Bridge: daemon MCP tool → Tauri webview content extraction → daemon fulfillment
  useBrowserContentBridge(activeTab?.webviewId ?? null);

  // Initialize Tauri API
  useEffect(() => {
    getTauriApi().then((resolved) => {
      setApi(resolved);
    });
  }, []);

  // ── Persist state and hide webviews on unmount (workspace switch) ──
  useEffect(() => {
    return () => {
      persistedTabs = tabsRef.current;
      persistedActiveTabId = activeTabIdRef.current;
      // Move all child webviews offscreen
      const inv = apiRef.current;
      tabsRef.current.forEach((t) => {
        if (t.webviewId && inv) {
          inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
        }
      });
    };
  }, []);

  // ── Sync active webview position with the container div ──
  const syncBounds = useCallback(() => {
    const inv = apiRef.current;
    if (!containerRef.current || !inv) return;
    const rect = containerRef.current.getBoundingClientRect();

    const currentTabs = tabsRef.current;
    const currentActiveId = activeTabIdRef.current;

    // Hide webviews when container is hidden (workspace switch)
    if (rect.width === 0 || rect.height === 0) {
      currentTabs.forEach((t) => {
        if (t.webviewId) inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
      });
      return;
    }

    currentTabs.forEach((t) => {
      if (!t.webviewId) return;
      if (t.id === currentActiveId) {
        inv.invoke('update_browser_bounds', {
          webviewId: t.webviewId,
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height,
        }).catch(() => {});
      } else {
        inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
      }
    });
  }, []);

  // ── ResizeObserver + visibility polling keeps webview in sync ──
  useEffect(() => {
    if (!containerRef.current || !api) return;

    // Restore any persisted webviews on mount
    syncBounds();

    const observer = new ResizeObserver(() => syncBounds());
    observer.observe(containerRef.current);

    // ResizeObserver doesn't fire on display:none changes (workspace switches).
    // Poll visibility at low frequency so native webviews get hidden promptly.
    const visibilityInterval = setInterval(syncBounds, 500);

    window.addEventListener('resize', syncBounds);

    return () => {
      observer.disconnect();
      clearInterval(visibilityInterval);
      window.removeEventListener('resize', syncBounds);
    };
  }, [api, syncBounds]);

  // ── Re-sync whenever active tab changes ──
  useEffect(() => {
    syncBounds();
  }, [activeTabId, tabs, syncBounds]);

  // ── Hide all webviews when a transient overlay is open ──
  useEffect(() => {
    if (overlayBlocking > 0) {
      const inv = apiRef.current;
      if (inv) {
        tabsRef.current.forEach(t => {
          if (t.webviewId) inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
        });
      }
    } else {
      syncBounds();
    }
  }, [overlayBlocking, syncBounds]);

  // Listen for navigation events from Tauri
  useEffect(() => {
    if (!api) return;

    let unlisten: (() => void) | null = null;

    api.listen('browser_navigated', (e) => {
      const payload = e.payload as { webview_id: string; url: string };
      // Activity: browser navigated
      const navTab = tabsRef.current.find((t) => t.webviewId === payload.webview_id);
      emitActivity(api, 'browser_navigated', {
        url: payload.url,
        title: extractTitle(payload.url),
        referrer: navTab?.url || '',
        tab_id: navTab?.id || payload.webview_id,
      });
      setTabs((prev) =>
        prev.map((t) =>
          t.webviewId === payload.webview_id
            ? { ...t, url: payload.url, label: extractTitle(payload.url), loading: false }
            : t,
        ),
      );
      // Update URL bar if this is the active tab
      setTabs((prev) => {
        const active = prev.find((t) => t.id === activeTabIdRef.current);
        if (active?.webviewId === payload.webview_id) {
          setUrlInput(payload.url);
        }
        return prev;
      });
    }).then((fn) => {
      unlisten = fn;
    });

    // Listen for page title changes from the native webview
    let unlistenTitle: (() => void) | null = null;
    api.listen('browser_title_changed', (e) => {
      const payload = e.payload as { webview_id: string; title: string };
      setTabs((prev) =>
        prev.map((t) =>
          t.webviewId === payload.webview_id
            ? { ...t, label: payload.title }
            : t,
        ),
      );
    }).then((fn) => {
      unlistenTitle = fn;
    });

    // Listen for OAuth URL open events
    let unlistenOAuth: (() => void) | null = null;
    api.listen('browser_open_url', (e) => {
      const payload = e.payload as { url: string; oauth?: boolean; provider?: string };
      handleOpenUrl(payload.url, payload.oauth ? `OAuth: ${payload.provider || ''}` : undefined);
    }).then((fn) => {
      unlistenOAuth = fn;
    });

    // Listen for OAuth completion to close the OAuth tab
    let unlistenOAuthComplete: (() => void) | null = null;
    api.listen('browser_oauth_complete', () => {
      // Find and close any tab with an OAuth URL
      setTabs((prev) => {
        const oauthTab = prev.find(
          (t) => t.url.includes('accounts.google.com') || t.label.startsWith('OAuth:'),
        );
        if (oauthTab?.webviewId && apiRef.current) {
          apiRef.current.invoke('close_browser', { webviewId: oauthTab.webviewId }).catch(() => {});
        }
        const remaining = prev.filter(
          (t) => !t.url.includes('accounts.google.com') && !t.label.startsWith('OAuth:'),
        );
        if (remaining.length === 0) {
          const newTab = createTab();
          setActiveTabId(newTab.id);
          return [newTab];
        }
        return remaining;
      });
    }).then((fn) => {
      unlistenOAuthComplete = fn;
    });

    return () => {
      unlisten?.();
      unlistenTitle?.();
      unlistenOAuth?.();
      unlistenOAuthComplete?.();
    };
  }, [api]);

  function extractTitle(url: string): string {
    try {
      const u = new URL(url);
      return u.hostname.replace(/^www\./, '');
    } catch {
      return url.slice(0, 30);
    }
  }

  const handleOpenUrl = useCallback(
    async (url: string, label?: string) => {
      if (!apiRef.current) return;

      const rect = containerRef.current?.getBoundingClientRect();
      const tab = createTab();
      tab.url = url;
      tab.label = label || extractTitle(url);
      tab.loading = true;

      try {
        const webviewId = (await apiRef.current.invoke('create_browser_webview', {
          url,
          x: rect?.x ?? 0,
          y: rect?.y ?? 0,
          width: rect?.width ?? 800,
          height: rect?.height ?? 600,
        })) as string;
        tab.webviewId = webviewId;
        tab.loading = false;
      } catch (err) {
        tab.label = `Error: ${err}`;
        tab.loading = false;
      }

      setTabs((prev) => [...prev, tab]);
      setActiveTabId(tab.id);
      setUrlInput(url);
    },
    [],
  );

  const handleNavigate = useCallback(
    async (url: string) => {
      const normalized = normalizeUrl(url);
      if (!normalized || !apiRef.current) return;

      const tab = tabs.find((t) => t.id === activeTabId);
      if (!tab) return;

      setUrlInput(normalized);

      if (tab.webviewId) {
        // Navigate existing webview
        try {
          await apiRef.current.invoke('navigate_browser', {
            webviewId: tab.webviewId,
            url: normalized,
          });
          setTabs((prev) =>
            prev.map((t) =>
              t.id === activeTabId
                ? { ...t, url: normalized, label: extractTitle(normalized), loading: true }
                : t,
            ),
          );
        } catch (err) {
          console.error('Navigate failed:', err);
        }
      } else {
        // Create new child webview for this tab
        const rect = containerRef.current?.getBoundingClientRect();
        try {
          const webviewId = (await apiRef.current.invoke('create_browser_webview', {
            url: normalized,
            x: rect?.x ?? 0,
            y: rect?.y ?? 0,
            width: rect?.width ?? 800,
            height: rect?.height ?? 600,
          })) as string;
          // Activity: browser session started
          emitActivity(apiRef.current, 'browser_session_started', { tab_id: tab.id });
          setTabs((prev) =>
            prev.map((t) =>
              t.id === activeTabId
                ? {
                    ...t,
                    webviewId,
                    url: normalized,
                    label: extractTitle(normalized),
                    loading: false,
                  }
                : t,
            ),
          );
        } catch (err) {
          console.error('Create webview failed:', err);
        }
      }
    },
    [tabs, activeTabId],
  );

  const handleNewTab = useCallback(() => {
    const tab = createTab();
    setTabs((prev) => [...prev, tab]);
    setActiveTabId(tab.id);
    setUrlInput('');
    setTimeout(() => urlInputRef.current?.focus(), 50);
  }, []);

  const handleCloseTab = useCallback((tabId: string, e?: React.MouseEvent) => {
    e?.stopPropagation();

    setClosingTabId((prev) => {
      if (prev === tabId) {
        // Double-click to confirm close
        setTabs((prevTabs) => {
          const tab = prevTabs.find((t) => t.id === tabId);
          if (tab?.webviewId && apiRef.current) {
            // Activity: browser session ended
            emitActivity(apiRef.current, 'browser_session_ended', { tab_id: tab.id });
            apiRef.current.invoke('close_browser', { webviewId: tab.webviewId }).catch(() => {});
          }
          const next = prevTabs.filter((t) => t.id !== tabId);
          if (next.length === 0) {
            const newTab = createTab();
            setActiveTabId(newTab.id);
            setUrlInput('');
            return [newTab];
          }
          if (tabId === activeTabIdRef.current) {
            const idx = prevTabs.findIndex((t) => t.id === tabId);
            const nextActive = next[Math.min(idx, next.length - 1)];
            setActiveTabId(nextActive.id);
            setUrlInput(nextActive.url);
          }
          return next;
        });
        return null;
      }
      setTimeout(() => setClosingTabId((p) => (p === tabId ? null : p)), 2000);
      return tabId;
    });
  }, []);

  const handleSelectTab = useCallback(
    (tabId: string) => {
      setActiveTabId(tabId);
      const tab = tabs.find((t) => t.id === tabId);
      if (tab) {
        setUrlInput(tab.url);
      }
      // syncBounds will fire from the activeTabId effect
    },
    [tabs],
  );

  const handleReload = useCallback(() => {
    if (!activeTab?.webviewId || !apiRef.current) return;
    apiRef.current.invoke('navigate_browser', {
      webviewId: activeTab.webviewId,
      url: activeTab.url,
    }).catch(() => {});
  }, [activeTab]);

  const handleZoom = useCallback((delta: number) => {
    if (!activeTab?.webviewId || !apiRef.current) return;
    const newZoom = Math.max(0.25, Math.min(3.0, zoomLevel + delta));
    setZoomLevel(newZoom);
    apiRef.current.invoke('zoom_browser', {
      webviewId: activeTab.webviewId,
      zoomLevel: newZoom,
    }).catch(() => {});
  }, [activeTab, zoomLevel]);

  const handleUrlKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleNavigate(urlInput);
      urlInputRef.current?.blur();
    } else if (e.key === 'Escape') {
      setUrlInput(activeTab?.url || '');
      urlInputRef.current?.blur();
    }
  };

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) {
        switch (e.key) {
          case 't':
            e.preventDefault();
            handleNewTab();
            break;
          case 'w':
            e.preventDefault();
            handleCloseTab(activeTabIdRef.current);
            break;
          case 'l':
            e.preventDefault();
            urlInputRef.current?.focus();
            urlInputRef.current?.select();
            break;
          case 'r':
            e.preventDefault();
            handleReload();
            break;
          case '=':
          case '+':
            e.preventDefault();
            handleZoom(0.1);
            break;
          case '-':
            e.preventDefault();
            handleZoom(-0.1);
            break;
          case '0':
            e.preventDefault();
            setZoomLevel(1.0);
            if (activeTabIdRef.current) {
              const tab = tabsRef.current.find(t => t.id === activeTabIdRef.current);
              if (tab?.webviewId && apiRef.current) {
                apiRef.current.invoke('zoom_browser', { webviewId: tab.webviewId, zoomLevel: 1.0 }).catch(() => {});
              }
            }
            break;
        }
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [handleNewTab, handleCloseTab, handleReload, handleZoom]);

  const protocol = activeTab?.url ? getUrlProtocol(activeTab.url) : 'other';

  return (
    <div className="flex h-full flex-col bg-[#0A0E17]">
      {/* Tab bar */}
      <BrowserTabs
        tabs={tabs}
        activeTabId={activeTabId}
        closingTabId={closingTabId}
        onSelectTab={handleSelectTab}
        onCloseTab={handleCloseTab}
        onNewTab={handleNewTab}
      />

      {/* URL bar */}
      <div className="flex items-center gap-2 border-b border-dark-border bg-[#0B1120] px-3 py-2">
        {/* Nav buttons */}
        <div className="flex items-center gap-1">
          <button
            className="p-1.5 rounded text-dark-muted hover:text-dark-text hover:bg-white/5 transition-colors"
            title="Back (Cmd+[)"
            disabled
          >
            <FiArrowLeft size={14} />
          </button>
          <button
            className="p-1.5 rounded text-dark-muted hover:text-dark-text hover:bg-white/5 transition-colors"
            title="Forward (Cmd+])"
            disabled
          >
            <FiArrowRight size={14} />
          </button>
          <button
            onClick={handleReload}
            className="p-1.5 rounded text-dark-muted hover:text-dark-text hover:bg-white/5 transition-colors"
            title="Reload (Cmd+R)"
            disabled={!activeTab?.webviewId}
          >
            <FiRefreshCw size={14} className={activeTab?.loading ? 'animate-spin' : ''} />
          </button>
        </div>

        {/* Address bar */}
        <div className="flex-1 flex items-center bg-[#0A0E17] rounded-md border border-dark-border focus-within:border-accent transition-colors">
          <span className="pl-2.5 pr-1">
            {protocol === 'https' ? (
              <FiLock size={12} className="text-accent" />
            ) : protocol === 'http' ? (
              <FiAlertTriangle size={12} className="text-amber-400" />
            ) : (
              <FiShield size={12} className="text-dark-muted" />
            )}
          </span>
          <input
            ref={urlInputRef}
            type="text"
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            onKeyDown={handleUrlKeyDown}
            onFocus={(e) => e.target.select()}
            placeholder="Search or enter URL..."
            className="flex-1 bg-transparent text-xs text-dark-text placeholder:text-dark-muted/50 py-1.5 pr-3 outline-none font-mono"
          />
        </div>
      </div>

      {/* Content area — the child webview overlays this div */}
      <div ref={containerRef} className="flex-1 min-h-0 relative">
        {!api ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center p-8">
              <FiShield size={48} className="mx-auto mb-4 text-dark-muted" />
              <h3 className="text-lg font-semibold text-dark-text mb-2">Desktop App Required</h3>
              <p className="text-xs text-dark-muted max-w-md">
                The embedded browser requires the Permagent desktop app. Run{' '}
                <code className="bg-black/30 px-1.5 py-0.5 rounded text-accent">
                  npm run tauri:dev
                </code>{' '}
                to use this feature.
              </p>
            </div>
          </div>
        ) : !activeTab?.webviewId ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center p-8">
              <FiGlobe size={48} className="mx-auto mb-4 text-accent/30" />
              <h3 className="text-sm font-medium text-dark-text mb-2">Ready to Browse</h3>
              <p className="text-xs text-dark-muted">
                Enter a URL above or press{' '}
                <kbd className="bg-dark-border px-1.5 py-0.5 rounded text-[10px]">Cmd+L</kbd> to
                focus the address bar
              </p>
              <div className="mt-6 flex flex-wrap gap-2 justify-center">
                {['google.com', 'github.com', 'accounts.google.com'].map((site) => (
                  <button
                    key={site}
                    onClick={() => handleNavigate(`https://${site}`)}
                    className="px-3 py-1.5 rounded-md bg-white/5 text-xs text-dark-muted hover:text-accent hover:bg-accent/10 transition-colors"
                  >
                    {site}
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : null /* Child webview renders natively over this area */}
      </div>

      {/* Status bar */}
      <div className="flex items-center gap-3 border-t border-dark-border bg-[#0B1120] px-3 py-1">
        <span className="text-[10px] font-mono text-dark-muted flex-1 truncate">
          {activeTab?.loading
            ? 'Loading...'
            : activeTab?.webviewId
              ? activeTab.url
              : 'Ready'}
        </span>
        {activeTab?.webviewId && (
          <span className="flex items-center gap-1 text-[10px] font-mono text-dark-muted">
            <button onClick={() => handleZoom(-0.1)} className="hover:text-accent transition-colors px-0.5">−</button>
            <span className="w-8 text-center">{Math.round(zoomLevel * 100)}%</span>
            <button onClick={() => handleZoom(0.1)} className="hover:text-accent transition-colors px-0.5">+</button>
          </span>
        )}
        <span className="text-[10px] font-mono text-dark-muted">
          {tabs.length} tab{tabs.length !== 1 ? 's' : ''}
        </span>
      </div>
    </div>
  );
}
