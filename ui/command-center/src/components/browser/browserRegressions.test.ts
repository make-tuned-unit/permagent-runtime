/**
 * Regression tests for the browser/pane bugs fixed on 2026-08-04.
 *
 * Each of these shipped as a "reasoned" fix that turned out to be wrong at
 * least once, so the point here is to pin the ORDERING each one depends on —
 * that is what actually broke, and it is invisible in the component's shape.
 * The logic is extracted as pure functions rather than mounting the whole
 * Browser (which needs a Tauri bridge, a ResizeObserver and a live webview);
 * mounting would test React, not the invariant.
 */
import { describe, it, expect } from 'vitest';
import {
  applyEvent,
  bufferEvent,
  extractTitle,
  isPlaceholderUrl,
  pageLoadUpdate,
  replayEvents,
  shouldOpenPopupTab,
  titleUpdate,
  urlOnlyUpdate,
  MAX_PENDING_EVENTS,
  type PendingEvent,
} from './tabIdentity';
import type { BrowserTab } from './BrowserTabs';

// ── 1. Tab-state survives a remount ─────────────────────────────────────────
//
// BuildView keys its PanelGroup on the pane-visibility flags, so toggling the
// terminal REMOUNTS <Browser>. React runs the render phase before the commit
// phase, so the incoming instance's useState initializer reads the module-level
// cache BEFORE the outgoing instance's cleanup writes it. Persisting on unmount
// therefore always lost a toggle; persisting on change cannot.

type Tab = BrowserTab;

/** Models the module-level cache Browser.tsx keeps. */
function makeCache() {
  let persisted: Tab[] | null = null;
  return {
    /** What the component writes. */
    persist: (tabs: Tab[]) => { persisted = tabs; },
    /** What a fresh mount's useState initializer reads. */
    restore: (): Tab[] => persisted ?? [{ id: 'fresh', label: 'New Tab', webviewId: null, url: '', loading: false }],
  };
}

const LIVE: Tab[] = [{ id: 't1', label: 'ogs.google.com', webviewId: 'browser-0', url: 'https://ogs.google.com', loading: false }];

describe('tab state across a keyed remount', () => {
  it('is LOST when persisted on unmount — render runs before cleanup', () => {
    const cache = makeCache();
    // React order for a keyed swap: new instance renders…
    const restored = cache.restore();
    // …then the old instance's cleanup finally writes.
    cache.persist(LIVE);
    expect(restored[0].label).toBe('New Tab'); // the bug in the reported screenshot
    expect(restored[0].webviewId).toBeNull();
  });

  it('survives when persisted on every change', () => {
    const cache = makeCache();
    cache.persist(LIVE);          // written while alive, not during teardown
    const restored = cache.restore();
    expect(restored[0].label).toBe('ogs.google.com');
    expect(restored[0].webviewId).toBe('browser-0');
  });
});

// ── 2. Navigation metadata is never dropped ────────────────────────────────
//
// `create_browser_webview` starts loading before it returns, so the first
// `browser_page_load` can arrive before the setTabs that records the webviewId.
// A handler that only maps over existing tabs silently drops it, and the events
// are never replayed — the tab sits on "New Tab" forever, which also blinds the
// agent because it selects tabs by label.

/** The buffer-and-replay rule from Browser.tsx, driven through the real logic. */
function ingest(tabs: Tab[], pending: Map<string, PendingEvent[]>, id: string, ev: PendingEvent): Tab[] {
  if (!tabs.some(t => t.webviewId === id)) {
    bufferEvent(pending, id, ev);
    return tabs;
  }
  return tabs.map(t => (t.webviewId === id ? applyEvent(t as BrowserTab, ev) : t));
}

function drain(tabs: Tab[], pending: Map<string, PendingEvent[]>): Tab[] {
  return tabs.map(t => {
    if (!t.webviewId) return t;
    const held = pending.get(t.webviewId);
    if (!held) return t;
    pending.delete(t.webviewId);
    return replayEvents(t as BrowserTab, held);
  });
}

