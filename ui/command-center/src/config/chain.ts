export const BASE_RPC_URL =
  (typeof import.meta !== 'undefined' && (import.meta.env?.VITE_BASE_RPC_URL as string | undefined)) ||
  'https://mainnet.base.org';

export const SBT_CONTRACT = '0x4DB94aD31BC202831A49Fd9a2Fa354583002F894' as const;
export const PASSPORT_CONTRACT = '0x8004A169FB4a3325136EB29fA0ceB6D2e539a432' as const;
export const AGENT_OWNER = '0x95Ab1B24f8c0C70E59687f742C79F97a9277996f' as const;
export const SBT_TOKEN_ID = 54n;
export const PASSPORT_TOKEN_ID = 38105n;

// ── CSP allowlist for RPC origins ────────────────────────────────────
// These must stay in sync with connect-src in both tauri.conf.json files:
//   ui/desktop/src-tauri/tauri.conf.json
//   ui/command-center/src-tauri/tauri.conf.json

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
const _isDev = typeof import.meta !== 'undefined' && import.meta.env?.DEV === true;

export const RPC_ORIGIN_ALLOWED = CSP_ALLOWED_RPC_ORIGINS.some(
  (allowed) => resolveOrigin(allowed) === _rpcOrigin,
);

if (!RPC_ORIGIN_ALLOWED) {
  const msg =
    `[chain] VITE_BASE_RPC_URL origin "${_rpcOrigin}" is not in the CSP allowlist.\n` +
    `Allowed origins: ${CSP_ALLOWED_RPC_ORIGINS.join(', ')}\n` +
    `Update connect-src in:\n` +
    `  - ui/desktop/src-tauri/tauri.conf.json\n` +
    `  - ui/command-center/src-tauri/tauri.conf.json`;

  if (_isDev) {
    throw new Error(msg);
  } else {
    console.error(msg);
  }
}
