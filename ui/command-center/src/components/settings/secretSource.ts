import type { SecretBackendStatus, SecretKeySource } from '../../lib/api';

/**
 * Pure logic behind the "where does this key come from?" control in the
 * provider modal.
 *
 * It lives outside the component for the same reason everything else in this
 * folder does: the tests here are pure-logic (no react-testing-library), so the
 * rules that matter — which specs are valid, what a key's source actually is,
 * and when a manager may be offered as ready — are asserted directly instead of
 * through a rendered tree.
 */

/** The choices the picker offers. `file` is intentionally absent: it exists in
 *  the daemon for installs with the keyring disabled, but offering "write my
 *  API keys to a plaintext-ish file" as a one-click option in Settings is not a
 *  choice we want to make easy. */
export type SourceKind = 'keychain' | 'onepassword' | 'bitwarden';

export const SOURCE_KINDS: Array<{ kind: SourceKind; label: string; backendId?: string }> = [
  { kind: 'keychain', label: 'macOS Keychain' },
  { kind: 'onepassword', label: '1Password', backendId: 'onepassword' },
  { kind: 'bitwarden', label: 'Bitwarden', backendId: 'bitwarden' },
];

/** Placeholder + help text per kind, so the reference field says what it wants. */
export const REFERENCE_HINTS: Record<SourceKind, { placeholder: string; help: string }> = {
  keychain: { placeholder: '', help: '' },
  onepassword: {
    placeholder: 'op://Personal/OpenAI/credential',
    help: 'Copy the secret reference from 1Password (right-click a field → Copy Secret Reference).',
  },
  bitwarden: {
    placeholder: 'bw://OpenAI/password',
    help: 'bw://<item> uses the password field; add /<field> for a custom field.',
  },
};

/**
 * Which kind a stored entry represents.
 *
 * `undefined` means the key has no explicit source, so it takes the daemon's
 * default. We show that as the keychain unless the daemon says otherwise —
 * never as "unknown", because a key whose source the UI cannot name reads as
 * broken when it is in fact perfectly ordinary.
 */
export function kindForKey(
  entry: SecretKeySource | undefined,
  defaultSource: string,
): SourceKind | 'file' | 'invalid' {
  const raw = entry ? entry.kind : normalizeDefault(defaultSource);
  if (raw === 'keychain' || raw === 'onepassword' || raw === 'bitwarden' || raw === 'file') return raw;
  return 'invalid';
}

function normalizeDefault(defaultSource: string): string {
  const d = (defaultSource || '').trim().toLowerCase();
  if (d === 'file') return 'file';
  // 'keyring' is the accepted alias on the daemon side; collapse it here so the
  // picker does not show a second name for the same thing.
  return 'keychain';
}

export function findKeySource(
  keys: SecretKeySource[] | undefined,
  key: string,
): SecretKeySource | undefined {
  if (!keys || !key) return undefined;
  // The daemon matches key names case-insensitively (providers read
  // OPENAI_API_KEY, config_value! reads openai_api_key). Matching exactly here
  // would show "macOS Keychain" for a key that is really on 1Password.
  return keys.find(k => k.key.toLowerCase() === key.toLowerCase());
}

/** The one-line label shown next to a key: "macOS Keychain", "1Password", … */
export function sourceLabel(
  entry: SecretKeySource | undefined,
  defaultSource: string,
): string {
  if (entry) return entry.label;
  return normalizeDefault(defaultSource) === 'file' ? 'Local secrets file' : 'macOS Keychain';
}

/**
 * Build the spec string the daemon persists, or report why we cannot.
 *
 * Validation mirrors `SecretSource::parse` so a typo is caught before it is
 * saved rather than at the next chat turn. It is deliberately a duplicate of
 * the daemon rule and not the authority: the daemon re-validates and is the one
 * that can refuse.
 */
export function buildSpec(
  kind: SourceKind,
  reference: string,
): { spec: string | null } | { error: string } {
  if (kind === 'keychain') return { spec: null };

  const ref = (reference || '').trim();
  if (!ref) return { error: 'Enter a reference.' };

  if (kind === 'onepassword') {
    if (!ref.startsWith('op://')) return { error: 'A 1Password reference starts with op://.' };
    const segments = ref.slice('op://'.length).split('/');
    if (segments.length < 3 || segments.some(s => s.length === 0)) {
      return { error: 'Needs vault, item and field: op://Vault/Item/field.' };
    }
    return { spec: ref };
  }

  // Bitwarden. `bw://Item` is legal (it means the password field), so only the
  // empty item and an empty trailing field are rejected.
  if (!ref.startsWith('bw://')) return { error: 'A Bitwarden reference starts with bw://.' };
  const rest = ref.slice('bw://'.length);
  const [item, ...fieldParts] = rest.split('/');
  if (!item.trim()) return { error: 'Needs an item: bw://Item.' };
  if (fieldParts.length > 0 && !fieldParts.join('/').trim()) {
    return { error: "Drop the trailing '/' to use the password field." };
  }
  return { spec: ref };
}

/**
 * Whether a manager may be OFFERED as ready.
 *
 * Installed is not enough. A locked Bitwarden vault and a signed-out `op` both
 * exist on PATH and fail every read; presenting either as available is the
 * false green light this whole feature is meant to remove. Onboarding uses the
 * same predicate, which is why it lives here and not in a component.
 */
export function isBackendReady(
  backends: SecretBackendStatus[] | undefined,
  backendId: string,
): boolean {
  const b = backends?.find(x => x.id === backendId);
  return !!b && b.installed && b.signedIn;
}

/**
 * The sentence shown under a manager option when it cannot be used, or null
 * when it can. Never blocks the choice — a user may be setting up a reference
 * before signing in — it just refuses to pretend.
 */
export function backendBlockedReason(
  backends: SecretBackendStatus[] | undefined,
  backendId: string,
): string | null {
  const b = backends?.find(x => x.id === backendId);
  if (!b) return 'Not detected on this machine.';
  if (!b.installed) return b.detail || 'Not installed.';
  if (!b.signedIn) return b.detail || 'Installed, but not signed in.';
  return null;
}

/**
 * The single manager onboarding should suggest, or null.
 *
 * "Offers a detected, signed-in manager; never blocks on one" — so this returns
 * a suggestion only when a manager is genuinely usable right now, and callers
 * treat null as "carry on with the keychain".
 */
export function suggestedManager(
  backends: SecretBackendStatus[] | undefined,
): SecretBackendStatus | null {
  return backends?.find(b => b.installed && b.signedIn) ?? null;
}

/**
 * What to say about a key whose source is external.
 *
 * A `resolves === false` row is the honest-failure case the acceptance test
 * describes: the reference is configured, the manager cannot answer, and the
 * user needs to be told that rather than shown a generic provider error later.
 */
export function keyStatusMessage(entry: SecretKeySource | undefined): string | null {
  if (!entry) return null;
  if (entry.kind === 'invalid') {
    return entry.error ?? 'This key has an invalid source and cannot be read.';
  }
  if (entry.resolves === false) {
    return entry.error ?? `Couldn't read this key from ${entry.label}.`;
  }
  return null;
}