const BLANK: Tab[] = [{ id: 't1', label: 'New Tab', webviewId: null, url: '', loading: false }];

describe('navigation metadata ordering', () => {
  it('applies immediately when the tab already knows its webviewId', () => {
    const pending = new Map<string, PendingEvent[]>();
    const registered = [{ ...BLANK[0], webviewId: 'browser-1' }];
    const out = ingest(registered, pending, 'browser-1', { kind: 'load', url: 'https://x.dev', loading: true });
    expect(out[0].label).toBe('x.dev');
    expect(pending.size).toBe(0);
  });

  it('holds the update when the event beats registration, then applies it', () => {
    const pending = new Map<string, PendingEvent[]>();
    // Event first — this is the ordering that used to lose the label.
    let tabs = ingest(BLANK, pending, 'browser-1', { kind: 'load', url: 'https://x.dev', loading: true });
    expect(tabs[0].label).toBe('New Tab');
    expect(pending.get('browser-1')?.length).toBe(1);
    // Registration lands, then the replay runs.
    tabs = drain(tabs.map(t => ({ ...t, webviewId: 'browser-1' })), pending);
    expect(tabs[0].label).toBe('x.dev');
    expect(tabs[0].url).toBe('https://x.dev');
    expect(pending.size).toBe(0);
  });

  it('replays in ORDER, so a real page title still supersedes the host', () => {
    const pending = new Map<string, PendingEvent[]>();
    ingest(BLANK, pending, 'browser-1', { kind: 'load', url: 'https://x.dev', loading: true });
    ingest(BLANK, pending, 'browser-1', { kind: 'title', title: 'Actual Page Title' });
    const tabs = drain([{ ...BLANK[0], webviewId: 'browser-1' }], pending);
    expect(tabs[0].label).toBe('Actual Page Title');
    expect(tabs[0].url).toBe('https://x.dev'); // url from the earlier event survives
  });

  it('keeps buffered updates separate per webview', () => {
    const pending = new Map<string, PendingEvent[]>();
    ingest(BLANK, pending, 'browser-1', { kind: 'title', title: 'one' });
    ingest(BLANK, pending, 'browser-2', { kind: 'title', title: 'two' });
    const tabs = drain(
      [
        { id: 'a', label: 'New Tab', webviewId: 'browser-1', url: '', loading: false },
        { id: 'b', label: 'New Tab', webviewId: 'browser-2', url: '', loading: false },
      ],
      pending,
    );
    expect(tabs.map(t => t.label)).toEqual(['one', 'two']);
  });

  it('bounds the buffer for a webviewId no tab ever claims', () => {
    const pending = new Map<string, PendingEvent[]>();
    for (let i = 0; i < MAX_PENDING_EVENTS * 3; i++) {
      bufferEvent(pending, 'orphan', { kind: 'title', title: `t${i}` });
    }
    expect(pending.get('orphan')!.length).toBe(MAX_PENDING_EVENTS);
    // The newest events are the ones kept — they describe the current page.
    const kept = pending.get('orphan')!;
    expect(kept[kept.length - 1]).toEqual({ kind: 'title', title: `t${MAX_PENDING_EVENTS * 3 - 1}` });
  });
});

// ── 3. Teardown always terminates ───────────────────────────────────────────
//
// The pane window's close path preventDefaults, redocks, then destroys — and
// every step rides the DYING window's IPC bridge. When `destroy()` wedged, the
// window survived; adding a JS "fallback" that also invoked over that same
// bridge did nothing. The guarantee has to be reachable without the bridge.

/** Promise.race deadline, as used by PaneWindowApp. */
function withDeadline<T>(p: Promise<T>, ms: number, timers: Array<() => void>): Promise<T | undefined> {
  return Promise.race([
    p,
    new Promise<undefined>(resolve => {
      const id = setTimeout(() => resolve(undefined), ms);
      timers.push(() => clearTimeout(id));
    }),
  ]);
}

