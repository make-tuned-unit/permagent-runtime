import { useState, useCallback, useRef, useEffect } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useBrowserContentBridge } from '../../hooks/useBrowserContentBridge';
import { useBrowserActBridge } from '../../hooks/useBrowserActBridge';
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
import { CHAT_LAUNCHER_MARGIN } from '../chat/ChatLauncher';
import { CHAT_DOCK_WIDTH } from '../chat/ChatDock';

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
  const { colors } = useTheme();
  const overlayBlocking = useCommandCenter(s => s.overlayBlockingBrowser);
  const chatLauncherSize = useCommandCenter(s => s.chatLauncherSize);
  const chatDockOpen = useCommandCenter(s => s.chatDockOpen);
  const pendingBrowserUrl = useCommandCenter(s => s.pendingBrowserUrl);
  const clearPendingBrowserUrl = useCommandCenter(s => s.clearPendingBrowserUrl);

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
  const chatLauncherSizeRef = useRef(chatLauncherSize);
  chatLauncherSizeRef.current = chatLauncherSize;
  const chatDockOpenRef = useRef(chatDockOpen);
  chatDockOpenRef.current = chatDockOpen;

  const activeTab = tabs.find((t) => t.id === activeTabId);

  // Bridge: daemon MCP tool → Tauri webview content extraction → daemon fulfillment
  useBrowserContentBridge(activeTab?.webviewId ?? null);
  // Bridge: daemon MCP tool → Tauri webview snapshot/act (#649) → daemon fulfillment
  useBrowserActBridge(activeTab?.webviewId ?? null);

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

    // Bounds-subtract the collapsed chat launcher's corner (#553, ruled D2 in
    // WEBVIEW_LIFECYCLE.md). The launcher is DOM inside the main webview; the
    // browser is a native child surface that always composites above it, so
    // the only way to keep the launcher visible is to keep the webview's rect
    // out of its corner. When the webview's bottom-right reaches the reserved
    // corner (launcher size + margins, anchored bottom-right), raise the
    // webview's bottom edge above it. When the chat window is open the
    // launcher unmounts (size = null) and the webview gets the full rect —
    // that window is a separate native surface ordered above main already.
    let height = rect.height;
    let width = rect.width;
    // Chat dock (2026-07-11): when the sidebar is open, the native webview must
    // not paint under it — reserve its right strip (matches the DOM overlay).
    if (chatDockOpenRef.current && window.innerWidth >= 640) {
      const dockLeft = window.innerWidth - CHAT_DOCK_WIDTH;
      if (rect.x + width > dockLeft) {
        width = Math.max(0, dockLeft - rect.x);
      }
    }
    const launcher = chatLauncherSizeRef.current;
    if (launcher) {
      const reservedTop = window.innerHeight - launcher.height - 2 * CHAT_LAUNCHER_MARGIN;
      const reservedLeft = window.innerWidth - launcher.width - 2 * CHAT_LAUNCHER_MARGIN;
      const intersectsCorner =
        rect.y + rect.height > reservedTop && rect.x + rect.width > reservedLeft;
      if (intersectsCorner && reservedTop - rect.y > 0) {
        height = reservedTop - rect.y;
      }
    }

    currentTabs.forEach((t) => {
      if (!t.webviewId) return;
      if (t.id === currentActiveId) {
        inv.invoke('update_browser_bounds', {
          webviewId: t.webviewId,
          x: rect.x,
          y: rect.y,
          width,
          height,
        }).catch(() => {});
      } else {
        inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
      }
    });
  }, []);

  // ── ResizeObserver + visibility polling keeps webview in sync ──
  //
  // Nap-safe pump (#562 C1). The 500 ms poll drives a MAIN-THREAD native op
  // (`set_position`/`set_size`) on the child WKWebView every tick. Left running
  // while the window is hidden/occluded it keeps the app's AppKit main thread
  // coupled to a throttled/App-Napped WebContent process — the mechanism of the
  // idle-wedge. So we SUSPEND the pump whenever `document.hidden` and resume it
  // (with one immediate re-sync) on return. Nothing keeps the main thread busy
  // against a napped surface while idle, so App-Nap becomes harmless rather than
  // something to hold a wake-lock against. ResizeObserver/resize fire only on
  // real layout changes (never while hidden), so they need no gating.
  useEffect(() => {
    if (!containerRef.current || !api) return;

    // Restore any persisted webviews on mount
    syncBounds();

    const observer = new ResizeObserver(() => syncBounds());
    observer.observe(containerRef.current);

    // The 500 ms poll exists because ResizeObserver doesn't fire on display:none
    // changes (workspace switches). Gate it on visibility so it never runs while
    // occluded.
    let pump: ReturnType<typeof setInterval> | null = null;
    const startPump = () => {
      if (pump === null) pump = setInterval(syncBounds, 500);
    };
    const stopPump = () => {
      if (pump !== null) {
        clearInterval(pump);
        pump = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) {
        stopPump();
      } else {
        syncBounds(); // re-align immediately on return
        startPump();
      }
    };
    if (!document.hidden) startPump();
    document.addEventListener('visibilitychange', onVisibility);

    window.addEventListener('resize', syncBounds);

    return () => {
      observer.disconnect();
      stopPump();
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('resize', syncBounds);
    };
  }, [api, syncBounds]);

  // ── Re-sync whenever active tab changes ──
  useEffect(() => {
    syncBounds();
  }, [activeTabId, tabs, syncBounds]);

  // ── Re-sync when the chat launcher appears/disappears/resizes (#553) ──
  // Event-driven, not polled — composes with the nap-safe pump suspension.
  useEffect(() => {
    syncBounds();
  }, [chatLauncherSize, chatDockOpen, syncBounds]);

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

    // Listen for new-window requests (target=_blank, window.open) from child webviews
    let unlistenNewWindow: (() => void) | null = null;
    api.listen('browser_new_window_request', (e) => {
      const payload = e.payload as { source_webview_id: string; url: string };
      handleOpenUrl(payload.url);
    }).then((fn) => {
      unlistenNewWindow = fn;
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
      unlistenNewWindow?.();
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

  // ── Open a URL pushed from elsewhere (chat-link click, agent tour #353) ──
  // Waits for the Tauri API to be ready, opens it in a new tab, then clears the
  // signal so it fires once.
  useEffect(() => {
    if (!pendingBrowserUrl || !api) return;
    let cancelled = false;
    (async () => {
      await handleOpenUrl(pendingBrowserUrl);
      if (!cancelled) clearPendingBrowserUrl();
    })();
    return () => {
      cancelled = true;
    };
  }, [pendingBrowserUrl, api, handleOpenUrl, clearPendingBrowserUrl]);

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
    <div className="flex h-full flex-col" style={{ backgroundColor: colors.bg }}>
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
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: colors.surface, borderBottom: `1px solid ${colors.border}` }}>
        {/* Nav buttons */}
        <div className="flex items-center gap-1">
          <button
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            title="Back (Cmd+[)"
            disabled
          >
            <FiArrowLeft size={14} />
          </button>
          <button
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            title="Forward (Cmd+])"
            disabled
          >
            <FiArrowRight size={14} />
          </button>
          <button
            onClick={handleReload}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            title="Reload (Cmd+R)"
            disabled={!activeTab?.webviewId}
          >
            <FiRefreshCw size={14} className={activeTab?.loading ? 'animate-spin' : ''} />
          </button>
        </div>

        {/* Address bar */}
        <div
          className="flex-1 flex items-center rounded-md transition-colors"
          style={{ backgroundColor: colors.bgDeeper, border: `1px solid ${colors.border}` }}
          onFocus={e => { e.currentTarget.style.borderColor = colors.cyan; }}
          onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
        >
          <span className="pl-2.5 pr-1">
            {protocol === 'https' ? (
              <FiLock size={12} style={{ color: colors.cyan }} />
            ) : protocol === 'http' ? (
              <FiAlertTriangle size={12} className="text-amber-400" />
            ) : (
              <FiShield size={12} style={{ color: colors.textMuted }} />
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
            className="browser-url-input flex-1 bg-transparent text-xs py-1.5 pr-3 outline-none"
            style={{ fontFamily: font.mono, color: colors.text }}
          />
          <style>{`.browser-url-input::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
        </div>
      </div>

      {/* Content area — the child webview overlays this div */}
      <div ref={containerRef} className="flex-1 min-h-0 relative">
        {!api ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center p-8">
              <FiShield size={48} className="mx-auto mb-4" style={{ color: colors.textMuted }} />
              <h3 className="text-lg mb-2" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Desktop App Required</h3>
              <p className="text-xs max-w-md" style={{ fontFamily: font.body, color: colors.textMuted }}>
                The embedded browser requires the Permagent desktop app. Run{' '}
                <code className="px-1.5 py-0.5 rounded" style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.codeText }}>
                  npm run tauri:dev
                </code>{' '}
                to use this feature.
              </p>
            </div>
          </div>
        ) : !activeTab?.webviewId ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center p-8">
              <FiGlobe size={48} className="mx-auto mb-4" style={{ color: colors.cyan, opacity: 0.3 }} />
              <h3 className="text-sm mb-2" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Ready to Browse</h3>
              <p className="text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>
                Enter a URL above or press{' '}
                <kbd className="px-1.5 py-0.5 rounded text-[10px]" style={{ fontFamily: font.mono, backgroundColor: colors.border }}>Cmd+L</kbd> to
                focus the address bar
              </p>
              <div className="mt-6 flex flex-wrap gap-2 justify-center">
                {['google.com', 'github.com', 'accounts.google.com'].map((site) => (
                  <button
                    key={site}
                    onClick={() => handleNavigate(`https://${site}`)}
                    className="px-3 py-1.5 rounded-md bg-white/5 text-xs transition-colors"
                    style={{ fontFamily: font.body, color: colors.textMuted }}
                    onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
                    onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; e.currentTarget.style.backgroundColor = ''; }}
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
      <div className="flex items-center gap-3 px-3 py-1" style={{ backgroundColor: colors.surface, borderTop: `1px solid ${colors.border}` }}>
        <span className="text-[10px] flex-1 truncate" style={{ fontFamily: font.mono, color: colors.textMuted }}>
          {activeTab?.loading
            ? 'Loading...'
            : activeTab?.webviewId
              ? activeTab.url
              : 'Ready'}
        </span>
        {activeTab?.webviewId && (
          <span className="flex items-center gap-1 text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>
            <button
              onClick={() => handleZoom(-0.1)}
              className="transition-colors px-0.5"
              onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; }}
              onMouseLeave={e => { e.currentTarget.style.color = ''; }}
            >−</button>
            <span className="w-8 text-center">{Math.round(zoomLevel * 100)}%</span>
            <button
              onClick={() => handleZoom(0.1)}
              className="transition-colors px-0.5"
              onMouseEnter={e => { e.currentTarget.style.color = colors.cyan; }}
              onMouseLeave={e => { e.currentTarget.style.color = ''; }}
            >+</button>
          </span>
        )}
        <span className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>
          {tabs.length} tab{tabs.length !== 1 ? 's' : ''}
        </span>
      </div>
    </div>
  );
}
