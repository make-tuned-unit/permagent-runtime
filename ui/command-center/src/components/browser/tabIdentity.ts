/**
 * What a browser tab and the address bar are allowed to say.
 *
 * Extracted from Browser.tsx so there is ONE implementation of these rules and
 * it can be tested without a Tauri bridge, a ResizeObserver or a live webview.
 * Every rule here exists because a tab or the address bar once displayed
 * something that was not the page the user was looking at.
 */
import type { BrowserTab } from './BrowserTabs';

/** `about:blank` and friends carry no identity — a webview sits there between
 *  creation and its first real navigation, and WebKit commits it as a real
 *  page load. Treating that as informative lets a transient state overwrite a
 *  good label with nothing. */
export function isPlaceholderUrl(url: string): boolean {
  const u = url.trim().toLowerCase();
  return u === '' || u === 'about:blank' || u.startsWith('about:');
}

/**
 * A tab label derived from a URL. NEVER returns empty.
 *
 * `new URL('about:blank')` parses fine and has an EMPTY hostname, so a naive
 * `hostname` read handed back `''`, which renders as "New Tab" and is
 * indistinguishable from a tab that was never navigated. An unlabelled tab is
 * also invisible to the agent, which selects tabs by label.
 */
export function extractTitle(url: string): string {
  try {
    const host = new URL(url).hostname.replace(/^www\./, '');
    if (host) return host;
  } catch {
    // not a URL — fall through to the raw slice below
  }
  const raw = url.slice(0, 30).trim();
  return raw && !isPlaceholderUrl(url) ? raw : 'New Tab';
}

/**
 * A main-frame page-load transition (`browser_page_load` from Rust).
 *
 * `loading` is true at commit and false at finish. The label is only re-derived
 * on a commit onto a DIFFERENT url: that is the moment the old page's title
 * stops being true, and `browser_title_changed` will supply the real one a beat
 * later. Doing it on finish as well would clobber the title that just arrived,
 * and doing it on a reload would throw away a perfectly good one.
 */
export function pageLoadUpdate(tab: BrowserTab, url: string, loading: boolean): BrowserTab {
  if (isPlaceholderUrl(url)) return tab.loading === loading ? tab : { ...tab, loading };
  const movedToNewPage = tab.url !== url;
  if (!movedToNewPage && tab.loading === loading) return tab;
  return {
    ...tab,
    url,
    label: loading && movedToNewPage ? extractTitle(url) : tab.label,
    loading,
  };
}

/**
 * A document title change (`browser_title_changed`).
 *
 * WKWebView reports an EMPTY title while a page is between documents; taking it
 * would blank the tab. A title is only ever an improvement on a host-derived
 * label, so an empty one is simply ignored.
 */
export function titleUpdate(tab: BrowserTab, title: string): BrowserTab {
  const trimmed = title.trim();
  if (!trimmed || trimmed === tab.label) return tab;
  return { ...tab, label: trimmed };
}

/**
 * A same-document URL change — `history.pushState`, a hash change, an SPA
 * route. WebKit fires no navigation callback for these, so they arrive as a
 * re-read of `WKWebView.URL` rather than as a push. Identity moves; the title
 * does not (the title event that prompted the re-read already set it).
 */
export function urlOnlyUpdate(tab: BrowserTab, url: string): BrowserTab {
  if (isPlaceholderUrl(url) || tab.url === url) return tab;
  return { ...tab, url };
}

/**
 * An event that arrived for a webview React has not registered yet.
 *
 * `create_browser_webview` starts loading the page before it returns, so the
 * first `browser_page_load` can beat the `setTabs` that records the webviewId.
 * A handler that only `map`s over existing tabs drops it silently, and nothing
 * replays it — the tab sits on "New Tab" forever. These are buffered instead
 * and replayed IN ORDER once the id is claimed, so a title that followed a load
 * still wins.
 */
export type PendingEvent =
  | { kind: 'load'; url: string; loading: boolean }
  | { kind: 'title'; title: string }
  | { kind: 'url'; url: string };

/** Bound on the buffer, so a webview id that is never claimed (a tab whose
 *  creation failed) cannot grow it without limit. */
export const MAX_PENDING_EVENTS = 32;

export function applyEvent(tab: BrowserTab, ev: PendingEvent): BrowserTab {
  switch (ev.kind) {
    case 'load':
      return pageLoadUpdate(tab, ev.url, ev.loading);
    case 'title':
      return titleUpdate(tab, ev.title);
    case 'url':
      return urlOnlyUpdate(tab, ev.url);
  }
}

export function replayEvents(tab: BrowserTab, events: PendingEvent[]): BrowserTab {
  return events.reduce(applyEvent, tab);
}

export function bufferEvent(
  pending: Map<string, PendingEvent[]>,
  webviewId: string,
  ev: PendingEvent,
): void {
  const held = pending.get(webviewId) ?? [];
  held.push(ev);
  if (held.length > MAX_PENDING_EVENTS) held.splice(0, held.length - MAX_PENDING_EVENTS);
  pending.set(webviewId, held);
}

/**
 * Whether THIS Browser instance should open a tab for a `browser_new_window_request`.
 *
 * Rust denies the native popup and emits globally. Two gates keep that from
 * either doing nothing useful or opening N tabs:
 *   1. ownership — only the instance whose tab strip already hosts
 *      `source_webview_id` may act (Build + detached panes both listen);
 *   2. placeholder URL — `window.open()` often starts at about:blank; opening
 *      that as a tab littered the strip with empty "New Tab"s while real
 *      target=_blank hrefs arrive as absolute http(s) and pass.
 */
export function shouldOpenPopupTab(
  ownedWebviewIds: ReadonlyArray<string | null | undefined>,
  sourceWebviewId: string,
  url: string,
): boolean {
  if (!sourceWebviewId || !ownedWebviewIds.includes(sourceWebviewId)) return false;
  if (isPlaceholderUrl(url)) return false;
  return true;
}
