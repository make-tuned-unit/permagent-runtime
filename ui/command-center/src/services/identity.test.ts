/**
 * Regression test for the identity hydrate honesty fix (2026-07 wiring audit).
 *
 * The World → Henry identity tab is driven by hydrate(): chitin.id + an on-chain
 * ownerOf read, merged over a constant DEFAULTS scaffold. The defect: DEFAULTS
 * asserted `soulValid: true` / `status: 'sealed'`, so a machine that could reach
 * NEITHER source still rendered a fully "verified, sealed" soul built entirely
 * from constants. This pins the fix — the verification-claiming fields must be
 * honest (unverified) when nothing was actually verified.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Force the on-chain read to bail at the CSP allowlist guard: no viem call, no
// network, no retry backoff — so the both-sources-unreachable path is exercised
// deterministically and fast.
vi.mock('../config/chain', () => ({
  BASE_RPC_URL: 'https://mainnet.base.org',
  RPC_ORIGIN_ALLOWED: false,
  SBT_CONTRACT: '0x4DB94aD31BC202831A49Fd9a2Fa354583002F894',
  PASSPORT_CONTRACT: '0x8004A169FB4a3325136EB29fA0ceB6D2e539a432',
  AGENT_OWNER: '0x95Ab1B24f8c0C70E59687f742C79F97a9277996f',
  SBT_TOKEN_ID: 54n,
  PASSPORT_TOKEN_ID: 38105n,
}));

import { hydrate } from './identity';

beforeEach(() => {
  // chitin.id resolves non-OK (a resolved response never throws, so withRetry
  // does not back off) — it contributes no real data, forcing the constant
  // fallback that the fix must keep honest.
  vi.stubGlobal('fetch', vi.fn(async () => new Response('down', { status: 503 })));
});

afterEach(() => { vi.unstubAllGlobals(); });

describe('identity hydrate — honest fallback when unreachable', () => {
  it('does not present a verified soul when neither source returns data', async () => {
    const res = await hydrate(true);

    // Both sources failed → not reachable, errors recorded.
    expect(res.chainReachable).toBe(false);
    expect(res.errors.length).toBeGreaterThan(0);

    // The core fix: constants must NOT masquerade as a verified identity.
    expect(res.data.soulValid).toBe(false);
    expect(res.data.status).toBe('unverified');
    expect(res.data.status).not.toBe('sealed');
    expect(res.data.lastVerifiedAt).toBeNull();
  });
});
