// @vitest-environment jsdom
/**
 * Pairing-capture seam — the truthful `devices_paired` signal.
 *
 * A companion device pairs by opening http://<hub>:3001/ui/#token=<token>; the
 * token rides the URL fragment (never sent to the server), so the daemon can
 * only learn of a completed pairing from the capture code itself. These tests
 * drive `api.ts`'s module-load capture (`browserToken` via the boot-time
 * `loadDaemonToken()`) and assert the emission contract:
 *
 *   - a genuine first capture stores the token, scrubs the URL, and POSTs an
 *     authenticated `devices_paired` to `/activity/emit` (no token in the body);
 *   - re-opening the same pairing URL does NOT re-claim a pairing;
 *   - capturing a rotated (different) token DOES count as a new pairing;
 *   - a plain load with no fragment emits nothing.
 *
 * Each test resets modules and imports `./api` fresh, because capture runs as a
 * module side effect exactly once per page load — the same shape as production.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const TOKEN_KEY = 'permagent-daemon-token';
const fetchMock = vi.fn(() => Promise.resolve(new Response('{"accepted":true}')));

/** Map-backed Storage: Node ≥22 defines an experimental global `localStorage`
 *  accessor that shadows jsdom's storage and yields `undefined` unless the
 *  process was started with `--localstorage-file`, so neither node's nor
 *  jsdom's is usable here. This behaves like the real thing for the
 *  getItem/setItem/clear surface the pairing code touches. */
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

function setHash(hash: string) {
  // jsdom supports fragment-only navigation + history.replaceState.
  window.location.hash = hash;
}

async function importApiFresh() {
  vi.resetModules();
  return await import('./api');
}

beforeEach(() => {
  vi.stubGlobal('localStorage', makeStorage());
  history.replaceState(null, '', '/ui/');
  fetchMock.mockClear();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('pairing token capture (browser context)', () => {
  it('captures the token, scrubs the URL, and reports devices_paired with the captured credential', async () => {
    setHash('#token=tok-first');
    const { loadDaemonToken } = await importApiFresh();

    expect(localStorage.getItem(TOKEN_KEY)).toBe('tok-first');
    expect(window.location.hash).toBe('');
    await expect(loadDaemonToken()).resolves.toBe('tok-first');

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/activity/emit');
    expect(init.method).toBe('POST');
    // Self-proving: the report authenticates with the just-captured token, so
    // an invalid token can never mint the event.
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok-first');

    const body = init.body as string;
    const event = JSON.parse(body);
    expect(event).toMatchObject({
      event_type: 'devices_paired',
      source_surface: 'settings',
      tier: 'always',
      payload: {},
    });
    expect(event.event_id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
    );
    expect(Number.isNaN(Date.parse(event.timestamp))).toBe(false);
    // The bearer secret must never ride in the event body.
    expect(body).not.toContain('tok-first');
  });

  it('does not re-claim a pairing when the same URL is re-opened (credential already held)', async () => {
    localStorage.setItem(TOKEN_KEY, 'tok-first');
    setHash('#token=tok-first');
    await importApiFresh();

    expect(localStorage.getItem(TOKEN_KEY)).toBe('tok-first');
    expect(window.location.hash).toBe('');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('treats a rotated token as a genuine new pairing', async () => {
    localStorage.setItem(TOKEN_KEY, 'tok-old');
    setHash('#token=tok-rotated');
    await importApiFresh();

    expect(localStorage.getItem(TOKEN_KEY)).toBe('tok-rotated');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok-rotated');
    expect(JSON.parse(init.body as string).event_type).toBe('devices_paired');
  });

  it('emits nothing on a plain load with no token fragment', async () => {
    const { loadDaemonToken } = await importApiFresh();
    await expect(loadDaemonToken()).resolves.toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('keeps other fragment params while scrubbing only the token', async () => {
    setHash('#token=tok-first&view=world');
    await importApiFresh();
    expect(window.location.hash).toBe('#view=world');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
