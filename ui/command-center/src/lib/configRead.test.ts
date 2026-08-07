/**
 * The `/config/read` wire contract.
 *
 * `ConfigValueResponse` is an UNTAGGED enum: a non-secret key answers with the
 * bare JSON value, a secret answers with `{ maskedValue }`. There is no
 * envelope with a `value` key — nothing sends that shape.
 *
 * The client typed it as `{ value?, maskedValue? }` anyway, so every reader
 * unwrapped `.value` and got `undefined`. The Guard's Settings toggle saved
 * `strix_enabled: true` correctly and then read back OFF forever, in Settings
 * and in the World HUD both: a setting that could be turned on but never
 * observed to be on. Nothing caught it because nothing asserted the shape.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const fetchMock = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('localStorage', {
    getItem: () => null,
    setItem: () => undefined,
    removeItem: () => undefined,
  });
});
afterEach(() => vi.unstubAllGlobals());

function respond(body: unknown) {
  fetchMock.mockResolvedValue({
    ok: true,
    status: 200,
    headers: { get: () => 'application/json' },
    text: async () => JSON.stringify(body),
    json: async () => body,
  });
}

/** The body the client POSTed on its most recent call. */
function sentBody(): Record<string, unknown> {
  const init = fetchMock.mock.calls.at(-1)?.[1] as RequestInit;
  return JSON.parse(String(init.body));
}

describe('readConfig — non-secret keys', () => {
  it('returns the bare value, not an envelope', async () => {
    const { api } = await import('./api');
    respond(true);
    await expect(api.readConfig('strix_enabled')).resolves.toBe(true);
  });

  it('round-trips the shapes config actually holds', async () => {
    const { api } = await import('./api');
    for (const value of [true, false, 24, 'anthropic', null]) {
      respond(value);
      await expect(api.readConfig('k')).resolves.toEqual(value);
    }
  });

  it('asks for the non-secret form', async () => {
    const { api } = await import('./api');
    respond(true);
    await api.readConfig('strix_enabled');
    expect(sentBody()).toEqual({ key: 'strix_enabled', is_secret: false });
  });

  it('a value of true is distinguishable from an unset key', async () => {
    // The whole failure: both used to read as "off".
    const { api } = await import('./api');
    respond(true);
    const on = await api.readConfig('strix_enabled');
    respond(null);
    const unset = await api.readConfig('strix_enabled');
    expect(on === true).toBe(true);
    expect(unset === true).toBe(false);
  });
});

describe('readSecretConfig — secret keys', () => {
  it('asks for the secret form and returns the masked envelope', async () => {
    const { api } = await import('./api');
    respond({ maskedValue: 'sk-…9f2' });
    const r = await api.readSecretConfig('TAVILY_API_KEY');
    expect(sentBody()).toEqual({ key: 'TAVILY_API_KEY', is_secret: true });
    expect(r?.maskedValue).toBe('sk-…9f2');
  });

  it('answers null for a key that was never set', async () => {
    const { api } = await import('./api');
    respond(null);
    await expect(api.readSecretConfig('NOPE')).resolves.toBeNull();
  });
});
