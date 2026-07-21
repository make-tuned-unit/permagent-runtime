/**
 * Wire-contract tests for the browser bookmarks + saved-tabs API client
 * (#790), with a stubbed fetch: endpoint paths, PUT method, camelCase body
 * keys (`tabSets`, per routes/browser_state.rs rename_all), and response
 * unwrapping. This pins the exact wiring BookmarksBar rides.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from './api';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('api.getBrowserBookmarks', () => {
  it('GETs the mounted endpoint and returns the bookmark list', async () => {
    const bookmarks = [
      { url: 'https://example.com', title: 'Example', createdAt: '2026-07-20T00:00:00Z' },
    ];
    fetchMock.mockResolvedValueOnce(jsonResponse({ bookmarks }));

    const res = await api.getBrowserBookmarks();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toBe('/api/browser/bookmarks');
    expect(res.bookmarks).toEqual(bookmarks);
  });
});

describe('api.putBrowserBookmarks', () => {
  it('PUTs the full list as {bookmarks} JSON', async () => {
    const bookmarks = [
      { url: 'https://example.com', title: 'Example', createdAt: '2026-07-20T00:00:00Z' },
    ];
    fetchMock.mockResolvedValueOnce(jsonResponse({ bookmarks }));

    await api.putBrowserBookmarks(bookmarks);

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('/api/browser/bookmarks');
    expect(init.method).toBe('PUT');
    expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
    expect(JSON.parse(init.body as string)).toEqual({ bookmarks });
  });

  it('surfaces the daemon validation message on 400', async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ message: 'duplicate bookmark url: https://a.dev' }, 400),
    );
    await expect(
      api.putBrowserBookmarks([
        { url: 'https://a.dev', title: 'A', createdAt: 'x' },
        { url: 'https://a.dev', title: 'A', createdAt: 'x' },
      ]),
    ).rejects.toThrow('duplicate bookmark url: https://a.dev');
  });
});

describe('api.getBrowserTabSets', () => {
  it('GETs the mounted endpoint and returns the tab-set list', async () => {
    const tabSets = [
      {
        name: 'Research',
        tabs: [{ url: 'https://example.com', title: 'Example' }],
        createdAt: '2026-07-20T00:00:00Z',
      },
    ];
    fetchMock.mockResolvedValueOnce(jsonResponse({ tabSets }));

    const res = await api.getBrowserTabSets();

    expect(fetchMock.mock.calls[0][0]).toBe('/api/browser/tab-sets');
    expect(res.tabSets).toEqual(tabSets);
  });
});

describe('api.putBrowserTabSets', () => {
  it('PUTs the full list under the camelCase tabSets key', async () => {
    const tabSets = [
      {
        name: 'Research',
        tabs: [{ url: 'https://example.com', title: 'Example' }],
        createdAt: '2026-07-20T00:00:00Z',
      },
    ];
    fetchMock.mockResolvedValueOnce(jsonResponse({ tabSets }));

    await api.putBrowserTabSets(tabSets);

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('/api/browser/tab-sets');
    expect(init.method).toBe('PUT');
    const body = JSON.parse(init.body as string);
    // rename_all = camelCase on the daemon: the key must be tabSets.
    expect(body).toEqual({ tabSets });
    expect(body.tab_sets).toBeUndefined();
  });
});
