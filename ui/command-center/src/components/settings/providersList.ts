import type { ProviderInfo } from '../../lib/store';

export type ProviderTab = 'connected' | 'providers';

type ListedProvider = Pick<
  ProviderInfo,
  'name' | 'displayName' | 'isConfigured' | 'isDefault'
>;

function byDisplayName(a: ListedProvider, b: ListedProvider): number {
  return a.displayName.localeCompare(b.displayName);
}

/**
 * Split the API-keys list the way the page is used: keys you already have,
 * then the catalogue of providers you can still add. Connected ones sort
 * default-first so the active model is at the top of that tab; the catalogue
 * stays alphabetical so it does not jump around as keys come and go.
 */
export function partitionProviders<T extends ListedProvider>(providers: T[]): {
  connected: T[];
  available: T[];
} {
  const connected: T[] = [];
  const available: T[] = [];
  for (const provider of providers) {
    if (provider.isConfigured) connected.push(provider);
    else available.push(provider);
  }
  connected.sort((a, b) => {
    if (a.isDefault && !b.isDefault) return -1;
    if (!a.isDefault && b.isDefault) return 1;
    return byDisplayName(a, b);
  });
  available.sort(byDisplayName);
  return { connected, available };
}

/** Open on Connected once any key is in; otherwise land on the catalogue. */
export function initialProviderTab(connectedCount: number): ProviderTab {
  return connectedCount > 0 ? 'connected' : 'providers';
}
