/**
 * api.getSessions — pins the actual request URL, not just the client-side
 * classifier.
 *
 * REVIEW_CONFIRMED #11: the daemon's `GET /api/sessions` defaults to
 * `[User, Scheduled]` and only returns `SubAgent` sessions when the caller
 * opts in with `?types=...` (crates/goose-server/src/routes/session.rs,
 * crates/goose/src/session/session_manager.rs `DEFAULT_LIST_TYPES`). Every
 * earlier test of History's "Agent runs" tab seeded a `sub_agent` row
 * directly into the store, which proves `classifySession()` buckets it
 * correctly but never proves the real fetch ever asks the daemon for one.
 * This test asserts the URL `api.getSessions()` actually sends.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from './api';

function mockFetchOk(body: unknown = { sessions: [] }) {
  return vi.fn(async () => ({
    ok: true,
    status: 200,
    json: async () => body,
    text: async () => JSON.stringify(body),
  })) as unknown as typeof fetch;
}

describe('api.getSessions', () => {
  beforeEach(() => {
    vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
    vi.stubGlobal('localStorage', { getItem: () => 'tok-test', setItem: () => undefined });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('requests sub_agent sessions explicitly, alongside the previous default types', async () => {
    const fetchMock = mockFetchOk();
    vi.stubGlobal('fetch', fetchMock);

    await api.getSessions();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const url = (fetchMock as unknown as { mock: { calls: unknown[][] } }).mock.calls[0][0] as string;
    expect(url).toContain('/api/sessions?types=');
    const query = new URL(url, 'http://localhost').searchParams.get('types');
    const types = (query ?? '').split(',');
    expect(types).toEqual(expect.arrayContaining(['user', 'scheduled', 'sub_agent']));
  });
});
