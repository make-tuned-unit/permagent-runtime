/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it } from 'vitest';
import { SEARCH_TEMPLATE_KEY, getSearchTemplate, resolveAddress } from './addressInput';

beforeEach(() => localStorage.clear());

describe('resolveAddress', () => {
  it('opens URLs and hostnames with no search engine configured', () => {
    expect(resolveAddress('https://example.org/a', null)).toEqual({ kind: 'url', url: 'https://example.org/a' });
    expect(resolveAddress('example.org', null)).toEqual({ kind: 'url', url: 'https://example.org' });
    expect(resolveAddress('localhost:3000/x', null)).toEqual({ kind: 'url', url: 'http://localhost:3000/x' });
  });

  it('never falls back to a vendor: free text with no engine asks for search setup', () => {
    expect(resolveAddress('best pizza near me', null)).toEqual({ kind: 'needs-search-setup', query: 'best pizza near me' });
  });

  it('uses the user-configured engine for free text', () => {
    expect(resolveAddress('a b', 'https://search.example/?q=%s')).toEqual({ kind: 'url', url: 'https://search.example/?q=a%20b' });
  });

  it('only a well-formed stored template counts', () => {
    expect(getSearchTemplate()).toBeNull();
    localStorage.setItem(SEARCH_TEMPLATE_KEY, 'not a url');
    expect(getSearchTemplate()).toBeNull();
    localStorage.setItem(SEARCH_TEMPLATE_KEY, 'https://search.example/?q=%s');
    expect(getSearchTemplate()).toBe('https://search.example/?q=%s');
  });

  it('empty input is empty', () => {
    expect(resolveAddress('  ', null)).toEqual({ kind: 'empty' });
  });
});
