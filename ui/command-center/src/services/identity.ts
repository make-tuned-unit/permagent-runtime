import { createPublicClient, http, type Address } from 'viem';
import { base } from 'viem/chains';
import { api } from '../lib/api';
import {
  BASE_RPC_URL,
  RPC_ORIGIN_ALLOWED,
  IDENTITY_CHITIN_SLUG_KEY,
  IDENTITY_CHAIN_KEY,
  parseIdentityChain,
  type IdentityChainConfig,
} from '../config/chain';

// ── Opt-in config ────────────────────────────────────────────────────
// On-chain identity is OPT-IN: both keys are empty by default and with neither
// set hydrate() makes no network request at all.

export interface IdentityConfig {
  /** chitin.id slug (the path segment after chitin.id/), or '' when unset. */
  slug: string;
  chain: IdentityChainConfig | null;
}

const SLUG_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;

export async function loadIdentityConfig(): Promise<IdentityConfig> {
  const [rawSlug, rawChain] = await Promise.all([
    api.readConfig(IDENTITY_CHITIN_SLUG_KEY).catch(() => null),
    api.readConfig(IDENTITY_CHAIN_KEY).catch(() => null),
  ]);
  const slug = typeof rawSlug === 'string' ? rawSlug.trim() : '';
  return {
    slug: SLUG_RE.test(slug) ? slug : '',
    chain: parseIdentityChain(rawChain),
  };
}

/** `did:chitin:<slug>` -> `<slug>`; any other string is returned unchanged. */
export function chitinIdFromDid(did: string): string {
  const prefix = 'did:chitin:';
  return did.startsWith(prefix) ? did.slice(prefix.length) : did;
}

export function isIdentityLinked(cfg: IdentityConfig): boolean {
  return cfg.slug !== '' || cfg.chain !== null;
}

// ── Types ────────────────────────────────────────────────────────────

export interface IdentityState {
  did: string;
  name: string;
  avatarUrl: string;
  status: string;
  soulValid: boolean;
  lastVerifiedAt: string | null;
  alignmentScore: number | null;
  chronicleCount: number;
  bindingsCount: number;
  sbtId: number;
  passportId: number;
  /** The user's own SBT contract (from config), for explorer links. */
  sbtContract: string;
  owner: string;
  arweaveTxId: string;
  bornAt: string;
}

export interface IdentityResult {
  data: Partial<IdentityState>;
  error: string | null;
  source: 'chitin' | 'chain' | 'merged';
}

// ── Viem client ──────────────────────────────────────────────────────

// Built lazily: nothing is constructed (or contacted) until a user opts in.
function makeClient() {
  return createPublicClient({ chain: base, transport: http(BASE_RPC_URL) });
}
let _client: ReturnType<typeof makeClient> | undefined;
function getClient() {
  return (_client ??= makeClient());
}

// ── Minimal ABIs for ownerOf / tokenURI ──────────────────────────────

const ERC721_ABI = [
  {
    name: 'ownerOf',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ name: 'tokenId', type: 'uint256' }],
    outputs: [{ name: '', type: 'address' }],
  },
  {
    name: 'tokenURI',
    type: 'function',
    stateMutability: 'view',
    inputs: [{ name: 'tokenId', type: 'uint256' }],
    outputs: [{ name: '', type: 'string' }],
  },
] as const;

// ── Retry with exponential backoff ───────────────────────────────────

async function withRetry<T>(
  fn: () => Promise<T>,
  maxRetries = 3,
  baseDelay = 1000,
): Promise<T> {
  let lastError: unknown;
  for (let i = 0; i <= maxRetries; i++) {
    try {
      return await fn();
    } catch (err: unknown) {
      lastError = err;
      if (i < maxRetries) {
        const is429 = err instanceof Error && err.message.includes('429');
        const delay = baseDelay * Math.pow(2, i) * (is429 ? 2 : 1);
        await new Promise((r) => setTimeout(r, delay));
      }
    }
  }
  throw lastError;
}

// ── Cache ────────────────────────────────────────────────────────────

const CACHE_TTL_MS = 10 * 60 * 1000; // 10 minutes

interface CacheEntry {
  data: IdentityState;
  fetchedAt: number;
}

let _cache: CacheEntry | null = null;

function getCached(): IdentityState | null {
  if (!_cache) return null;
  if (Date.now() - _cache.fetchedAt > CACHE_TTL_MS) return null;
  return _cache.data;
}

function setCache(data: IdentityState): void {
  _cache = { data, fetchedAt: Date.now() };
}

// ── fetchFromChitin ──────────────────────────────────────────────────

