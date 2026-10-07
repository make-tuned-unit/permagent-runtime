/**
 * Identity hydrate: opt-in, and honest when unreachable.
 *
 * On-chain identity is OPT-IN. With no `identity_chitin_slug` / `identity_chain`
 * configured, hydrate() must make no request at all and report `linked: false`.
 * When linked but unreachable, constants must never masquerade as a verified
 * soul (2026-07 wiring audit).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const readConfig = vi.hoisted(() => vi.fn());
vi.mock('../lib/api', () => ({ api: { readConfig } }));

// Force the on-chain read to bail at the CSP allowlist guard: no viem call, no
// network, no retry backoff.
vi.mock('../config/chain', async (orig) => ({
  ...(await orig<typeof import('../config/chain')>()),
  RPC_ORIGIN_ALLOWED: false,
}));

import { hydrate } from './identity';

const CHAIN = {
  sbt_contract: '0x' + '1'.repeat(40),
  passport_contract: '0x' + '2'.repeat(40),
  owner: '0x' + '3'.repeat(40),
  sbt_token_id: '7',
  passport_token_id: '9',
};

let fetchMock: ReturnType<typeof vi.fn<any[], Promise<Response>>>;
beforeEach(() => {
  readConfig.mockReset();
  fetchMock = vi.fn<any[], Promise<Response>>(async () => new Response('down', { status: 503 }));
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => { vi.unstubAllGlobals(); });

describe('identity hydrate — opt-in', () => {
  it('makes no request and reports unlinked when nothing is configured', async () => {
    readConfig.mockResolvedValue(null);
    const res = await hydrate(true);
    expect(res.linked).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(res.errors).toEqual([]);
    expect(res.data.did).toBe('');
    expect(res.data.owner).toBe('');
    expect(res.data.passportId).toBe(0);
  });

  it('treats an incomplete contract set as unlinked', async () => {
    readConfig.mockImplementation(async (k: string) => (k === 'identity_chain' ? { ...CHAIN, owner: 'nope' } : null));
    expect((await hydrate(true)).linked).toBe(false);
  });

  it('fetches only the configured slug', async () => {
    readConfig.mockImplementation(async (k: string) => (k === 'identity_chitin_slug' ? 'my-agent' : null));
    await hydrate(true);
    const urls = fetchMock.mock.calls.map((c) => String(c[0]));
    expect(urls.length).toBeGreaterThan(0);
    expect(urls.every((u) => u.startsWith('https://chitin.id/my-agent'))).toBe(true);
  });
});

describe('identity hydrate — honest fallback when unreachable', () => {
  it('does not present a verified soul when neither source returns data', async () => {
    readConfig.mockImplementation(async (k: string) =>
      k === 'identity_chitin_slug' ? 'my-agent' : k === 'identity_chain' ? CHAIN : null);
    const res = await hydrate(true);

    expect(res.linked).toBe(true);
    expect(res.chainReachable).toBe(false);
    expect(res.errors.length).toBeGreaterThan(0);

    expect(res.data.soulValid).toBe(false);
    expect(res.data.status).toBe('unverified');
    expect(res.data.lastVerifiedAt).toBeNull();
  });
});
