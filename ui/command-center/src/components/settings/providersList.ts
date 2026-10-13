import type { ProviderInfo } from '../../lib/store';

export type ProviderTab = 'connected' | 'ready' | 'providers';

type ListedProvider = Pick<
  ProviderInfo,
  'name' | 'displayName' | 'isConfigured' | 'isDefault'
> & Partial<Pick<ProviderInfo, 'configKeys'>>;

/** A provider with no REQUIRED secret to store (a local runtime). An optional
 *  secret (e.g. OPENAI_CUSTOM_HEADERS on Llama Swap / LM Studio) does not make
 *  a provider keyed: it reports configured from defaults alone, so counting it
 *  as Connected would claim a credential nobody saved. Unknown key metadata is
 *  treated as keyed: never claim "no key needed" without knowing. */
export function isKeyless(provider: ListedProvider): boolean {
  return !!provider.configKeys && !provider.configKeys.some(k => k.secret && k.required);
}

/** The secret key whose presence decides "Connected": the first required one. */
export function requiredSecretKeyName(provider: ListedProvider): string {
  return provider.configKeys?.find(k => k.secret && k.required)?.name ?? '';
}

/** Whether the credential behind a keyed, configured provider is known to be
 *  saved. The daemon reports a keyed provider configured only once its required
 *  secret resolves, so that is the presence signal; a key pointed at a password
 *  manager whose read has not been confirmed is 'unknown', shown as "Key saved?"
 *  rather than Connected. */
export function credentialState(
  provider: ListedProvider,
  source?: { kind: string; resolves?: boolean | null },
): 'present' | 'unknown' {
  if (!provider.isConfigured || isKeyless(provider)) return 'unknown';
  if (source && source.kind !== 'keychain' && source.kind !== 'file' && source.resolves !== true) return 'unknown';
  return 'present';
}

function byDisplayName(a: ListedProvider, b: ListedProvider): number {
  return a.displayName.localeCompare(b.displayName);
}

/**
 * Split the API-keys list the way the page is used: keys you already have
 * (Connected), local runtimes that need no key (Ready to use — a count of
 * secrets saved, not a handshake, so they are never called Connected), then
 * the catalogue of providers you can still add. Connected ones sort
 * default-first so the active model is at the top of that tab; the catalogue
 * stays alphabetical so it does not jump around as keys come and go.
 */
export function partitionProviders<T extends ListedProvider>(providers: T[]): {
  connected: T[];
  ready: T[];
  available: T[];
} {
  const connected: T[] = [];
  const ready: T[] = [];
  const available: T[] = [];
  for (const provider of providers) {
    if (!provider.isConfigured) available.push(provider);
    else if (isKeyless(provider)) ready.push(provider);
    else connected.push(provider);
  }
  connected.sort((a, b) => {
    if (a.isDefault && !b.isDefault) return -1;
    if (!a.isDefault && b.isDefault) return 1;
    return byDisplayName(a, b);
  });
  ready.sort((a, b) => {
    if (a.isDefault && !b.isDefault) return -1;
    if (!a.isDefault && b.isDefault) return 1;
    return byDisplayName(a, b);
  });
  available.sort(byDisplayName);
  return { connected, ready, available };
}

/** Open on Connected once any key is in, then on Ready to use; otherwise land
 *  on the catalogue. */
export function initialProviderTab(connectedCount: number, readyCount = 0): ProviderTab {
  if (connectedCount > 0) return 'connected';
  return readyCount > 0 ? 'ready' : 'providers';
}
