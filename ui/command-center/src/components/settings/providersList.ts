import type { ProviderInfo } from '../../lib/store';

export type ProviderTab = 'connected' | 'ready' | 'providers';

type ListedProvider = Pick<
  ProviderInfo,
  'name' | 'displayName' | 'isConfigured' | 'isDefault'
> & Partial<Pick<ProviderInfo, 'configKeys'>>;

/** A provider with nothing secret to store (a local runtime). Unknown key
 *  metadata is treated as keyed: never claim "no key needed" without knowing. */
export function isKeyless(provider: ListedProvider): boolean {
  return !!provider.configKeys && !provider.configKeys.some(k => k.secret);
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