export async function fetchFromChitin(slug: string): Promise<IdentityResult> {
  const enc = encodeURIComponent(slug);
  try {
    // Try JSON endpoint first
    const jsonRes = await withRetry(() =>
      fetch(`https://chitin.id/${enc}.json`, {
        signal: AbortSignal.timeout(10000),
      }),
    );
    if (jsonRes.ok) {
      const json = await jsonRes.json();
      return { data: parseChitinJson(json), error: null, source: 'chitin' };
    }

    // Fallback to HTML scrape
    const htmlRes = await withRetry(() =>
      fetch(`https://chitin.id/${enc}`, {
        signal: AbortSignal.timeout(10000),
      }),
    );
    if (!htmlRes.ok) {
      return { data: {}, error: `chitin.id returned ${htmlRes.status}`, source: 'chitin' };
    }
    const html = await htmlRes.text();
    return { data: parseChitinHtml(html), error: null, source: 'chitin' };
  } catch (err) {
    return {
      data: {},
      error: err instanceof Error ? err.message : 'Unknown chitin fetch error',
      source: 'chitin',
    };
  }
}

function parseChitinJson(json: Record<string, unknown>): Partial<IdentityState> {
  return {
    did: typeof json.did === 'string' ? json.did : undefined,
    name: typeof json.name === 'string' ? json.name : undefined,
    avatarUrl: typeof json.avatarUrl === 'string' || typeof json.avatar_url === 'string'
      ? (json.avatarUrl as string) ?? (json.avatar_url as string)
      : undefined,
    status: typeof json.status === 'string' ? json.status : undefined,
    soulValid: typeof json.soulValid === 'boolean' ? json.soulValid
      : typeof json.soul_valid === 'boolean' ? json.soul_valid : undefined,
    lastVerifiedAt: typeof json.lastVerifiedAt === 'string' ? json.lastVerifiedAt
      : typeof json.last_verified_at === 'string' ? json.last_verified_at : undefined,
    alignmentScore: typeof json.alignmentScore === 'number' ? json.alignmentScore
      : typeof json.alignment_score === 'number' ? json.alignment_score : undefined,
    chronicleCount: typeof json.chronicleCount === 'number' ? json.chronicleCount
      : typeof json.chronicle_count === 'number' ? json.chronicle_count : undefined,
    bindingsCount: typeof json.bindingsCount === 'number' ? json.bindingsCount
      : typeof json.bindings_count === 'number' ? json.bindings_count : undefined,
    sbtId: typeof json.sbtId === 'number' ? json.sbtId
      : typeof json.sbt_id === 'number' ? json.sbt_id : undefined,
    passportId: typeof json.passportId === 'number' ? json.passportId
      : typeof json.passport_id === 'number' ? json.passport_id : undefined,
    owner: typeof json.owner === 'string' ? json.owner : undefined,
    arweaveTxId: typeof json.arweaveTxId === 'string' ? json.arweaveTxId
      : typeof json.arweave_tx_id === 'string' ? json.arweave_tx_id : undefined,
    bornAt: typeof json.bornAt === 'string' ? json.bornAt
      : typeof json.born_at === 'string' ? json.born_at : undefined,
  };
}

