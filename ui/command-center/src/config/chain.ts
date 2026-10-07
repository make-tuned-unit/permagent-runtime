// On-chain identity is OPT-IN. Nothing here names a specific agent, wallet or
// contract: the user supplies their own through Settings -> Persona
// (config keys `identity_chitin_slug` and `identity_chain`), both empty by
// default. With neither set, no identity request is ever made.

export const BASE_RPC_URL =
  (typeof import.meta !== 'undefined' && (import.meta.env?.VITE_BASE_RPC_URL as string | undefined)) ||
  'https://mainnet.base.org';

/** Config key holding the user's chitin.id slug (string, empty = not linked). */
export const IDENTITY_CHITIN_SLUG_KEY = 'identity_chitin_slug';
/** Config key holding the user's contract set (object, empty = not linked). */
export const IDENTITY_CHAIN_KEY = 'identity_chain';

/** The user's own on-chain identity contracts, as stored in `identity_chain`. */
export interface IdentityChainConfig {
  sbtContract: string;
  passportContract: string;
  owner: string;
  sbtTokenId: bigint;
  passportTokenId: bigint;
}

const ADDRESS_RE = /^0x[0-9a-fA-F]{40}$/;

/** The stored (wire) shape: snake_case strings so it round-trips through JSON. */
export interface IdentityChainWire {
  sbt_contract?: string;
  passport_contract?: string;
  owner?: string;
  sbt_token_id?: string;
  passport_token_id?: string;
}

/**
 * Narrow a raw config value to a usable contract set, or null when it is unset
 * or incomplete. All five fields are required: a half-filled form must read as
 * "not linked" rather than querying a partial set.
 */
export function parseIdentityChain(raw: unknown): IdentityChainConfig | null {
  if (!raw || typeof raw !== 'object') return null;
  const w = raw as IdentityChainWire;
  const sbt = typeof w.sbt_contract === 'string' ? w.sbt_contract.trim() : '';
  const passport = typeof w.passport_contract === 'string' ? w.passport_contract.trim() : '';
  const owner = typeof w.owner === 'string' ? w.owner.trim() : '';
  if (!ADDRESS_RE.test(sbt) || !ADDRESS_RE.test(passport) || !ADDRESS_RE.test(owner)) return null;
  const toId = (v: unknown): bigint | null => {
    if (typeof v !== 'string' && typeof v !== 'number') return null;
    const s = String(v).trim();
    return /^\d+$/.test(s) ? BigInt(s) : null;
  };
  const sbtId = toId(w.sbt_token_id);
  const passportId = toId(w.passport_token_id);
  if (sbtId === null || passportId === null) return null;
  return { sbtContract: sbt, passportContract: passport, owner, sbtTokenId: sbtId, passportTokenId: passportId };
}

// ── CSP allowlist for RPC origins ────────────────────────────────────
// These serve the optional identity feature only, and must stay in sync with
// connect-src in the Tauri config:
//   ui/desktop/src-tauri/tauri.conf.json

const CSP_ALLOWED_RPC_ORIGINS = [
  'https://mainnet.base.org',
] as const;

function resolveOrigin(url: string): string {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.host}`;
  } catch {
    return url;
  }
}

const _rpcOrigin = resolveOrigin(BASE_RPC_URL);

export const RPC_ORIGIN_ALLOWED = CSP_ALLOWED_RPC_ORIGINS.some(
  (allowed) => resolveOrigin(allowed) === _rpcOrigin,
);

if (!RPC_ORIGIN_ALLOWED) {
  // Only matters once a user opts in; warn rather than throw so an unrelated
  // misconfigured env var cannot break a build that never uses the feature.
  console.error(
    `[chain] VITE_BASE_RPC_URL origin "${_rpcOrigin}" is not in the CSP allowlist. ` +
    `Allowed: ${CSP_ALLOWED_RPC_ORIGINS.join(', ')}. Update connect-src in ui/desktop/src-tauri/tauri.conf.json.`,
  );
}
