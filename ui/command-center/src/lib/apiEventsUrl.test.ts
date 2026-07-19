/**
 * Pins the REAL api.sessionEventsUrl contract (P1 + C1/C2): the resume cursor
 * rides a `last_event_id` query param — the exact name the daemon's SSE
 * endpoint parses as its Last-Event-ID header equivalent — and the daemon
 * token rides `?token=` the same way (EventSource cannot set an Authorization
 * header any more than it can Last-Event-ID). The store tests mock this
 * module, so this file imports the real one.
 *
 * Async since the auth plane: the URL builders await loadDaemonToken(). In
 * this node env there is no window/localStorage, so the default-import tests
 * pin the tokenless shapes; the paired-device block stubs a browser context
 * around a fresh module load to pin the token attachment itself.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, eventsWsUrl } from './api';

describe('api.sessionEventsUrl (no daemon token available)', () => {
  it('builds the plain events URL when no cursor is recorded', async () => {
    await expect(api.sessionEventsUrl('sess-1')).resolves.toBe('/sessions/sess-1/events');
    await expect(api.sessionEventsUrl('sess-1', null)).resolves.toBe('/sessions/sess-1/events');
  });

  it('appends ?last_event_id=<cursor> for a resume', async () => {
    await expect(api.sessionEventsUrl('sess-1', '42')).resolves.toBe(
      '/sessions/sess-1/events?last_event_id=42',
    );
  });

  it('URI-encodes both the session id and the cursor', async () => {
    await expect(api.sessionEventsUrl('a/b', '4 2')).resolves.toBe(
      '/sessions/a%2Fb/events?last_event_id=4+2',
    );
  });
});

describe('token attachment (paired browser device)', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  /** Fresh api module under a stubbed browser context holding a paired token
   *  (the #366 localStorage path — the same store loadDaemonToken reads). */
  async function apiWithPairedToken(token: string) {
    const storage = new Map<string, string>([['permagent-daemon-token', token]]);
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => void storage.set(k, v),
    });
    vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
    vi.resetModules();
    return import('./api');
  }

  it('sessionEventsUrl rides the daemon token on ?token= alongside the cursor', async () => {
    const fresh = await apiWithPairedToken('tok-123');
    await expect(fresh.api.sessionEventsUrl('sess-1', '42')).resolves.toBe(
      '/sessions/sess-1/events?last_event_id=42&token=tok-123',
    );
    await expect(fresh.api.sessionEventsUrl('sess-1')).resolves.toBe(
      '/sessions/sess-1/events?token=tok-123',
    );
  });

  it('eventsWsUrl attaches the token for the global /events WebSocket', async () => {
    const fresh = await apiWithPairedToken('tok 456');
    await expect(fresh.eventsWsUrl()).resolves.toBe('/events?token=tok%20456');
  });

  it('eventsWsUrl falls back to the bare URL when no token exists', async () => {
    await expect(eventsWsUrl()).resolves.toBe('/events');
  });
});