function parseChitinHtml(html: string): Partial<IdentityState> {
  const partial: Partial<IdentityState> = {};
  const extract = (pattern: RegExp): string | undefined => {
    const m = html.match(pattern);
    return m?.[1]?.trim();
  };

  const did = extract(/did[:\s]*["']?(did:chitin:[^"'<\s]+)/i);
  if (did) partial.did = did;

  const name = extract(/<title[^>]*>([^<]+)/i) ?? extract(/name[:\s]*["']?([^"'<\n]+)/i);
  if (name) partial.name = name.replace(/\s*[-–|].*$/, '').trim();

  const avatar = extract(/avatar[^"]*["'](https:\/\/arweave\.net\/[^"']+)/i)
    ?? extract(/og:image[^"]*content=["'](https:\/\/arweave\.net\/[^"']+)/i);
  if (avatar) partial.avatarUrl = avatar;

  const status = extract(/status[:\s]*["']?(sealed|unsealed|pending)/i);
  if (status) partial.status = status;

  const owner = extract(/(0x[a-fA-F0-9]{40})/);
  if (owner) partial.owner = owner;

  const arweave = extract(/arweave\.net\/([a-zA-Z0-9_-]{43})/);
  if (arweave) partial.arweaveTxId = arweave;

  return partial;
}

// ── fetchFromChain ───────────────────────────────────────────────────

export async function fetchFromChain(cfg: IdentityChainConfig): Promise<IdentityResult> {
  const { sbtContract, passportContract, owner: agentOwner, sbtTokenId, passportTokenId } = cfg;
  if (!RPC_ORIGIN_ALLOWED) {
    return {
      data: {},
      error: 'RPC origin not in CSP allowlist — chain reads disabled',
      source: 'chain',
    };
  }

  try {
    const [sbtOwner, passportOwner, sbtUri] = await withRetry(() =>
      Promise.all([
        getClient().readContract({
          address: sbtContract as Address,
          abi: ERC721_ABI,
          functionName: 'ownerOf',
          args: [sbtTokenId],
        }),
        getClient().readContract({
          address: passportContract as Address,
          abi: ERC721_ABI,
          functionName: 'ownerOf',
          args: [passportTokenId],
        }),
        getClient().readContract({
          address: sbtContract as Address,
          abi: ERC721_ABI,
          functionName: 'tokenURI',
          args: [sbtTokenId],
        }),
      ]),
    );

    const ownerMatch =
      (sbtOwner as string).toLowerCase() === agentOwner.toLowerCase() &&
      (passportOwner as string).toLowerCase() === agentOwner.toLowerCase();

    const partial: Partial<IdentityState> = {
      owner: sbtOwner as string,
      soulValid: ownerMatch,
      sbtId: Number(sbtTokenId),
      passportId: Number(passportTokenId),
    };

    // Try to extract arweave tx from tokenURI
    if (typeof sbtUri === 'string') {
      const arMatch = sbtUri.match(/arweave\.net\/([a-zA-Z0-9_-]{43})/);
      if (arMatch) partial.arweaveTxId = arMatch[1];
    }

    return { data: partial, error: null, source: 'chain' };
  } catch (err) {
    return {
      data: {},
      error: err instanceof Error ? err.message : 'Unknown chain read error',
      source: 'chain',
    };
  }
}

// ── hydrate (merge) ──────────────────────────────────────────────────

// Fallback scaffolding — the merge base, and all that remains when BOTH the
// chitin service and the on-chain read fail. The verification-claiming fields
// (status, soulValid) MUST stay honest here: they are only ever true/sealed
// when a live source actually asserts them (fetchFromChain sets soulValid from
// a real ownerOf match; parseChitin* from a reported value). Previously these
// defaulted to sealed/true, so an offline machine displayed a fully "verified"
// soul built entirely from constants (2026-07 wiring audit — misleading state).
const DEFAULTS: IdentityState = {
  did: '',
  name: '',
  avatarUrl: '',
  status: 'unverified',
  soulValid: false,
  lastVerifiedAt: null,
  alignmentScore: null,
  chronicleCount: 0,
  bindingsCount: 0,
  sbtId: 0,
  passportId: 0,
  sbtContract: '',
  owner: '',
  arweaveTxId: '',
  bornAt: '',
};

export interface HydrateResult {
  data: IdentityState;
  /** False when the user has not linked an on-chain identity: nothing was
   *  fetched and `data` is the neutral scaffold, not an identity. */
  linked: boolean;
  errors: string[];
  chainReachable: boolean;
}

export async function hydrate(force = false): Promise<HydrateResult> {
  const cfg = await loadIdentityConfig();
  if (!isIdentityLinked(cfg)) {
    _cache = null;
    return { data: { ...DEFAULTS }, linked: false, errors: [], chainReachable: true };
  }

  if (!force) {
    const cached = getCached();
    if (cached) return { data: cached, linked: true, errors: [], chainReachable: true };
  }

  const none = (source: IdentityResult['source']): IdentityResult => ({ data: {}, error: null, source });
  const [chitinResult, chainResult] = await Promise.all([
    cfg.slug ? fetchFromChitin(cfg.slug) : Promise.resolve(none('chitin')),
    cfg.chain ? fetchFromChain(cfg.chain) : Promise.resolve(none('chain')),
  ]);

  const errors: string[] = [];
  if (chitinResult.error) errors.push(`chitin: ${chitinResult.error}`);
  if (chainResult.error) errors.push(`chain: ${chainResult.error}`);

  // Merge: start with defaults, overlay chitin, then chain wins on conflicts
  const merged: IdentityState = { ...DEFAULTS };

  for (const key of Object.keys(DEFAULTS) as (keyof IdentityState)[]) {
    const chitinVal = chitinResult.data[key];
    const chainVal = chainResult.data[key];
    // Chain wins on conflicts
    if (chainVal !== undefined && chainVal !== null) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (merged as any)[key] = chainVal;
    } else if (chitinVal !== undefined && chitinVal !== null) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (merged as any)[key] = chitinVal;
    }
  }

  merged.sbtContract = cfg.chain?.sbtContract ?? '';
  setCache(merged);
  return { data: merged, linked: true, errors, chainReachable: !chainResult.error };
}
