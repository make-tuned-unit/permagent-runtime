/**
 * Pure list logic for the browser bookmarks bar + saved tab sets (#790).
 * Kept free of React/fetch so the behavior is unit-testable; BookmarksBar
 * applies these and persists the result via the daemon API (lib/api.ts).
 */

import type { BrowserBookmark, BrowserSavedTab, BrowserTabSet } from '../../lib/api';

/** Only http(s) URLs are bookmarkable/savable — mirrors the daemon's
 *  validation (routes/browser_state.rs) so a PUT never 400s on scheme. */
export function isPersistableUrl(url: string): boolean {
  return url.startsWith('https://') || url.startsWith('http://');
}

export function isBookmarked(bookmarks: BrowserBookmark[], url: string): boolean {
  return bookmarks.some((b) => b.url === url);
}

/** Star toggle: remove the URL if bookmarked, else append it. Returns a new
 *  list; `null` when the URL is not persistable (empty tab, non-web scheme). */
export function toggleBookmark(
  bookmarks: BrowserBookmark[],
  url: string,
  title: string,
  now: Date = new Date(),
): BrowserBookmark[] | null {
  if (!isPersistableUrl(url)) return null;
  if (isBookmarked(bookmarks, url)) {
    return bookmarks.filter((b) => b.url !== url);
  }
  return [...bookmarks, { url, title: title || url, createdAt: now.toISOString() }];
}

export function removeBookmark(bookmarks: BrowserBookmark[], url: string): BrowserBookmark[] {
  return bookmarks.filter((b) => b.url !== url);
}

/** The open tabs worth persisting in a set: real web pages only (blank tabs
 *  and error placeholders have no URL). */
export function savableTabs(tabs: Array<{ url: string; label: string }>): BrowserSavedTab[] {
  return tabs
    .filter((t) => isPersistableUrl(t.url))
    .map((t) => ({ url: t.url, title: t.label }));
}

/** Save the open tabs under `name`. Re-using an existing name overwrites that
 *  set (same-name save = update, keeping names unique as the daemon requires).
 *  Returns `null` when the trimmed name is empty or there is nothing to save. */
export function saveTabSet(
  sets: BrowserTabSet[],
  name: string,
  tabs: BrowserSavedTab[],
  now: Date = new Date(),
): BrowserTabSet[] | null {
  const trimmed = name.trim();
  if (!trimmed || tabs.length === 0) return null;
  const kept = sets.filter((s) => s.name !== trimmed);
  return [...kept, { name: trimmed, tabs, createdAt: now.toISOString() }];
}

export function removeTabSet(sets: BrowserTabSet[], name: string): BrowserTabSet[] {
  return sets.filter((s) => s.name !== name);
}
