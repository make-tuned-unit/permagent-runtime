/**
 * api.getHealth — the hub's `/status` answers with the plain text `ok`.
 * Settings > Devices read the resulting JSON SyntaxError as "the hub is not
 * answering", so every browser (proxied or not) was told its hub was down.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from './api';

describe('api.getHealth', () => {
  beforeEach(() => {
    vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
    vi.stubGlobal('localStorage', { getItem: () => 'tok-test', setItem: () => undefined });
  });
  afterEach(() => { vi.unstubAllGlobals(); });

  it('counts a plain-text "ok" as the hub answering', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, status: 200, text: async () => 'ok' })));
    expect(await api.getHealth()).toBe(true);
  });

  it('is false when the request cannot be sent or the hub errors', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => { throw new TypeError('Failed to fetch'); }));
    expect(await api.getHealth()).toBe(false);
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false, status: 503, json: async () => ({}) })));
    expect(await api.getHealth()).toBe(false);
  });
});