describe('pane teardown', () => {
  it('a bounded step resolves even when the underlying call never settles', async () => {
    const timers: Array<() => void> = [];
    const wedged = new Promise<boolean>(() => { /* never settles — the observed failure */ });
    const result = await withDeadline(wedged, 10, timers);
    expect(result).toBeUndefined(); // caller falls through to the native path
    timers.forEach(t => t());
  });

  it('a healthy step wins the race and skips the fallback', async () => {
    const timers: Array<() => void> = [];
    const result = await withDeadline(Promise.resolve(true), 1000, timers);
    expect(result).toBe(true);
    timers.forEach(t => t());
  });

  it('pane window labels are what the native guard actually matches', () => {
    // paneWindows.ts labels them `${kind}-${uuid}`. A guard checking "pane-"
    // (the first thing written) would never fire, and the native safety net
    // would silently do nothing.
    const isPane = (l: string) => l.startsWith('browser-') || l.startsWith('terminal-');
    expect(isPane('browser-3f2a-11ee-8c90')).toBe(true);
    expect(isPane('terminal-9b1c-4d02-aa31')).toBe(true);
    expect(isPane('pane-3f2a')).toBe(false); // the mistake this pins
    expect(isPane('main')).toBe(false);
    expect(isPane('chat')).toBe(false);
  });
});

// ── 4. Placeholder URLs never define a tab's identity ───────────────────────
//
// `new URL('about:blank')` parses fine and has an EMPTY hostname, so a
// hostname-derived label came back '' and rendered as "New Tab". Every popup
// passes through about:blank before its real URL, so fully-loaded sites sat
// unlabelled — invisible to the agent, which picks tabs by label.

describe('tab labelling', () => {
  it('never yields an empty label for a placeholder URL', () => {
    for (const u of ['about:blank', 'about:srcdoc', '', '   ']) {
      expect(extractTitle(u)).toBe('New Tab');
      expect(extractTitle(u)).not.toBe('');
    }
  });

  it('labels real sites by host, without www', () => {
    expect(extractTitle('https://www.bbc.com/news')).toBe('bbc.com');
    expect(extractTitle('https://cbc.ca/lite')).toBe('cbc.ca');
  });

  it('classifies placeholders so they cannot overwrite a real label', () => {
    expect(isPlaceholderUrl('about:blank')).toBe(true);
    expect(isPlaceholderUrl('')).toBe(true);
    expect(isPlaceholderUrl('https://bbc.com')).toBe(false);
  });

  it('ignores the empty title WKWebView reports between documents', () => {
    const titled: Tab = { id: 't', label: 'BBC News', webviewId: 'w', url: 'https://bbc.com', loading: false };
    expect(titleUpdate(titled, '')).toBe(titled);
    expect(titleUpdate(titled, '   ')).toBe(titled);
  });
});

// ── 5. Only the MAIN FRAME may define a tab's identity ──────────────────────
//
// The old `on_navigation` source wrapped decidePolicyForNavigationAction, which
// fires for every frame and hands over only a URL. CBC embeds Google's
// /api2/aframe, so the last iframe to load relabelled the tab `google.com` and
// put the ad frame's path in the address bar. The event now comes from
// on_page_load (didCommit/didFinish — main frame only, URL read from the
// webview), so a subframe simply never produces one. These pin the rules the
// frontend applies to that event.

const CBC: Tab = { id: 't', label: 'CBC News', webviewId: 'w', url: 'https://www.cbc.ca/news', loading: false };

