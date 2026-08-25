/**
 * api.createSession — memory-wing seam. `projectId` is the CANONICAL project
 * id (`project:<slug>`) of the project open/known when a chat session is
 * created; the daemon's `POST /api/sessions` (`CreateSessionRequest`) reads it
 * to scope memories written in that session to the right project "wing".
 *
 * Pins the exact request-body contract: `projectId` rides the body when a
 * project is known, and is entirely ABSENT (not `null`, not empty string)
 * when no project is open — the UI must never invent one.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from './api';

function mockFetchOk(body: unknown = { id: 'sess-1' }) {
  return vi.fn(async () => ({
    ok: true,
    status: 200,
    json: async () => body,
    text: async () => JSON.stringify(body),
  })) as unknown as typeof fetch;
}

function sentBody(fetchMock: ReturnType<typeof mockFetchOk>): Record<string, unknown> {
  const call = (fetchMock as unknown as { mock: { calls: unknown[][] } }).mock.calls[0];
  const init = call[1] as RequestInit;
  return JSON.parse(init.body as string);
}

describe('api.createSession', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('includes projectId in the POST body when a project is open', async () => {
    const fetchMock = mockFetchOk();
    vi.stubGlobal('fetch', fetchMock);

    await api.createSession(undefined, 'project:permagent');

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(sentBody(fetchMock)).toEqual({ projectId: 'project:permagent' });
  });

  it('omits projectId entirely when no project is open (never invented)', async () => {
    const fetchMock = mockFetchOk();
    vi.stubGlobal('fetch', fetchMock);

    await api.createSession();

    const body = sentBody(fetchMock);
    expect(body).not.toHaveProperty('projectId');
    expect(body).toEqual({});
  });

  it('sends workingDir and projectId together when both are known', async () => {
    const fetchMock = mockFetchOk();
    vi.stubGlobal('fetch', fetchMock);

    await api.createSession('/tmp/work', 'project:permagent');

    expect(sentBody(fetchMock)).toEqual({
      workingDir: '/tmp/work',
      projectId: 'project:permagent',
    });
  });
});
