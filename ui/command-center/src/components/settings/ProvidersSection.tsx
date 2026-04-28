import { useEffect, useState } from 'react';
import { FiCheck, FiSettings, FiStar } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { ConfigureProviderModal } from './ConfigureProviderModal';

export function ProvidersSection() {
  const providers = useCommandCenter(s => s.providers);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const [configuring, setConfiguring] = useState<ProviderInfo | null>(null);

  useEffect(() => { loadProviders(); }, [loadProviders]);

  return (
    <div className="space-y-3">
      <p className="text-xs text-dark-muted">Configure LLM providers and API keys. The default provider is used for new chat sessions.</p>

      {providers.length === 0 && (
        <div className="text-xs text-dark-muted font-mono py-4 text-center">Loading providers...</div>
      )}

      {providers.map(p => (
        <div key={p.name} className="rounded-lg border border-dark-border bg-[#111827] p-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded bg-accent/10 flex items-center justify-center text-accent text-xs font-bold uppercase">
                {p.displayName.slice(0, 2)}
              </div>
              <div>
                <div className="font-semibold text-sm">{p.displayName}</div>
                <div className="text-[10px] text-dark-muted">{p.description}</div>
              </div>
            </div>
            <div className="flex items-center gap-2">
              {p.isDefault && (
                <span className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded bg-accent/15 text-accent">
                  <FiStar size={10} /> Default
                </span>
              )}
              {p.isConfigured ? (
                <span className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded bg-emerald-500/15 text-emerald-400">
                  <FiCheck size={10} /> Connected
                </span>
              ) : (
                <span className="text-[10px] px-2 py-0.5 rounded bg-gray-500/15 text-gray-400">
                  Not configured
                </span>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2 mt-3">
            <button
              onClick={() => setConfiguring(p)}
              className="flex items-center gap-1.5 text-[11px] px-3 py-1.5 rounded border border-dark-border hover:bg-white/5 transition text-dark-muted hover:text-dark-text"
            >
              <FiSettings size={11} /> Configure
            </button>
            {p.isConfigured && !p.isDefault && (
              <button
                onClick={() => setDefaultProvider(p.name, p.defaultModel)}
                className="text-[11px] px-3 py-1.5 rounded border border-accent/30 text-accent hover:bg-accent/10 transition"
              >
                Set as default
              </button>
            )}
          </div>
        </div>
      ))}

      {configuring && (
        <ConfigureProviderModal
          provider={configuring}
          onClose={() => { setConfiguring(null); loadProviders(); }}
        />
      )}
    </div>
  );
}
