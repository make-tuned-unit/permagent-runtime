/**
 * Bookmarks + saved-tabs pure list logic (#790). These are the exact
 * transitions BookmarksBar persists via PUT /api/browser/* — behavior pinned
 * here, wire contract pinned in lib/browserState.test.ts.
 */

import { describe, expect, it } from 'vitest';
import type { BrowserBookmark, BrowserTabSet } from '../../lib/api';
import {
  isBookmarked,
  isPersistableUrl,
  removeBookmark,
  removeTabSet,
  savableTabs,
  saveTabSet,
  toggleBookmark,
} from './bookmarksLogic';

const NOW = new Date('2026-07-20T12:00:00Z');

const bm = (url: string, title = 'Page'): BrowserBookmark => ({
  url,
  title,
  createdAt: '2026-07-19T00:00:00Z',
});

describe('isPersistableUrl', () => {
  it('accepts only http(s) — mirrors the daemon validation', () => {
    expect(isPersistableUrl('https://example.com')).toBe(true);
    expect(isPersistableUrl('http://example.com')).toBe(true);
    expect(isPersistableUrl('')).toBe(false);
    expect(isPersistableUrl('javascript:alert(1)')).toBe(false);
    expect(isPersistableUrl('file:///etc/passwd')).toBe(false);
    expect(isPersistableUrl('example.com')).toBe(false);
  });
});

describe('toggleBookmark (the star control)', () => {
  it('appends a new bookmark with title and timestamp', () => {
    const next = toggleBookmark([], 'https://example.com', 'Example', NOW);
    expect(next).toEqual([
      { url: 'https://example.com', title: 'Example', createdAt: NOW.toISOString() },
    ]);
  });

  it('falls back to the URL when the page has no title', () => {
    const next = toggleBookmark([], 'https://example.com', '', NOW);
    expect(next?.[0].title).toBe('https://example.com');
  });

  it('removes an existing bookmark on re-star (toggle off)', () => {
    const list = [bm('https://a.dev'), bm('https://b.dev')];
    expect(toggleBookmark(list, 'https://a.dev', 'A', NOW)).toEqual([bm('https://b.dev')]);
  });

  it('returns null for non-persistable URLs (blank tab, non-web scheme)', () => {
    expect(toggleBookmark([], '', 'New Tab', NOW)).toBeNull();
    expect(toggleBookmark([], 'javascript:alert(1)', 'x', NOW)).toBeNull();
  });

  it('never produces duplicate URLs (the daemon rejects them)', () => {
    const once = toggleBookmark([], 'https://a.dev', 'A', NOW)!;
    const twice = toggleBookmark(once, 'https://a.dev', 'A', NOW)!;
    expect(twice).toEqual([]);
  });
});

describe('isBookmarked / removeBookmark', () => {
  it('matches by exact URL', () => {
    const list = [bm('https://a.dev')];
    expect(isBookmarked(list, 'https://a.dev')).toBe(true);
    expect(isBookmarked(list, 'https://a.dev/path')).toBe(false);
  });

  it('removeBookmark drops only the given URL', () => {
    const list = [bm('https://a.dev'), bm('https://b.dev')];
    expect(removeBookmark(list, 'https://a.dev')).toEqual([bm('https://b.dev')]);
  });
});

describe('savableTabs', () => {
  it('keeps only real web pages, mapping label → title', () => {
    const tabs = [
      { url: 'https://a.dev', label: 'A' },
      { url: '', label: 'New Tab' },
      { url: 'https://b.dev', label: 'B' },
    ];
    expect(savableTabs(tabs)).toEqual([
      { url: 'https://a.dev', title: 'A' },
      { url: 'https://b.dev', title: 'B' },
    ]);
  });
});

describe('saveTabSet / removeTabSet', () => {
  const tabs = [{ url: 'https://a.dev', title: 'A' }];

  it('appends a named set with trimmed name', () => {
    const next = saveTabSet([], '  Research  ', tabs, NOW);
    expect(next).toEqual([{ name: 'Research', tabs, createdAt: NOW.toISOString() }]);
  });

  it('overwrites an existing set of the same name (names stay unique)', () => {
    const existing: BrowserTabSet[] = [
      { name: 'Research', tabs: [{ url: 'https://old.dev', title: 'Old' }], createdAt: 'x' },
      { name: 'Work', tabs, createdAt: 'y' },
    ];
    const next = saveTabSet(existing, 'Research', tabs, NOW)!;
    expect(next).toHaveLength(2);
    expect(next.filter((s) => s.name === 'Research')).toEqual([
      { name: 'Research', tabs, createdAt: NOW.toISOString() },
    ]);
  });

  it('returns null on empty name or nothing to save (daemon would 400)', () => {
    expect(saveTabSet([], '   ', tabs, NOW)).toBeNull();
    expect(saveTabSet([], 'Research', [], NOW)).toBeNull();
  });

  it('removeTabSet drops only the named set', () => {
    const sets: BrowserTabSet[] = [
      { name: 'Research', tabs, createdAt: 'x' },
      { name: 'Work', tabs, createdAt: 'y' },
    ];
    expect(removeTabSet(sets, 'Research')).toEqual([{ name: 'Work', tabs, createdAt: 'y' }]);
  });
});