describe('page-load updates', () => {
  it('re-derives the label only when the page actually changes', () => {
    const moved = pageLoadUpdate(CBC, 'https://www.bbc.com/news', true);
    expect(moved.label).toBe('bbc.com'); // the old page's title is no longer true
    expect(moved.url).toBe('https://www.bbc.com/news');
  });

  it('does not clobber the real title when the load FINISHES', () => {
    // didFinish arrives after browser_title_changed; re-deriving here would
    // throw the page's own title away every single load.
    const finished = pageLoadUpdate(CBC, 'https://www.cbc.ca/news', false);
    expect(finished.label).toBe('CBC News');
    expect(finished.loading).toBe(false);
  });

  it('keeps the title across a reload of the same URL', () => {
    expect(pageLoadUpdate(CBC, 'https://www.cbc.ca/news', true).label).toBe('CBC News');
  });

  it('never lets about:blank take the identity', () => {
    const blanked = pageLoadUpdate(CBC, 'about:blank', true);
    expect(blanked.url).toBe('https://www.cbc.ca/news');
    expect(blanked.label).toBe('CBC News');
  });

  it('is referentially stable when nothing changed, so React does not re-render', () => {
    expect(pageLoadUpdate(CBC, 'https://www.cbc.ca/news', false)).toBe(CBC);
    expect(urlOnlyUpdate(CBC, 'https://www.cbc.ca/news')).toBe(CBC);
    expect(titleUpdate(CBC, 'CBC News')).toBe(CBC);
  });
});

describe('same-document navigation', () => {
  // pushState fires no WebKit navigation callback, so the URL is pulled after a
  // title change. It moves the address, never the title that prompted the pull.
  it('moves the url and leaves the label alone', () => {
    const routed = urlOnlyUpdate(CBC, 'https://www.cbc.ca/sports');
    expect(routed.url).toBe('https://www.cbc.ca/sports');
    expect(routed.label).toBe('CBC News');
  });

  it('ignores a placeholder answer', () => {
    expect(urlOnlyUpdate(CBC, 'about:blank')).toBe(CBC);
  });
});

// ── 6. The address bar cannot drift from its tab ────────────────────────────
//
// It used to be independent state written from a dozen places, one of them the
// raw navigation event — so the bar could read google.com/recaptcha/api2/aframe
// while the tab beside it read cbc.ca, with nothing to reconcile them. It is
// now derived from the active tab; only live typing holds it back.

/** The mirror rule from Browser.tsx. */
function mirror(bar: string, dirty: boolean, tabUrl: string | undefined): string {
  return dirty ? bar : (tabUrl ?? '');
}

describe('address bar mirroring', () => {
  it('follows the tab whenever the user is not typing', () => {
    expect(mirror('stale', false, 'https://www.cbc.ca/news')).toBe('https://www.cbc.ca/news');
  });

  it('does not overwrite a half-typed URL', () => {
    expect(mirror('git', true, 'https://www.cbc.ca/news')).toBe('git');
  });

  it('empties for a tab that has never navigated', () => {
    expect(mirror('https://old.example', false, '')).toBe('');
  });
});

// ── 7. The Rust↔frontend contract, and no return to polling ─────────────────
//
// Source guards. The rules above are all behavioural, but three things they
// cannot reach have each broken this feature once:
//
//   * the EVENT NAME is a contract across two languages — rename one side and
//     every tab silently stops updating, with nothing red to show for it;
//   * the frontend must not listen to the removed all-frames event; and
//   * `browser_current_url` must stay a pull-on-signal, never a timer. The
//     first fix for the iframe bug was a 1.5s poll that corrected the tab
//     AFTER it had already displayed the wrong page. `browser.rs` carries the
//     matching guards (`identity_events_come_from_the_main_frame_hook` and
//     friends).
//
// The needles are split so this file cannot satisfy its own assertions.
import { readFileSync } from 'node:fs';

const BROWSER_TSX = readFileSync(new URL('./Browser.tsx', import.meta.url), 'utf8');
const PAGE_LOAD_EVENT = 'browser_' + 'page_load';
const LEGACY_EVENT = 'browser_' + 'navigated';
const NEW_WINDOW_EVENT = 'browser_' + 'new_window_request';
// The pull channel is now `browser_nav_state`: the same one round trip returns
// the authoritative URL AND canGoBack/canGoForward, so the restored back and
// forward buttons cannot disagree with the address bar. Renaming it is exactly
// the kind of change this guard exists to notice.
const NAV_STATE_CMD = 'browser_' + 'nav_state';

