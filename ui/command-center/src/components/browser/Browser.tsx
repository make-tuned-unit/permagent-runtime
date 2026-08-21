import { useState, useCallback, useRef, useEffect, useImperativeHandle, forwardRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useBrowserContentBridge } from '../../hooks/useBrowserContentBridge';
import { useBrowserActBridge } from '../../hooks/useBrowserActBridge';
import {
  FiRefreshCw,
  FiChevronLeft,
  FiChevronRight,
  FiLock,
  FiAlertTriangle,
  FiShield,
  FiGlobe,
  FiInbox,
} from 'react-icons/fi';
import { BrowserTabs, type BrowserTab } from './BrowserTabs';
import { BookmarksBar } from './BookmarksBar';
import {
  applyEvent,
  bufferEvent,
  extractTitle,
  isPlaceholderUrl,
  popupTabDecision,
  replayEvents,
  type PendingEvent,
} from './tabIdentity';
import { CHAT_LAUNCHER_MARGIN } from '../chat/ChatLauncher';
import { nextPaneTabId, usePaneTabCycling } from '../build/paneTabCycling';
import { createBoundsPump } from './boundsPump';
import { reserveFromLeft } from './reservedRect';

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

interface BrowserProps { initialTab?: BrowserTab | null; ownerWindowLabel?: string; detached?: boolean }

