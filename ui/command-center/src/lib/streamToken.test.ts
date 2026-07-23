import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.resetModules();
});

function stubPairedBrowser(token: string) {
  const storage = new Map<string, string>([['permagent-daemon-token', token]]);
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
  });
  vi.stubGlobal('window', { location: { hash: '', pathname: '/', search: '' } });
}

describe('getStreamToken', () => {
  it('defaults off and returns the existing long-lived token without fetching', async () => {
    stubPairedBrowser('daemon-token');
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);

    const { getStreamToken } = await import('./streamToken');

    await expect(getStreamToken()).resolves.toBe('daemon-token');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('mints a scoped token when the flag is on', async () => {
    vi.stubEnv('PERMAGENT_SHORTLIVED_STREAM_TOKEN', '1');
    stubPairedBrowser('daemon-token');
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ token: 'scoped-token', expires_in_secs: 120 }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const { getStreamToken } = await import('./streamToken');

    await expect(getStreamToken()).resolves.toBe('scoped-token');
    expect(fetchMock).toHaveBeenCalledWith('/sse-token', {
      method: 'POST',
      headers: { Authorization: 'Bearer daemon-token' },
    });
  });

  it('falls back to the long-lived token when minting fails', async () => {
    vi.stubEnv('PERMAGENT_SHORTLIVED_STREAM_TOKEN', '1');
    stubPairedBrowser('daemon-token');
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')));

    const { getStreamToken } = await import('./streamToken');

    await expect(getStreamToken()).resolves.toBe('daemon-token');
  });
});
