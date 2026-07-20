// @vitest-environment jsdom
/**
 * Device registry wire contract (#628).
 *
 * Two seams under test, mirroring pairingCapture.test.ts:
 *
 * 1. Claim-code capture: a companion opening /ui/#claim=<code> must scrub the
 *    code from the URL, exchange it via the public POST /pair/claim for its
 *    OWN device token, persist that token, and report the genuine
 *    `devices_paired` completion authenticated with the fresh credential.
 *    A failed exchange must fall back to any stored credential and emit
 *    nothing.
 *
 * 2. The Settings panel's registry calls: list / pair / rename / revoke must
 *    hit the real daemon endpoints with the right method, path, body, and
 *    bearer header — the NO-DEAD-UI contract for the Devices panel.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const TOKEN_KEY = 'permagent-daemon-token';

function makeStorage(): Storage {
  const m = new Map<string, string>();
  return {
    get length() {
      return m.size;
    },
    clear: () => m.clear(),
    getItem: (k: string) => (m.has(k) ? m.get(k)! : null),
    key: (i: number) => Array.from(m.keys())[i] ?? null,
    removeItem: (k: string) => {
      m.delete(k);
    },
    setItem: (k: string, v: string) => {
      m.set(k, String(v));
    },
  };
}

const fetchMock = vi.fn();

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

async function importApiFresh() {
  vi.resetModules();
  return await import('./api');
}

beforeEach(() => {
  vi.stubGlobal('localStorage', makeStorage());
  history.replaceState(null, '', '/ui/');
  fetchMock.mockReset();
  fetchMock.mockImplementation(() => Promise.resolve(jsonResponse({})));
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('claim-code capture (companion first load, #628)', () => {
  it('exchanges #claim= for a device token, scrubs the URL, stores it, and reports devices_paired', async () => {
    fetchMock.mockImplementation((url: string) => {
      if (String(url).endsWith('/pair/claim')) {
        return Promise.resolve(
          jsonResponse({ token: 'dev-tok-1', device: { id: 'd1', name: 'iPhone' } }),
        );
      }
      return Promise.resolve(jsonResponse({ accepted: true }));
    });

    window.location.hash = '#claim=code-abc';
    const { loadDaemonToken } = await importApiFresh();
    await expect(loadDaemonToken()).resolves.toBe('dev-tok-1');

    // The code is scrubbed before the exchange resolves — it never lingers.
    expect(window.location.hash).toBe('');
    expect(localStorage.getItem(TOKEN_KEY)).toBe('dev-tok-1');

    // Exchange: public POST /pair/claim carrying ONLY the code, no auth.
    const claimCall = fetchMock.mock.calls.find(([u]) => String(u).endsWith('/pair/claim'));
    expect(claimCall).toBeDefined();
    const [, claimInit] = claimCall as [string, RequestInit];
    expect(claimInit.method).toBe('POST');
    expect(JSON.parse(claimInit.body as string)).toEqual({ code: 'code-abc' });
    expect((claimInit.headers as Record<string, string>).Authorization).toBeUndefined();

    // Completion report authenticated with the freshly minted device token.
    const emitCall = fetchMock.mock.calls.find(([u]) => String(u).endsWith('/activity/emit'));
    expect(emitCall).toBeDefined();
    const [, emitInit] = emitCall as [string, RequestInit];
    expect((emitInit.headers as Record<string, string>).Authorization).toBe('Bearer dev-tok-1');
    const event = JSON.parse(emitInit.body as string);
    expect(event.event_type).toBe('devices_paired');
    // The token must never ride in the event body.
    expect(emitInit.body as string).not.toContain('dev-tok-1');
  });

  it('keeps other fragment params while scrubbing only the claim code', async () => {
    fetchMock.mockImplementation((url: string) =>
      Promise.resolve(
        String(url).endsWith('/pair/claim')
          ? jsonResponse({ token: 'dev-tok-2', device: { id: 'd2', name: 'iPad' } })
          : jsonResponse({ accepted: true }),
      ),
    );
    window.location.hash = '#claim=code-xyz&view=world';
    const { loadDaemonToken } = await importApiFresh();
    await loadDaemonToken();
    expect(window.location.hash).toBe('#view=world');
  });

  it('falls back to the stored credential and emits nothing when the claim is rejected (used/expired)', async () => {
    localStorage.setItem(TOKEN_KEY, 'tok-existing');
    fetchMock.mockImplementation((url: string) =>
      Promise.resolve(
        String(url).endsWith('/pair/claim')
          ? jsonResponse({ error: 'unknown code' }, 404)
          : jsonResponse({ accepted: true }),
      ),
    );
    window.location.hash = '#claim=dead-code';
    const { loadDaemonToken } = await importApiFresh();
    await expect(loadDaemonToken()).resolves.toBe('tok-existing');
    expect(localStorage.getItem(TOKEN_KEY)).toBe('tok-existing');
    const emitCall = fetchMock.mock.calls.find(([u]) => String(u).endsWith('/activity/emit'));
    expect(emitCall).toBeUndefined();
  });

  it('legacy #token= capture still works unchanged (zero-breakage)', async () => {
    window.location.hash = '#token=legacy-tok';
    const { loadDaemonToken } = await importApiFresh();
    await expect(loadDaemonToken()).resolves.toBe('legacy-tok');
    expect(localStorage.getItem(TOKEN_KEY)).toBe('legacy-tok');
    expect(window.location.hash).toBe('');
  });
});

describe('Devices panel wire contract (#628)', () => {
  async function apiWithToken() {
    localStorage.setItem(TOKEN_KEY, 'panel-tok');
    const mod = await importApiFresh();
    return mod.api;
  }

  it('listDevices GETs /api/devices with the bearer header', async () => {
    const api = await apiWithToken();
    fetchMock.mockImplementation(() => Promise.resolve(jsonResponse([])));
    await expect(api.listDevices()).resolves.toEqual([]);
    const [url, init] = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as [string, RequestInit];
    expect(url).toBe('/api/devices');
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer panel-tok');
  });

  it('pairDevice POSTs the name to /api/devices/pair and returns the claim code', async () => {
    const api = await apiWithToken();
    fetchMock.mockImplementation(() =>
      Promise.resolve(jsonResponse({ claim_code: 'c1', expires_at: '2026-07-20T00:00:00Z' })),
    );
    const r = await api.pairDevice('iPhone');
    expect(r.claim_code).toBe('c1');
    const [url, init] = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as [string, RequestInit];
    expect(url).toBe('/api/devices/pair');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toEqual({ name: 'iPhone' });
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer panel-tok');
  });

  it('renameDevice PATCHes /api/devices/{id}', async () => {
    const api = await apiWithToken();
    fetchMock.mockImplementation(() =>
      Promise.resolve(
        jsonResponse({ id: 'd1', name: 'New', created: '', last_seen: null, revoked: false }),
      ),
    );
    await api.renameDevice('d1', 'New');
    const [url, init] = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as [string, RequestInit];
    expect(url).toBe('/api/devices/d1');
    expect(init.method).toBe('PATCH');
    expect(JSON.parse(init.body as string)).toEqual({ name: 'New' });
  });

  it('revokeDevice POSTs /api/devices/{id}/revoke', async () => {
    const api = await apiWithToken();
    fetchMock.mockImplementation(() =>
      Promise.resolve(
        jsonResponse({ id: 'd1', name: 'iPhone', created: '', last_seen: null, revoked: true }),
      ),
    );
    const r = await api.revokeDevice('d1');
    expect(r.revoked).toBe(true);
    const [url, init] = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as [string, RequestInit];
    expect(url).toBe('/api/devices/d1/revoke');
    expect(init.method).toBe('POST');
  });

  it('URL-encodes device ids in paths', async () => {
    const api = await apiWithToken();
    fetchMock.mockImplementation(() =>
      Promise.resolve(
        jsonResponse({ id: 'a/b', name: 'x', created: '', last_seen: null, revoked: false }),
      ),
    );
    await api.renameDevice('a/b', 'x');
    const [url] = fetchMock.mock.calls[fetchMock.mock.calls.length - 1] as [string, RequestInit];
    expect(url).toBe('/api/devices/a%2Fb');
  });
});