export const Browser = forwardRef<{ getActiveTab: () => BrowserTab }, BrowserProps>(function Browser({ initialTab, ownerWindowLabel = 'main', detached = false }, ref) {
  const { colors } = useTheme();
  const overlayBlocking = useCommandCenter(s => s.overlayBlockingBrowser);
  const chatLauncherSize = useCommandCenter(s => s.chatLauncherSize);
  const sidebarTooltipRect = useCommandCenter(s => s.sidebarTooltipRect);
  const chatDockOpen = useCommandCenter(s => s.chatDockOpen);
  const pendingBrowserUrl = useCommandCenter(s => s.pendingBrowserUrl);
  const clearPendingBrowserUrl = useCommandCenter(s => s.clearPendingBrowserUrl);

  const [tabs, setTabs] = useState<BrowserTab[]>(() => {
    if (initialTab) return [initialTab];
    if (!detached && persistedTabs) return persistedTabs;
    return [createTab()];
  });
  const [activeTabId, setActiveTabId] = useState<string>(() => {
    return initialTab?.id || (!detached ? persistedActiveTabId : null) || tabs[0].id;
  });
  const [closingTabId, setClosingTabId] = useState<string | null>(null);
  const [urlInput, setUrlInput] = useState('');
  const [api, setApi] = useState<TauriApi | null>(null);
  const [zoomLevel, setZoomLevel] = useState(1.0);
  const urlInputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

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
  // Stable handle to syncBounds for effects registered before its definition
  // (the pane_redock listener needs to snap bounds on arrival).
  const syncBoundsRef = useRef<(() => void) | null>(null);

  // The rect we last actually handed to `update_browser_bounds`. The suspended
  // pump's drift probe compares the container against this: if they disagree,
  // the native surface is somewhere the container is not — which is the
  // stranded-after-restore symptom, stated as a checkable fact.
  const lastAppliedBoundsRef = useRef<{ x: number; y: number; width: number; height: number } | null>(null);
  // Events that arrived for a webview React does not know about yet.
  // `create_browser_webview` starts loading the page before it returns, so the
  // first `browser_page_load` can beat the `setTabs` that records the
  // webviewId — and a handler that `map`s over tabs matches nothing, drops the
  // update, and never sees it again because these events are not replayed.
  // The tab then sits on "New Tab" forever, which also blinds the agent: it
  // picks tabs by label (reported 2026-08-04). See tabIdentity.ts.
  const pendingEventsRef = useRef<Map<string, PendingEvent[]>>(new Map());

  /** Apply an event to the tab owning `webviewId`, or buffer it if no tab has
   *  claimed that id yet. The single entry point for every identity update. */
  const ingest = useCallback((webviewId: string, ev: PendingEvent) => {
    setTabs((prev) => {
      if (!prev.some((t) => t.webviewId === webviewId)) {
        bufferEvent(pendingEventsRef.current, webviewId, ev);
        return prev;
      }
      let changed = false;
      const next = prev.map((t) => {
        if (t.webviewId !== webviewId) return t;
        const updated = applyEvent(t, ev);
        if (updated !== t) changed = true;
        return updated;
      });
      return changed ? next : prev;
    });
  }, []);

  // Reap browser webviews the shell has forgotten (#548).
  //
  // BrowserSessions and the native children live for the PROCESS; this
  // component's memory of their ids dies with the page. After a shell reload
  // (or a dev hot-reload) every previously-open child keeps compositing above
  // the DOM — native webviews always render over HTML — while nothing left
  // running can address them. That is the "force-quit required" state.
  //
  // Must wait for the Tauri API. The first version of this effect ran on
  // mount, read `apiRef.current` (still null — `getTauriApi` is async), set
  // `reapedRef`, and returned. A Cmd+R then left the old page painted over
  // the new shell with no close button that could reach it (2026-08-21).
  // Waiting on `api` retries until invoke is real; `reapedRef` still
  // guarantees one sweep. Right after a reload `keep` is empty, so every
  // native child of THIS window is orphaned and gets closed.
  const reapedRef = useRef(false);
  useEffect(() => {
    if (reapedRef.current || !api) return;
    reapedRef.current = true;
    const keep = tabsRef.current.map(t => t.webviewId).filter((id): id is string => !!id);
    api.invoke('reap_orphan_browsers', { keep, ownerWindow: ownerWindowLabel })
      .catch(() => { /* older shell without the command — nothing to reap */ });
  }, [api, ownerWindowLabel]);

  // Replay buffered events onto tabs that have since registered their
  // webviewId. Runs after every tabs change, so whichever of the two orderings
  // happened, the tab ends up correct — and in event order, so a title that
  // followed a page load still wins.
  useEffect(() => {
    if (pendingEventsRef.current.size === 0) return;
    let changed = false;
    const next = tabs.map((t) => {
      if (!t.webviewId) return t;
      const held = pendingEventsRef.current.get(t.webviewId);
      if (!held) return t;
      pendingEventsRef.current.delete(t.webviewId);
      const updated = replayEvents(t, held);
      if (updated !== t) changed = true;
      return updated;
    });
    if (changed) setTabs(next);
  }, [tabs]);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  // ── The address bar mirrors the ACTIVE TAB'S URL. One source of truth ──
  //
  // It used to be independent state written from a dozen call sites, one of
  // which was the raw navigation event — so the bar could disagree with the
  // tab it belongs to and nothing would ever bring them back together. CBC
  // showed tab `cbc.ca` and status bar `cbc.ca/` while the address bar read
  // `google.com/recaptcha/api2/aframe` (reported 2026-08-04). Now the bar is
  // derived; the ONLY thing that can hold it away from the tab's URL is the
  // user actively typing in it.
  // A half-typed URL survives the page navigating under it, but NOT a switch to
  // a different tab — that is a different address, so the edit is abandoned.
  // Centralising the reset here means every path that changes the active tab
  // (select, cycle, close, new, pop-out, redock, an agent opening a link) gets
  // it without having to remember to.
  const urlDirtyRef = useRef(false);
  const mirroredTabRef = useRef<string | null>(null);
  useEffect(() => {
    const nextId = activeTab?.id ?? null;
    if (mirroredTabRef.current !== nextId) {
      mirroredTabRef.current = nextId;
      urlDirtyRef.current = false;
    }
    if (urlDirtyRef.current) return;
    setUrlInput(activeTab?.url ?? '');
  }, [activeTab?.url, activeTab?.id]);

  // Back/forward availability, straight from WKWebView. The buttons were
  // removed in the 2026-07 audit for being permanently disabled with no
  // handler; they only come back with the REAL history stack behind them, so
  // this is never inferred from a URL list — a redirect or a fragment change
  // would desync that immediately.
  const [navState, setNavState] = useState<{ canGoBack: boolean; canGoForward: boolean }>({
    canGoBack: false,
    canGoForward: false,
  });

  /** Re-read the webview's real main-frame URL. WebKit fires no navigation
   *  callback for a same-document change (`pushState`, hash, SPA route), so a
   *  signal that the page probably moved — a title change, or a tab rejoining
   *  the event stream — pulls the truth instead. Not a poll: event-driven. */
  const resyncUrl = useCallback((webviewId: string) => {
    const inv = apiRef.current;
    if (!inv) return;
    inv.invoke('browser_nav_state', { webviewId })
      .then((s) => {
        const st = s as { url?: string; canGoBack?: boolean; canGoForward?: boolean };
        if (st?.url) ingest(webviewId, { kind: 'url', url: st.url });
        if (webviewId === tabsRef.current.find(t => t.id === activeTabIdRef.current)?.webviewId) {
          setNavState({ canGoBack: !!st?.canGoBack, canGoForward: !!st?.canGoForward });
        }
      })
      .catch(() => { /* webview gone — the tab close path handles it */ });
  }, [ingest]);

  // Bridge: daemon MCP tool → Tauri webview content extraction → daemon fulfillment
  useBrowserContentBridge(activeTab?.webviewId ?? null);
  // Bridge: daemon MCP tool → Tauri webview snapshot/act (#649) → daemon fulfillment
  // Owned webviews (all tabs), not just the active one: the act event fans out
  // to every client, so ownership decides who performs it (#939).
  useBrowserActBridge(
    activeTab?.webviewId ?? null,
    tabs.map((t) => t.webviewId),
  );

  // Initialize Tauri API
  useEffect(() => {
    getTauriApi().then((resolved) => {
      setApi(resolved);
    });
  }, []);

  // ── Persist CONTINUOUSLY, not on unmount ──
  //
  // Persisting in the unmount cleanup looks right and is a race React always
  // wins. Toggling the terminal changes the PanelGroup's `key` in BuildView,
  // which remounts this component — and React runs the RENDER phase before the
  // COMMIT phase. The incoming instance's `useState` initializer (which reads
  // `persistedTabs`) therefore executes BEFORE the outgoing instance's cleanup
  // writes it. The new Browser read stale-or-null state, called `createTab()`,
  // and rendered an empty "New Tab" — while the old native webview, which is
  // Rust-side and outlives React, kept compositing on top. That is the
  // reported "browser lost the page": a fresh tab with an orphaned surface
  // over it (screenshot 2026-08-04 — tab "New Tab", url bar empty,
  // `about:blank` in the status bar, Google still painted).
  //
  // Writing on every change means the value is always current before any
  // remount, whatever order React chooses.
  useEffect(() => {
    if (!detached) {
      persistedTabs = tabs;
      persistedActiveTabId = activeTabId;
    }
  }, [tabs, activeTabId, detached]);

  // ── Park webviews offscreen on unmount (workspace switch) ──
  useEffect(() => {
    return () => {
      // Move all child webviews offscreen
      const inv = apiRef.current;
      tabsRef.current.forEach((t) => {
        if (t.webviewId && inv) {
          inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
        }
      });
    };
  }, [detached]);

  useImperativeHandle(ref, () => ({
    getActiveTab: () => tabsRef.current.find(t => t.id === activeTabIdRef.current) || tabsRef.current[0],
    // Every tab this browser owns — a detached pane window needs all of them
    // to destroy their child webviews when the window is genuinely closed.
    getAllTabs: () => tabsRef.current,
  }), []);

  useEffect(() => {
    if (detached || !('__TAURI_INTERNALS__' in window)) return;
    let unlisten: (() => void) | undefined;
    import('@tauri-apps/api/event').then(({ listen }) => listen<{ kind: string; tab: BrowserTab }>('pane_redock', e => {
      if (e.payload.kind !== 'browser') return;
      setTabs(prev => [...prev.filter(t => t.id !== e.payload.tab.id), e.payload.tab]);
      setActiveTabId(e.payload.tab.id);
      // The tab travelled as a snapshot; it may have navigated since. Pull the
      // webview's real URL now that it is back on this window's event stream.
      if (e.payload.tab.webviewId) resyncUrl(e.payload.tab.webviewId);
      // Place the incoming webview NOW — it arrives parked offscreen (the pane
      // hid it before reparenting) and waiting up to 500ms for the pump left a
      // visible gap (or, before the hide, a flash over the terminal panel).
      requestAnimationFrame(() => syncBoundsRef.current?.());
    })).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [detached, resyncUrl]);

  // ── Sync active webview position with the container div ──
  const syncBounds = useCallback(() => {
    const inv = apiRef.current;
    if (!containerRef.current || !inv) return;

    // A transient DOM overlay is up (record-meeting picker, modals): the
    // native webview always composites above DOM, so the only correct bounds
    // are offscreen. This must live HERE, not only in the overlay effect —
    // the 500 ms pump and the ResizeObserver both call syncBounds, and
    // without this check they snapped the webview back over the overlay
    // within a tick of the hide (reported live 2026-08-06).
    if (useCommandCenter.getState().overlayBlockingBrowser > 0) {
      lastAppliedBoundsRef.current = null;
      tabsRef.current.forEach((t) => {
        if (t.webviewId) inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
      });
      return;
    }

    const rect = containerRef.current.getBoundingClientRect();

    const currentTabs = tabsRef.current;
    const currentActiveId = activeTabIdRef.current;

    // Hide webviews when container is hidden (workspace switch)
    if (rect.width === 0 || rect.height === 0) {
      lastAppliedBoundsRef.current = null;
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
    // Chat dock: on wide screens the dock is a flex sibling of <main>, so the
    // container rect ALREADY excludes it — subtracting again would shrink the
    // webview by a second dock width. Only the narrow (<640) full-width sheet
    // still overlays, and it covers the pane entirely, so reserve nothing here.
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

    // Sidebar hover label: same class of problem as the launcher corner, and
    // the same remedy. The tooltip is drawn to the RIGHT of the rail, into
    // this pane; a native child surface composites above the shell's DOM, so
    // the only way it stays visible is for the browser's rect not to reach it.
    // Reported 2026-08-19 as "browser full view + terminal toggled off",
    // which is precisely when this pane spans the full width — with the
    // terminal showing, BuildView's horizontal split puts the browser on the
    // right half and the tooltip lands over the terminal's DOM instead.
    // `reserveFromLeft` returns the input untouched when there is no overlap,
    // so every other layout keeps the bounds it has today.
    const reserved = reserveFromLeft(
      { x: rect.x, y: rect.y, width, height },
      useCommandCenter.getState().sidebarTooltipRect,
    );

    currentTabs.forEach((t) => {
      if (!t.webviewId) return;
      if (t.id === currentActiveId) {
        lastAppliedBoundsRef.current = { ...reserved };
        inv.invoke('update_browser_bounds', {
          webviewId: t.webviewId,
          x: reserved.x,
          y: reserved.y,
          width: reserved.width,
          height: reserved.height,
        }).catch(() => {});
      } else {
        inv.invoke('hide_browser', { webviewId: t.webviewId }).catch(() => {});
      }
    });
  }, []);
  syncBoundsRef.current = syncBounds;

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

    // Restore any persisted webviews on mount.
    //
    // Retried, not fired once. Toggling the terminal changes the PanelGroup's
    // `key` in BuildView, which REMOUNTS this component — and the outgoing
    // instance's cleanup fires `hide_browser` (a move to -10000,-10000) for
    // every tab without awaiting it. Those invokes can land AFTER the incoming
    // instance's `update_browser_bounds`, which parks the webview offscreen
    // and reads to the user as "the browser lost the page" (reported
    // 2026-08-04 on exactly this sequence). The page is fine; the surface is
    // just parked.
    //
    // A few cheap re-syncs cover the ordering window regardless of which side
    // wins the race. They are idempotent — syncBounds only ever sets position
    // and size — so an unnecessary one costs nothing.
    syncBounds();
    const settleTimers = [50, 150, 400].map(ms => setTimeout(syncBounds, ms));

    const observer = new ResizeObserver(() => syncBounds());
    observer.observe(containerRef.current);

    // The 500 ms poll exists because ResizeObserver doesn't fire on display:none
    // changes (workspace switches). It suspends while the surface is off screen
    // (the #562 C1 nap-safety property), but WHO gets to say "off screen" is no
    // longer the Page Visibility API alone — see boundsPump.ts. Reported
    // 2026-08-19: minimised for ten minutes, and on restore the browser was
    // still painting at the coordinates it had before, because the
    // `visibilitychange` that says "back" never arrived.
    const pump = createBoundsPump({
      sync: syncBounds,
      // The last-resort drift check, run only while suspended. It performs NO
      // native op — it compares the container's rect against the bounds we last
      // actually applied — so it cannot reintroduce the idle wedge.
      probe: () => {
        const el = containerRef.current;
        const last = lastAppliedBoundsRef.current;
        if (!el || !last) return false;
        const rect = el.getBoundingClientRect();
        if (rect.width === 0 || rect.height === 0) return false;
        return (
          Math.abs(rect.x - last.x) > 0.5 ||
          Math.abs(rect.y - last.y) > 0.5 ||
          Math.abs(rect.width - last.width) > 0.5 ||
          Math.abs(rect.height - last.height) > 0.5
        );
      },
    });
    pump.signal(document.hidden ? 'page-hidden' : 'page-visible');

    const onVisibility = () => {
      pump.signal(document.hidden ? 'page-hidden' : 'page-visible');
    };
    document.addEventListener('visibilitychange', onVisibility);

    // NATIVE window events. These are the signals that actually survive a
    // macOS minimise/restore, and the window — not the page — is asked whether
    // it is miniaturised, because `document.hidden` is precisely the value that
    // cannot be trusted here. Asking also stops the `Resized` macOS emits ON
    // minimise from restarting the pump behind a miniaturised window.
    let disposed = false;
    const windowUnlisteners: Array<() => void> = [];
    (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();
        const onWindowSignal = async () => {
          if (disposed) return;
          let minimized = false;
          try {
            minimized = await win.isMinimized();
          } catch {
            minimized = false;
          }
          if (disposed) return;
          pump.signal(minimized ? 'window-occluded' : 'window-active');
        };
        const handlers = await Promise.all([
          win.onFocusChanged(({ payload: focused }) => {
            if (focused) void onWindowSignal();
          }),
          win.onResized(() => void onWindowSignal()),
          win.onMoved(() => void onWindowSignal()),
        ]);
        if (disposed) {
          handlers.forEach(fn => fn());
          return;
        }
        windowUnlisteners.push(...handlers);
      } catch {
        // Not running under Tauri (unit tests, a browser preview): the Page
        // Visibility path above is all there is, which is where we started.
      }
    })();

    // DOM focus as well. Cheap, and it lands in cases where the native event is
    // late — it can only ever cause an extra idempotent re-align.
    const onWindowFocus = () => pump.signal('window-active');
    window.addEventListener('focus', onWindowFocus);
    window.addEventListener('resize', syncBounds);

    return () => {
      disposed = true;
      settleTimers.forEach(clearTimeout);
      observer.disconnect();
      pump.dispose();
      windowUnlisteners.forEach(fn => fn());
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('focus', onWindowFocus);
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
  }, [chatLauncherSize, chatDockOpen, sidebarTooltipRect, syncBounds]);

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

  // Stable open-URL handle for Tauri event listeners registered in an effect
  // that must NOT re-subscribe every time handleOpenUrl's identity changes.
  // Calling a stale closure after a remount is how a popup click can create a
  // native webview whose setTabs never lands in the live tab strip — the click
  // then looks like it did nothing.
  const openUrlRef = useRef<(url: string, label?: string) => Promise<void>>(async () => {});

  // Last popup this instance turned into a tab. Guards the belt-and-braces
  // path: the injected interceptor cancels the default before re-expressing a
  // gesture as window.open, so WebKit's own new-frame path should never also
  // fire — but one click must be one tab even if it does.
  const lastPopupRef = useRef<{ url: string; at: number } | null>(null);

  // Listen for navigation events from Tauri
  useEffect(() => {
    if (!api) return;

    let unlisten: (() => void) | null = null;

    // MAIN-FRAME page loads. Emitted from Rust's `on_page_load`, which wraps
    // WebKit's didCommit/didFinish — main-frame-only callbacks carrying the
    // webview's own URL. Its predecessor (`on_navigation`) fired for every
    // frame, so an ad iframe could rename the tab and rewrite the address bar.
    api.listen('browser_page_load', (e) => {
      const payload = e.payload as { webview_id: string; url: string; loading: boolean };
      if (payload.loading) {
        const navTab = tabsRef.current.find((t) => t.webviewId === payload.webview_id);
        emitActivity(api, 'browser_navigated', {
          url: payload.url,
          title: extractTitle(payload.url),
          referrer: navTab?.url || '',
          tab_id: navTab?.id || payload.webview_id,
        });
      }
      ingest(payload.webview_id, { kind: 'load', url: payload.url, loading: payload.loading });
    }).then((fn) => {
      unlisten = fn;
    });

    // Listen for page title changes from the native webview.
    let unlistenTitle: (() => void) | null = null;
    api.listen('browser_title_changed', (e) => {
      const payload = e.payload as { webview_id: string; title: string };
      ingest(payload.webview_id, { kind: 'title', title: payload.title });
      // A title change with no page load behind it means a same-document
      // navigation — an SPA route, `pushState`, a hash. WebKit has no callback
      // for those, so this is the moment to pull the real URL.
      resyncUrl(payload.webview_id);
    }).then((fn) => {
      unlistenTitle = fn;
    });

    // Every "open in a new tab" gesture lands here, and this is the ONLY place
    // that turns one into a tab. Two sources feed it, both via browser.rs's
    // `on_new_window`: WebKit's own new-frame path (window.open, target=_blank)
    // and browser_links.js, which re-expresses the mouse gestures WebKit will
    // not route there (right-click menu, middle-click, Cmd-click) as a
    // window.open. Without this listener every one of them is a silent no-op —
    // WKWebView returns nil from createWebViewWithConfiguration and the click
    // just does nothing (#240 / #709 / #973).
    //
    // A DECLINE IS LOGGED WITH ITS REASON. That is the whole point: those three
    // regressions each survived weeks because a dropped popup left no trace.
    let unlistenNewWindow: (() => void) | null = null;
    api.listen('browser_new_window_request', (e) => {
      const payload = e.payload as { source_webview_id: string; url: string };
      const owned = tabsRef.current.map((t) => t.webviewId);
      const decision = popupTabDecision(
        owned,
        payload.source_webview_id,
        payload.url,
        lastPopupRef.current,
      );
      if (!decision.open) {
        console.warn(
          `[browser] new-tab request DROPPED (${decision.reason}):`,
          payload.url,
          'from',
          payload.source_webview_id,
          'owned by this Browser:',
          owned,
        );
        return;
      }
      lastPopupRef.current = { url: payload.url, at: Date.now() };
      console.info('[browser] new-tab request accepted:', payload.url, 'from', payload.source_webview_id);
      void openUrlRef.current(payload.url);
    }).then((fn) => {
      unlistenNewWindow = fn;
    });

    // Listen for OAuth URL open events
    let unlistenOAuth: (() => void) | null = null;
    api.listen('browser_open_url', (e) => {
      const payload = e.payload as { url: string; oauth?: boolean; provider?: string };
      void openUrlRef.current(
        payload.url,
        payload.oauth ? `OAuth: ${payload.provider || ''}` : undefined,
      );
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
  }, [api, ingest, resyncUrl]);

  const handleOpenUrl = useCallback(
    async (url: string, label?: string) => {
      if (!apiRef.current) return;
      if (isPlaceholderUrl(url)) return;

      const rect = containerRef.current?.getBoundingClientRect();
      const tab = createTab();
      tab.url = url;
      tab.label = label || extractTitle(url);
      tab.loading = true;

      try {
        const webviewId = (await apiRef.current.invoke('create_browser_webview', {
          url,
          windowLabel: ownerWindowLabel,
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
    },
    [ownerWindowLabel],
  );
  openUrlRef.current = handleOpenUrl;

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

      // The edit is committed — hand the address bar back to the active tab,
      // and show the normalized form now rather than after the round trip.
      urlDirtyRef.current = false;
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
            windowLabel: ownerWindowLabel,
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
    [tabs, activeTabId, ownerWindowLabel],
  );

  const handleNewTab = useCallback(() => {
    const tab = createTab();
    setTabs((prev) => [...prev, tab]);
    setActiveTabId(tab.id);
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
            return [newTab];
          }
          if (tabId === activeTabIdRef.current) {
            const idx = prevTabs.findIndex((t) => t.id === tabId);
            const nextActive = next[Math.min(idx, next.length - 1)];
            setActiveTabId(nextActive.id);
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
      // The address bar and syncBounds both follow from activeTabId.
    },
    [],
  );

  const cycleTabs = useCallback((backwards = false) => {
    const nextId = nextPaneTabId(
      tabsRef.current.map(tab => tab.id),
      activeTabIdRef.current,
      backwards,
    );
    if (!nextId) return;
    setActiveTabId(nextId);
  }, []);
  const selectPane = usePaneTabCycling('browser', rootRef, cycleTabs);

  // Any page load or tab switch can change what history holds.
  useEffect(() => {
    const id = activeTab?.webviewId;
    if (!id) { setNavState({ canGoBack: false, canGoForward: false }); return; }
    resyncUrl(id);
  }, [activeTab?.webviewId, activeTab?.url, activeTab?.loading, resyncUrl]);

  const goHistory = useCallback((forward: boolean) => {
    const id = tabsRef.current.find(t => t.id === activeTabIdRef.current)?.webviewId;
    if (!id || !apiRef.current) return;
    apiRef.current.invoke('browser_go', { webviewId: id, forward })
      .then(() => {
        // WKWebView commits asynchronously; the page_load event will correct
        // the URL, this just stops the buttons lagging a beat behind.
        setTimeout(() => resyncUrl(id), 120);
      })
      .catch(() => {});
  }, [resyncUrl]);

  // Save the open tab into the Downloads inbox (#392/#393, and the 2026-08-19
  // report that nothing downloaded ever arrived there).
  //
  // This is the human-facing half of the capture path. WebKit renders a PDF or
  // a Word document instead of downloading it — `canShowMIMEType` is true for
  // both — so there is no download event to hook and no amount of fixing
  // `on_download` produces one. The only way such a file reaches the inbox is
  // for someone to ask for it, so: a button.
  const [savingToInbox, setSavingToInbox] = useState<null | 'busy' | 'done' | 'failed'>(null);
  const saveToInbox = useCallback(async () => {
    const inv = apiRef.current;
    const tab = tabsRef.current.find(t => t.id === activeTabIdRef.current);
    if (!inv || !tab?.webviewId) return;
    setSavingToInbox('busy');
    try {
      const capture = await inv.invoke('save_tab_to_inbox', {
        webviewId: tab.webviewId,
        projectId: null,
        expectDocument: false,
      }) as { filename: string };
      console.info('[permagent] browser: saved to inbox:', capture.filename);
      setSavingToInbox('done');
    } catch (err) {
      console.error('[permagent] browser: save to inbox failed:', err);
      setSavingToInbox('failed');
    }
    setTimeout(() => setSavingToInbox(null), 2500);
  }, []);

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
      // Abandon the edit; the mirror effect puts the tab's URL back.
      urlDirtyRef.current = false;
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

  const popOutActive = useCallback(async () => {
    const tab = tabsRef.current.find(t => t.id === activeTabIdRef.current);
    if (!tab || detached) return;
    try {
      const { createPaneWindow } = await import('../../lib/paneWindows');
      const label = await createPaneWindow('browser', tab);
      if (tab.webviewId && apiRef.current) {
        await apiRef.current.invoke('reparent_browser', { webviewId: tab.webviewId, windowLabel: label });
      }
      setTabs(prev => {
        const next = prev.filter(t => t.id !== tab.id);
        if (next.length) { setActiveTabId(next[0].id); return next; }
        const replacement = createTab();
        setActiveTabId(replacement.id);
        return [replacement];
      });
    } catch (err) { console.error('[browser] pop-out failed:', err); }
  }, [detached]);

  return (
    <div ref={rootRef} onFocusCapture={selectPane} className="flex h-full flex-col" style={{ backgroundColor: colors.bg }}>
      {/* Tab bar */}
      <BrowserTabs
        tabs={tabs}
        activeTabId={activeTabId}
        closingTabId={closingTabId}
        onSelectTab={handleSelectTab}
        onCloseTab={handleCloseTab}
        onNewTab={handleNewTab}
        onCycleTab={() => cycleTabs()}
        onPopOut={detached ? undefined : popOutActive}
      />

      {/* URL bar */}
      <div className="flex items-center gap-2 px-3 py-2" style={{ backgroundColor: colors.surface, borderBottom: `1px solid ${colors.border}` }}>
        {/* Back/Forward act on WKWebView's own history via `browser_go`, and
            their enabled state comes from canGoBack/canGoForward rather than a
            URL list we keep ourselves — that is the "real history" the 2026-07
            audit was waiting for before restoring these. */}
        <div className="flex items-center gap-1">
          <button
            onClick={() => goHistory(false)}
            className="p-1.5 rounded hover:bg-white/5 transition-colors disabled:opacity-30 disabled:hover:bg-transparent"
            style={{ color: colors.textMuted }}
            title="Back"
            aria-label="Back"
            disabled={!navState.canGoBack}
          >
            <FiChevronLeft size={16} />
          </button>
          <button
            onClick={() => goHistory(true)}
            className="p-1.5 rounded hover:bg-white/5 transition-colors disabled:opacity-30 disabled:hover:bg-transparent"
            style={{ color: colors.textMuted }}
            title="Forward"
            aria-label="Forward"
            disabled={!navState.canGoForward}
          >
            <FiChevronRight size={16} />
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
          <button
            onClick={saveToInbox}
            className="p-1.5 rounded hover:bg-white/5 transition-colors disabled:opacity-30 disabled:hover:bg-transparent"
            style={{
              color:
                savingToInbox === 'done'
                  ? colors.cyan
                  : savingToInbox === 'failed'
                    ? colors.danger
                    : colors.textMuted,
            }}
            title={
              savingToInbox === 'done'
                ? 'Saved to your inbox'
                : savingToInbox === 'failed'
                  ? 'Could not save this tab — see the console for why'
                  : 'Save this page or document to your Downloads inbox'
            }
            aria-label="Save to inbox"
            disabled={!activeTab?.webviewId || savingToInbox === 'busy'}
          >
            <FiInbox size={14} />
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
            onChange={(e) => { urlDirtyRef.current = true; setUrlInput(e.target.value); }}
            onKeyDown={handleUrlKeyDown}
            onFocus={(e) => e.target.select()}
            placeholder="Search or enter URL..."
            className="browser-url-input flex-1 bg-transparent text-xs py-1.5 pr-3 outline-none"
            style={{ fontFamily: font.mono, color: colors.text }}
          />
          <style>{`.browser-url-input::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
        </div>
      </div>

      {/* Bookmarks + saved tab sets row (#790) — daemon-persisted state */}
      <BookmarksBar
        currentUrl={activeTab?.url ?? ''}
        currentTitle={activeTab?.label ?? ''}
        openTabs={tabs}
        onNavigate={handleNavigate}
        onOpenInNewTab={handleOpenUrl}
      />

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
});
