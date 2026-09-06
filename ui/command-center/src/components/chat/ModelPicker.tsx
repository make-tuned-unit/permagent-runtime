import { useState, useEffect } from 'react';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { ProviderModelPicker } from '../settings/ProviderModelPicker';

export type PickerModel = {
  providerName: string;
  displayName: string;
  model: string;
};

export function filterConfiguredModels(providers: ProviderInfo[], query: string): PickerModel[] {
  const q = query.trim().toLowerCase();
  const configured = providers.filter(p => p.isConfigured);
  const out: PickerModel[] = [];
  for (const p of configured) {
    for (const model of p.knownModels) {
      if (
        !q
        || p.name.toLowerCase().includes(q)
        || p.displayName.toLowerCase().includes(q)
        || model.toLowerCase().includes(q)
      ) {
        out.push({ providerName: p.name, displayName: p.displayName, model });
      }
    }
  }
  return out;
}

export function ModelPicker() {
  const providers = useCommandCenter(s => s.providers);
  const storeModel = useCommandCenter(s => s.currentModel);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  /** The model the daemon refused. `setDefaultProvider` used to swallow the
   *  error, so the menu closed and the label stayed on the old model with
   *  nothing on screen saying the switch had not happened. */
  const [switchError, setSwitchError] = useState<{ provider: string; model: string } | null>(null);

  // Keep retrying while empty: opening chat during daemon boot left the
  // picker permanently on "no model" (the single fetch failed and nothing
  // re-triggered it). The interval dissolves as soon as providers land.
  useEffect(() => {
    if (providers.length > 0) return;
    loadProviders();
    const id = setInterval(loadProviders, 5000);
    return () => clearInterval(id);
  }, [providers.length, loadProviders]);

  const defaultProvider = providers.find(p => p.isDefault);
  const selectedProvider = defaultProvider?.name ?? null;
  const displayModel = storeModel || defaultProvider?.defaultModel || null;

  const handleSelect = async (providerName: string, model: string) => {
    setSwitchError(null);
    const ok = await setDefaultProvider(providerName, model);
    if (!ok) setSwitchError({ provider: providerName, model });
  };

  return <div>
    <ProviderModelPicker value={{ provider: selectedProvider, model: displayModel }} compact onChange={v => void handleSelect(v.provider, v.model)} aria-label="Choose chat provider and model" />
    {switchError && <div role="alert" className="text-right" style={{ fontSize: 10, color: '#ef6b73' }}>Couldn't switch to {switchError.provider}/{switchError.model}. Current: {selectedProvider ?? 'default'}/{displayModel ?? 'no model'}.</div>}
  </div>;
}
