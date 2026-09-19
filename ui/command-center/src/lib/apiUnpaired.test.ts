/**
 * An unpaired browser must not send requests the daemon can only refuse.
 *
 * 2026-09-19: the standalone World page (localhost:5173/ui/forum.html) left
 * open in an unpaired Chrome tab sent a steady ~73 requests a minute to the
 * daemon through the Vite proxy — /api/agent/identity twice every 5s, /events
 * every few seconds, /api/agents + /permagent/skills + /permagent/skills/proposals
 * + /api/goals/active every 30s, /api/henry/status every 2s — and every one
 * came back 401 into daemon.err (804 in 11 minutes). Those pollers live in many
 * modules, but they all go through this client, so the rule lives here: no
 * credential, no request; the one exception is the daemon's public routes.
 *
 * What this proves: with no token, apiFetch / fetchAuthed never call `fetch`
 * for an authenticated route and fail with a 401-shaped error or Response;
 * public routes still go out; the /events URL is held back (no socket can be
 * opened) until a credential appears, and then carries it; the desktop app
 * (Tauri) path still sends as before. What it does not prove: real network
 * traffic in a real browser — `fetch` is a spy here — nor anything about
 * modules that call `fetch` themselves (see the node's open issues).
 */

import { afterEach, describe, expect, it, vi } from 'vitest';

const fetchSpy = vi.fn();

/** Fresh api module in a browser context whose localStorage holds `token`. */
async function apiInBrowser(token: string | null) {
  const storage = new Map<string, string>(token ? [['permagent-daemon-token', token]] : []);
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => void storage.set(k, v),
  });
  vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
  fetchSpy.mockReset().mockImplementation(async () => new Response('{}', { status: 200 }));
  vi.stubGlobal('fetch', fetchSpy);
  vi.resetModules();
  const [api, auth] = await Promise.all([import('./api'), import('./daemonAuth')]);
  auth.resetDaemonTokenError();
  return { ...api, auth, pair: (t: string) => storage.set('permagent-daemon-token', t) };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.doUnmock('@tauri-apps/api/core');
  vi.resetModules();
  vi.restoreAllMocks();
});

describe('unpaired browser: no credential, no request', () => {
  it('does not send the World pollers\' authenticated requests', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser(null);

    for (const endpoint of [
      '/api/agent/identity',
      '/api/agents',
      '/permagent/skills/proposals',
      '/api/goals/active',
      '/api/henry/status',
    ]) {
      await expect(m.apiFetch(endpoint)).rejects.toMatchObject({ status: 401 });
    }
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(m.auth.getDaemonTokenError()).toBe(m.auth.BROWSER_UNPAIRED);
  });

  it('still reaches the daemon\'s public routes', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser(null);
    await m.apiFetch('/status');
    await m.apiFetch('/api/version');
    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(String(fetchSpy.mock.calls[0][0])).toBe('/status');
  });

  it('fetchAuthed answers 401 locally instead of sending', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser(null);
    const resp = await m.api.fetchAuthed('/activity/recent');
    expect(resp.status).toBe(401);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('holds the /events URL back until paired, then attaches the token', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.useFakeTimers();
    const m = await apiInBrowser(null);

    let url: string | null = null;
    void m.eventsWsUrl().then(u => { url = u; });
    // Several recheck periods: before the fix the tokenless URL resolved at
    // once and every caller opened a socket the daemon refused.
    for (let i = 0; i < 4; i++) await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBeNull();

    m.pair('tok-paired');
    await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBe('/events?token=tok-paired');
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('sends normally, with the token, once paired', async () => {
    const m = await apiInBrowser('tok-1');
    await m.apiFetch('/api/agents');
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    const init = fetchSpy.mock.calls[0][1] as RequestInit;
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok-1');
  });
});

/**
 * A browser holding a credential the daemon no longer accepts (a revoked device
 * token, or one minted under a secret that has since changed) is unpaired in
 * every way that matters, but the no-token guard above never fired for it:
 * each poll sent, got 401, re-read the SAME token from localStorage, retried,
 * and got 401 again — the 2026-09-19 daemon.err flood, doubled.
 *
 * What this proves: the tab stops sending after a few consecutive refusals of
 * one token, its /events URL is held back too, a new pairing resumes both, and
 * one refusing route beside working ones never locks a tab out. What it does
 * not prove: a WebSocket's own 401 (the browser API does not expose the status,
 * so only the REST pollers can notice a dead token) or real network traffic.
 */
