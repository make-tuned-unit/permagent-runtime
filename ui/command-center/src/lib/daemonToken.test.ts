/**
 * Pins the daemon-token auth plane against the defect it used to have: a bare
 * `.catch(() => null)` whose resolved-null promise stayed memoized in
 * `_daemonTokenPromise`, so ONE early failure left the client unauthenticated
 * for the life of the window — no log, no retry, no UI. Every request 401s,
 * including the `/events` stream the agent's replies ride, and the agent goes
 * silent with nothing to show for it.
 *
 * Each test loads a fresh copy of the module (the token cache is module state).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/** Load a fresh api module inside a stubbed Tauri context whose
 *  `get_daemon_token` invoke behaves as `impl` says. */
async function apiInTauri(impl: () => Promise<string>) {
  vi.resetModules();
  vi.doMock('@tauri-apps/api/core', () => ({ invoke: () => impl() }));
  vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: { hash: '', pathname: '/', search: '' } });
  const [api, auth] = await Promise.all([import('./api'), import('./daemonAuth')]);
  return { ...api, getDaemonTokenError: auth.getDaemonTokenError, onDaemonTokenError: auth.onDaemonTokenError };
}

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.doUnmock('@tauri-apps/api/core');
  vi.resetModules();
  vi.restoreAllMocks();
});

describe('loadDaemonToken (Tauri)', () => {
  it('retries after a failure instead of memoizing null forever', async () => {
    let calls = 0;
    const api = await apiInTauri(async () => {
      calls++;
      if (calls === 1) throw new Error('IPC not ready');
      return 'tok-live';
    });

    // The module kicks off an eager load at import; let it settle so the
    // assertions below are about *subsequent* callers, as in the real app.
    await api.loadDaemonToken().catch(() => null);

    // Whatever the eager load did, a later caller must still be able to get a
    // real token — that is exactly what the memoized null used to prevent.
    await expect(api.loadDaemonToken()).resolves.toBe('tok-live');
    expect(api.getDaemonTokenError()).toBeNull();
  });

  it('reports the failure instead of swallowing it', async () => {
    const api = await apiInTauri(async () => { throw new Error('HOME not set'); });

    await expect(api.loadDaemonToken()).resolves.toBeNull();
    expect(api.getDaemonTokenError()).toMatch(/HOME not set/);
    expect(console.error).toHaveBeenCalled();
  });

  it('notifies subscribers so the UI can show the dead-agent state', async () => {
    const api = await apiInTauri(async () => { throw new Error('boom'); });
    const seen: (string | null)[] = [];
    api.onDaemonTokenError(m => seen.push(m));

    await api.loadDaemonToken();

    expect(seen.some(m => typeof m === 'string' && /boom/.test(m))).toBe(true);
  });

  it('treats an empty token as a failure, not a credential', async () => {
    const api = await apiInTauri(async () => '');

    await expect(api.loadDaemonToken()).resolves.toBeNull();
    expect(api.getDaemonTokenError()).toMatch(/empty token/);
  });

  it('clears the error once a token finally loads', async () => {
    let calls = 0;
    const api = await apiInTauri(async () => {
      calls++;
      if (calls === 1) throw new Error('not ready');
      return 'tok-late';
    });

    await api.loadDaemonToken().catch(() => null);
    await api.loadDaemonToken();

    expect(api.getDaemonTokenError()).toBeNull();
  });
});

describe('eventsWsUrl', () => {
  it('carries the token on ?token= once the load succeeds', async () => {
    const api = await apiInTauri(async () => 'tok-ws');
    await api.loadDaemonToken();

    await expect(api.eventsWsUrl()).resolves.toContain('/events?token=tok-ws');
  });

  it('recovers the token on a later reconnect after an early failure', async () => {
    let calls = 0;
    const api = await apiInTauri(async () => {
      calls++;
      if (calls === 1) throw new Error('IPC not ready');
      return 'tok-ws';
    });

    // First attempt (the eager import-time load) fails; the reconnect that
    // follows must not inherit a permanently tokenless URL.
    await api.loadDaemonToken().catch(() => null);

    await expect(api.eventsWsUrl()).resolves.toContain('/events?token=tok-ws');
  });
});

describe('a lone 401 beside working routes is not a dead credential', () => {
  it('knows whether an authenticated request succeeded recently', async () => {
    const { noteAuthenticatedSuccess, recentlyAuthenticated, resetDaemonTokenError } = await import('./daemonAuth');
    resetDaemonTokenError();
    expect(recentlyAuthenticated(30_000, 1_000_000)).toBe(false);
    noteAuthenticatedSuccess(1_000_000);
    expect(recentlyAuthenticated(30_000, 1_020_000)).toBe(true);
    // Older than the window: a 401 now could be the credential dying.
    expect(recentlyAuthenticated(30_000, 1_031_000)).toBe(false);
  });
});