describe('browser event-source contract', () => {
  it('listens for the exact event name browser.rs emits', () => {
    expect(BROWSER_TSX).toContain(`api.listen('${PAGE_LOAD_EVENT}'`);
  });

  it('no longer listens to the removed all-frames navigation event', () => {
    // Still fine as an ACTIVITY type — that is a daemon-side label, not a
    // Tauri event — so this pins the listener specifically.
    expect(BROWSER_TSX).not.toContain(`api.listen('${LEGACY_EVENT}'`);
  });

  it('reads the authoritative URL on a signal, never on a timer', () => {
    // Exactly one call site: resyncUrl. More than one means someone added a
    // second path, and the one people reach for is a poll.
    const hits = BROWSER_TSX.split(NAV_STATE_CMD).length - 1;
    expect(hits).toBe(1);
    // The bounds pump legitimately uses setInterval, so this pins the timer
    // NOT being pointed at the URL resync rather than banning timers.
    expect(BROWSER_TSX).not.toMatch(/set(Interval|Timeout)\(\s*(resyncUrl|reconcile)/);
    expect(BROWSER_TSX).not.toMatch(/setInterval\([^)]*\b(resync|reconcile)/i);
  });

  it('routes popup links through the deny-and-reroute event', () => {
    // Without this listener, WKWebView's nil createWebView response makes
    // every target=_blank / window.open click a silent no-op (#240 / #709).
    expect(BROWSER_TSX).toContain(`api.listen('${NEW_WINDOW_EVENT}'`);
    expect(BROWSER_TSX).toContain('shouldOpenPopupTab');
    expect(BROWSER_TSX).toContain('openUrlRef');
  });
});

// ── 9. Popup clicks become in-app tabs, not silent drops ────────────────────
//
// Matching browser.rs's deny-and-emit: the shell must accept the event only
// for webviews it owns, and must refuse about:blank. Opening in the system
// browser would leave the agent blind to the page; opening every emit in
// every Browser instance doubles tabs across Build + detached panes.
describe('popup → in-app tab gate', () => {
  const owned = ['browser-0', 'browser-1'];

  it('opens a real URL from a webview this instance owns', () => {
    expect(shouldOpenPopupTab(owned, 'browser-0', 'https://example.com/meet')).toBe(true);
  });

  it('ignores a popup from a webview owned by another Browser instance', () => {
    expect(shouldOpenPopupTab(owned, 'browser-99', 'https://example.com')).toBe(false);
  });

  it('ignores about:blank — window.open() handshake, not a destination', () => {
    expect(shouldOpenPopupTab(owned, 'browser-0', 'about:blank')).toBe(false);
    expect(shouldOpenPopupTab(owned, 'browser-0', '')).toBe(false);
  });

  it('ignores a missing source id rather than opening unbound', () => {
    expect(shouldOpenPopupTab(owned, '', 'https://example.com')).toBe(false);
  });
});

// ── 8. Back/forward act on real history ─────────────────────────────────────
//
// These buttons were REMOVED in the 2026-07 audit because they were rendered
// permanently disabled with no handler, and tooltips promised Cmd+[ / Cmd+]
// shortcuts that did not exist. A control that cannot act is worse than no
// control, so they only return with WKWebView's own history behind them.
describe('browser history controls', () => {
  it('both buttons have a handler — never rendered inert again', () => {
    expect(BROWSER_TSX).toMatch(/onClick=\{\(\) => goHistory\(false\)\}/);
    expect(BROWSER_TSX).toMatch(/onClick=\{\(\) => goHistory\(true\)\}/);
  });

  it('enabled state comes from the webview, not a URL list we keep', () => {
    expect(BROWSER_TSX).toContain('disabled={!navState.canGoBack}');
    expect(BROWSER_TSX).toContain('disabled={!navState.canGoForward}');
    // A self-kept history array desyncs on the first redirect or fragment change.
    expect(BROWSER_TSX).not.toMatch(/historyStack|backStack|forwardStack/);
  });

  it('does not promise keyboard shortcuts it has not wired', () => {
    const titles = BROWSER_TSX.match(/title="(Back|Forward)"/g) ?? [];
    expect(titles.length).toBe(2);
    expect(BROWSER_TSX).not.toMatch(/title="Back \(Cmd|title="Forward \(Cmd/);
  });
});