describe('browser with a credential the daemon refuses', () => {
  const refuseAll = () =>
    fetchSpy.mockImplementation(async () => new Response('{"message":"Unauthorized"}', { status: 401 }));

  it('stops sending once the stored token is refused every time', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser('tok-revoked');
    refuseAll();

    for (let i = 0; i < 10; i++) {
      await expect(m.apiFetch('/api/goals/active')).rejects.toMatchObject({ status: 401 });
    }
    // Before: 2 requests per call (the retry re-read the same token) = 20.
    // After: one per call until the third consecutive refusal, then none.
    expect(fetchSpy).toHaveBeenCalledTimes(3);
    expect(m.auth.getDaemonTokenError()).toMatch(/no longer accepts/);

    // Raw-Response callers are held back as well.
    const resp = await m.api.fetchAuthed('/activity/recent');
    expect(resp.status).toBe(401);
    expect(fetchSpy).toHaveBeenCalledTimes(3);
  });

  it('holds the /events URL back while the only token is dead, and resumes on a new pairing', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser('tok-revoked');
    refuseAll();
    for (let i = 0; i < 3; i++) {
      await m.apiFetch('/api/agents').catch(() => {});
    }

    vi.useFakeTimers();
    let url: string | null = null;
    void m.eventsWsUrl().then(u => { url = u; });
    for (let i = 0; i < 4; i++) await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBeNull();

    m.pair('tok-fresh');
    fetchSpy.mockImplementation(async () => new Response('{}', { status: 200 }));
    await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBe('/events?token=tok-fresh');

    const before = fetchSpy.mock.calls.length;
    await m.apiFetch('/api/agents');
    expect(fetchSpy).toHaveBeenCalledTimes(before + 1);
    const init = fetchSpy.mock.calls[before][1] as RequestInit;
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok-fresh');
    expect(m.auth.getDaemonTokenError()).toBeNull();
  });

  it('never locks out a working token because one route refuses it', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const m = await apiInBrowser('tok-1');
    // An older daemon: /api/agent/identity 401s, everything else answers.
    fetchSpy.mockImplementation(async (url: string) =>
      String(url).startsWith('/api/agent/identity')
        ? new Response('{}', { status: 401 })
        : new Response('{}', { status: 200 }),
    );
    for (let i = 0; i < 10; i++) {
      await m.apiFetch('/api/agent/identity').catch(() => {});
      await m.apiFetch('/api/goals/active');
    }
    expect(fetchSpy).toHaveBeenCalledTimes(20);
  });
});

describe('desktop app (Tauri) is unchanged', () => {
  it('still sends when the token read failed, so the 401 can raise the credential banner', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.doMock('@tauri-apps/api/core', () => ({ invoke: async () => { throw new Error('IPC down'); } }));
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: { hash: '', pathname: '/', search: '' } });
    fetchSpy.mockReset().mockImplementation(async () => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchSpy);
    vi.resetModules();
    const m = await import('./api');
    await m.apiFetch('/api/agents');
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });
});

/**
 * A pairing link that fails must say so. Opening a `#claim=` link whose
 * exchange fails — used or expired code, or the :5173 dev server answering 404
 * because it does not forward /pair — used to fall back silently to the plain
 * "not paired" message, the same words a browser that never tried gets. What
 * this proves: the published auth message names the failed claim and its
 * HTTP status, keeps saying so on later token loads, and a good claim clears
 * it and starts sending. It does not prove anything renders the message — that
 * is the page's job (`daemonAuthError` in the store).
 */
describe('#claim= pairing link outcome', () => {
  async function apiOpenedWithClaim(claimReply: Response) {
    const storage = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => void storage.set(k, v),
    });
    vi.stubGlobal('window', { location: { hash: '#claim=code-1', pathname: '/ui/forum.html', search: '' } });
    vi.stubGlobal('history', { replaceState: () => {} });
    fetchSpy.mockReset().mockImplementation(async (url: string) =>
      String(url).endsWith('/pair/claim') ? claimReply : new Response('{}', { status: 200 }),
    );
    vi.stubGlobal('fetch', fetchSpy);
    vi.resetModules();
    const auth = await import('./daemonAuth');
    auth.resetDaemonTokenError();
    const api = await import('./api');
    return { api, auth };
  }

  it('says the link was not redeemed, with the status, and keeps saying it', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { api, auth } = await apiOpenedWithClaim(new Response('Not Found', { status: 404 }));
    expect(await api.loadDaemonToken()).toBeNull();
    expect(auth.getDaemonTokenError()).toMatch(/pairing link could not be redeemed \(HTTP 404/);
    // A later load (the 5s re-check) must not quietly revert to "not paired".
    expect(await api.loadDaemonToken()).toBeNull();
    expect(auth.getDaemonTokenError()).toMatch(/pairing link could not be redeemed/);
    await api.apiFetch('/api/agents').catch(() => {});
    expect(fetchSpy.mock.calls.filter(([u]) => !String(u).endsWith('/pair/claim'))).toHaveLength(0);
  });

  it('a good claim clears the message and the tab starts sending', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { api, auth } = await apiOpenedWithClaim(
      new Response(JSON.stringify({ token: 'tok-claimed' }), { status: 200 }),
    );
    expect(await api.loadDaemonToken()).toBe('tok-claimed');
    expect(auth.getDaemonTokenError()).toBeNull();
    await api.apiFetch('/api/agents');
    expect(fetchSpy.mock.calls.some(([u]) => String(u) === '/api/agents')).toBe(true);
  });
});

