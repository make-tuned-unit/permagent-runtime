/**
 * Address-bar input resolution. A typed address is either something we can open
 * as a URL or a search; a search needs a search engine, and no engine is baked
 * in. The user's choice lives in localStorage as a URL template with `%s` where
 * the query goes. Until one is set, URLs and hostnames still work and anything
 * else asks for search to be set up instead of silently going to a vendor.
 */

export const SEARCH_TEMPLATE_KEY = 'permagent.browser.searchTemplate';

export type AddressResolution =
  | { kind: 'empty' }
  | { kind: 'url'; url: string }
  | { kind: 'needs-search-setup'; query: string };

/** The configured template, or null. Only http(s) templates containing %s count. */
export function getSearchTemplate(): string | null {
  try {
    const raw = localStorage.getItem(SEARCH_TEMPLATE_KEY)?.trim();
    return raw && /^https?:\/\/.+%s/i.test(raw) ? raw : null;
  } catch {
    return null;
  }
}

const HOSTNAME = /^[a-zA-Z0-9-]+(\.[a-zA-Z0-9-]+)*\.[a-zA-Z]{2,}(:\d+)?([/?#].*)?$/;
const LOCAL = /^(localhost|\d{1,3}(\.\d{1,3}){3})(:\d+)?([/?#].*)?$/i;

export function resolveAddress(input: string, searchTemplate: string | null): AddressResolution {
  const trimmed = input.trim();
  if (!trimmed) return { kind: 'empty' };
  if (/^https?:\/\//i.test(trimmed)) return { kind: 'url', url: trimmed };
  if (HOSTNAME.test(trimmed)) return { kind: 'url', url: `https://${trimmed}` };
  if (LOCAL.test(trimmed)) return { kind: 'url', url: `http://${trimmed}` };
  if (searchTemplate) return { kind: 'url', url: searchTemplate.replace('%s', encodeURIComponent(trimmed)) };
  return { kind: 'needs-search-setup', query: trimmed };
}
