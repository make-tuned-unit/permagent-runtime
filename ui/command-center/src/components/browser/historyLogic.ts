import type { BrowserHistoryEntry } from '../../lib/api';

/** Merge one successful web navigation into the daemon-backed suggestion list. */
export function recordHistory(
  entries: BrowserHistoryEntry[],
  url: string,
  title: string,
  now: Date = new Date(),
): BrowserHistoryEntry[] {
  if (!/^https?:\/\//i.test(url)) return entries;
  const existing = entries.find((entry) => entry.url === url);
  const next = existing
    ? entries.map((entry) => entry.url === url
      ? { ...entry, title: title || entry.title, lastVisited: now.toISOString(), visitCount: entry.visitCount + 1 }
      : entry)
    : [...entries, { url, title: title || url, lastVisited: now.toISOString(), visitCount: 1 }];
  return next
    .sort((a, b) => b.visitCount - a.visitCount || b.lastVisited.localeCompare(a.lastVisited) || a.url.localeCompare(b.url))
    .slice(0, 100);
}

/** Suggestions are explicit, bounded, and ranked by frequency then recency. */
export function historySuggestions(
  entries: BrowserHistoryEntry[],
  query: string,
  limit = 8,
): BrowserHistoryEntry[] {
  const needle = query.trim().toLocaleLowerCase();
  return entries
    .filter((entry) => !needle || `${entry.url} ${entry.title}`.toLocaleLowerCase().includes(needle))
    .slice()
    .sort((a, b) => b.visitCount - a.visitCount || b.lastVisited.localeCompare(a.lastVisited))
    .slice(0, Math.max(0, limit));
}