/**
 * Pairing THIS tab by pasting the link. Settings → Devices builds its link on
 * the hub origin (`http://<hub>:3001/ui/#claim=…`), and a credential lives in
 * the localStorage of whichever origin redeemed it — so opening that link
 * never paired the localhost:5173 forum tab that showed "not paired".
 * `pairThisBrowser` redeems the pasted code through this page's own origin.
 * What this proves: the claim goes to this origin's POST /pair/claim, a good
 * claim stores the credential here and the tab's requests and /events carry it
 * with no reload; a refused claim says why and sends nothing; a dead token can
 * be replaced. What it does not prove: that the dev server forwards /pair (see
 * vite.config.ts) — `fetch` is a spy — or anything about rendering.
 */
describe('pairing this tab from a pasted link', () => {
  it('reads a hub pairing URL, a fragment, or a bare code', async () => {
    const m = await apiInBrowser(null);
    expect(m.parsePairingInput('http://hub.tail:3001/ui/#claim=0123456789abcdef0123456789abcdef'))
      .toEqual({ claim: '0123456789abcdef0123456789abcdef' });
    expect(m.parsePairingInput('  #token=tok-9 ')).toEqual({ token: 'tok-9' });
    expect(m.parsePairingInput('0123456789abcdef0123456789abcdef')).toEqual({ claim: '0123456789abcdef0123456789abcdef' });
    expect(m.parsePairingInput('http://hub:3001/ui/')).toBeNull();
    expect(m.parsePairingInput('hello there')).toBeNull();
  });

  it('redeems the pasted code on this origin and starts sending with the new credential', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser(null);
    fetchSpy.mockImplementation(async (url: string) =>
      String(url) === '/pair/claim'
        ? new Response(JSON.stringify({ token: 'tok-pasted' }), { status: 200 })
        : new Response('{}', { status: 200 }),
    );
    await m.apiFetch('/api/agents').catch(() => {});
    expect(fetchSpy).not.toHaveBeenCalled();

    vi.useFakeTimers();
    let url: string | null = null;
    void m.eventsWsUrl().then(u => { url = u; });
    await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBeNull();

    const result = await m.pairThisBrowser('http://hub.tail:3001/ui/#claim=abcdef0123456789');
    expect(result).toEqual({ paired: true, message: null });
    const [claimUrl, claimInit] = fetchSpy.mock.calls[0] as [string, RequestInit];
    expect(claimUrl).toBe('/pair/claim');
    expect(JSON.parse(String(claimInit.body))).toEqual({ code: 'abcdef0123456789' });
    expect(m.auth.getDaemonTokenError()).toBeNull();
    expect(localStorage.getItem('permagent-daemon-token')).toBe('tok-pasted');

    await vi.advanceTimersByTimeAsync(m.UNPAIRED_RECHECK_MS);
    expect(url).toBe('/events?token=tok-pasted');
    await m.apiFetch('/api/agents');
    const last = fetchSpy.mock.calls[fetchSpy.mock.calls.length - 1];
    expect(last[0]).toBe('/api/agents');
    expect(((last[1] as RequestInit).headers as Record<string, string>).Authorization).toBe('Bearer tok-pasted');
  });

  it('says why a refused code did not pair, and still sends nothing', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser(null);
    fetchSpy.mockImplementation(async () => new Response('gone', { status: 410 }));
    const result = await m.pairThisBrowser('#claim=used-code-123');
    expect(result.paired).toBe(false);
    expect(result.message).toMatch(/could not be redeemed \(HTTP 410/);
    expect(m.auth.getDaemonTokenError()).toMatch(/HTTP 410/);
    await m.apiFetch('/api/agents').catch(() => {});
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it('replaces a credential the daemon refused', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const m = await apiInBrowser('tok-revoked');
    fetchSpy.mockImplementation(async () => new Response('{}', { status: 401 }));
    for (let i = 0; i < 3; i++) await m.apiFetch('/api/agents').catch(() => {});
    expect(m.auth.getDaemonTokenError()).toMatch(/no longer accepts/);

    // Pasting the same dead token back does not pair.
    expect((await m.pairThisBrowser('#token=tok-revoked')).paired).toBe(false);
    fetchSpy.mockImplementation(async () => new Response('{}', { status: 200 }));
    expect(await m.pairThisBrowser('#token=tok-new')).toEqual({ paired: true, message: null });
    await m.apiFetch('/api/agents');
    const last = fetchSpy.mock.calls[fetchSpy.mock.calls.length - 1];
    expect(((last[1] as RequestInit).headers as Record<string, string>).Authorization).toBe('Bearer tok-new');
    expect(m.auth.getDaemonTokenError()).toBeNull();
  });
});
