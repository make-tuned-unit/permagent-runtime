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
