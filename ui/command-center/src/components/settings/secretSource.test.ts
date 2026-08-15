import { describe, it, expect } from 'vitest';
import type { SecretBackendStatus, SecretKeySource } from '../../lib/api';
import {
  backendBlockedReason,
  buildSpec,
  findKeySource,
  isBackendReady,
  keyStatusMessage,
  kindForKey,
  sourceLabel,
  suggestedManager,
} from './secretSource';

function keyEntry(overrides: Partial<SecretKeySource> = {}): SecretKeySource {
  return {
    key: 'OPENAI_API_KEY',
    kind: 'onepassword',
    label: '1Password',
    reference: 'op://Personal/OpenAI/credential',
    resolves: true,
    error: null,
    ...overrides,
  };
}

function backend(overrides: Partial<SecretBackendStatus> = {}): SecretBackendStatus {
  return {
    id: 'onepassword',
    displayName: '1Password',
    installed: true,
    signedIn: true,
    detail: null,
    ...overrides,
  };
}

describe('buildSpec', () => {
  it('returns null for the keychain — the absence of an explicit source', () => {
    expect(buildSpec('keychain', 'ignored')).toEqual({ spec: null });
  });

  it('accepts a well-formed 1Password reference', () => {
    expect(buildSpec('onepassword', ' op://Personal/OpenAI/credential ')).toEqual({
      spec: 'op://Personal/OpenAI/credential',
    });
  });

  it('accepts a section-qualified 1Password reference', () => {
    expect(buildSpec('onepassword', 'op://Vault/Item/Section/field')).toEqual({
      spec: 'op://Vault/Item/Section/field',
    });
  });

  // The whole point of validating here: a two-segment reference looks right and
  // would be saved happily, then fail at the moment chat needs the key.
  it('rejects a reference missing the field', () => {
    expect(buildSpec('onepassword', 'op://Personal/OpenAI')).toEqual({
      error: 'Needs vault, item and field: op://Vault/Item/field.',
    });
  });

  it('rejects empty segments and wrong prefixes', () => {
    expect(buildSpec('onepassword', 'op://Personal//credential')).toHaveProperty('error');
    expect(buildSpec('onepassword', '1password://x/y/z')).toHaveProperty('error');
    expect(buildSpec('onepassword', '   ')).toEqual({ error: 'Enter a reference.' });
  });

  it('treats a bare bw:// item as the password field', () => {
    expect(buildSpec('bitwarden', 'bw://OpenAI')).toEqual({ spec: 'bw://OpenAI' });
    expect(buildSpec('bitwarden', 'bw://OpenAI/api-key')).toEqual({ spec: 'bw://OpenAI/api-key' });
  });

  it('rejects a bw:// reference with no item or a dangling field', () => {
    expect(buildSpec('bitwarden', 'bw://')).toHaveProperty('error');
    expect(buildSpec('bitwarden', 'bw://OpenAI/')).toHaveProperty('error');
  });
});

describe('findKeySource', () => {
  // Providers read OPENAI_API_KEY while config_value! reads openai_api_key; the
  // daemon matches either, so matching exactly here would show a key as being
  // on the keychain when it is really on 1Password.
  it('matches key names case-insensitively', () => {
    const keys = [keyEntry({ key: 'openai_api_key' })];
    expect(findKeySource(keys, 'OPENAI_API_KEY')?.kind).toBe('onepassword');
  });

  it('returns undefined for an unlisted key or missing list', () => {
    expect(findKeySource([keyEntry()], 'ANTHROPIC_API_KEY')).toBeUndefined();
    expect(findKeySource(undefined, 'OPENAI_API_KEY')).toBeUndefined();
  });
});

describe('kindForKey / sourceLabel', () => {
  it('falls back to the daemon default for keys with no explicit source', () => {
    expect(kindForKey(undefined, 'keychain')).toBe('keychain');
    expect(kindForKey(undefined, 'file')).toBe('file');
    expect(sourceLabel(undefined, 'keychain')).toBe('macOS Keychain');
    expect(sourceLabel(undefined, 'file')).toBe('Local secrets file');
  });

  it("collapses the daemon's 'keyring' alias onto one name", () => {
    expect(kindForKey(undefined, 'keyring')).toBe('keychain');
  });

  it('uses the entry when the key has an explicit source', () => {
    expect(kindForKey(keyEntry(), 'keychain')).toBe('onepassword');
    expect(sourceLabel(keyEntry(), 'keychain')).toBe('1Password');
  });

  it('surfaces an unparseable stored source as invalid rather than as the keychain', () => {
    expect(kindForKey(keyEntry({ kind: 'invalid' }), 'keychain')).toBe('invalid');
  });
});

describe('isBackendReady / backendBlockedReason', () => {
  // Installed is not enough. A locked vault is on PATH and fails every read.
  it('requires both installed AND signed in', () => {
    const backends = [backend({ signedIn: false, detail: 'Vault is locked.' })];
    expect(isBackendReady(backends, 'onepassword')).toBe(false);
    expect(backendBlockedReason(backends, 'onepassword')).toBe('Vault is locked.');
  });

  it('reports a ready backend as ready with no blocking reason', () => {
    const backends = [backend()];
    expect(isBackendReady(backends, 'onepassword')).toBe(true);
    expect(backendBlockedReason(backends, 'onepassword')).toBeNull();
  });

  it('treats an undetected backend as not ready', () => {
    expect(isBackendReady([], 'bitwarden')).toBe(false);
    expect(backendBlockedReason([], 'bitwarden')).toBe('Not detected on this machine.');
    expect(isBackendReady(undefined, 'bitwarden')).toBe(false);
  });
});

describe('suggestedManager', () => {
  // "Offers a detected, signed-in manager; never blocks on one."
  it('suggests nothing when no manager is usable right now', () => {
    expect(suggestedManager([])).toBeNull();
    expect(suggestedManager([backend({ installed: true, signedIn: false })])).toBeNull();
    expect(suggestedManager(undefined)).toBeNull();
  });

  it('suggests the first usable manager', () => {
    const bw = backend({ id: 'bitwarden', displayName: 'Bitwarden' });
    expect(suggestedManager([backend({ signedIn: false }), bw])?.id).toBe('bitwarden');
  });
});

describe('keyStatusMessage', () => {
  it('says nothing when the source resolves', () => {
    expect(keyStatusMessage(keyEntry())).toBeNull();
    expect(keyStatusMessage(undefined)).toBeNull();
  });

  // The acceptance case: killing `op` mid-session must produce an honest
  // "couldn't read the key from 1Password", not a generic provider error.
  it('reports a configured source that cannot answer', () => {
    const entry = keyEntry({ resolves: false, error: '1Password is installed but not signed in.' });
    expect(keyStatusMessage(entry)).toBe('1Password is installed but not signed in.');
  });

  it('falls back to naming the source when the daemon gave no detail', () => {
    const entry = keyEntry({ resolves: false, error: null });
    expect(keyStatusMessage(entry)).toBe("Couldn't read this key from 1Password.");
  });

  it('reports an invalid stored source', () => {
    const entry = keyEntry({ kind: 'invalid', error: "'1password' is not a valid secret source." });
    expect(keyStatusMessage(entry)).toContain('not a valid secret source');
  });
});
