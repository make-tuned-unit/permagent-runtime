import { useCallback, useEffect, useState } from 'react';
import { FiCheck, FiSettings, FiStar } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { ConfigureProviderModal } from './ConfigureProviderModal';

export function ProvidersSection() {
  const { colors } = useTheme();
  const providers = useCommandCenter(s => s.providers);
  const providersError = useCommandCenter(s => s.providersError);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const [configuring, setConfiguring] = useState<ProviderInfo | null>(null);
  const [loading, setLoading] = useState(true);

  // loadProviders never rejects (it sets providersError internally), so `.finally`
  // is enough to end the loading state; the flag distinguishes failure from empty.
  const load = useCallback(() => {
    setLoading(true);
    Promise.resolve(loadProviders()).finally(() => setLoading(false));
  }, [loadProviders]);

  useEffect(() => { load(); }, [load]);

  return (
    <div className="space-y-3">
      <p className="text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>Configure LLM providers and API keys. The default provider is used for new chat sessions.</p>

      {providers.length === 0 && loading && (
        <div className="text-xs py-4 text-center" style={{ fontFamily: font.mono, color: colors.textMuted }}>Loading providers...</div>
      )}

      {providers.length === 0 && !loading && providersError && (
        <div className="rounded-lg p-4 text-center space-y-2" style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}` }}>
          <div className="text-xs" style={{ fontFamily: font.body, color: colors.danger }}>Couldn't load providers. Check that the daemon is running.</div>
          <button
            onClick={load}
            className="text-[11px] px-3 py-1.5 rounded transition"
            style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
            onMouseEnter={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
            onMouseLeave={e => { e.currentTarget.style.backgroundColor = ''; }}
          >
            Retry
          </button>
        </div>
      )}

      {providers.length === 0 && !loading && !providersError && (
        <div className="text-xs py-4 text-center" style={{ fontFamily: font.mono, color: colors.textMuted }}>No providers available.</div>
      )}

      {providers.map(p => (
        <div
          key={p.name}
          className="rounded-lg p-4"
          style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}
        >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div
                className="w-8 h-8 rounded flex items-center justify-center text-xs uppercase"
                style={{ backgroundColor: colors.cyanSoft, color: colors.cyan, fontFamily: font.display, fontWeight: 700 }}
              >
                {p.displayName.slice(0, 2)}
              </div>
              <div>
                <div className="text-sm" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>{p.displayName}</div>
                <div className="text-[10px]" style={{ fontFamily: font.body, color: colors.textMuted }}>{p.description}</div>
              </div>
            </div>
            <div className="flex items-center gap-2">
              {p.isDefault && (
                <span
                  className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded"
                  style={{ backgroundColor: colors.cyanSoft, color: colors.cyan }}
                >
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
              className="flex items-center gap-1.5 text-[11px] px-3 py-1.5 rounded hover:bg-white/5 transition"
              style={{ border: `1px solid ${colors.border}`, color: colors.textMuted }}
              onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
              onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            >
              <FiSettings size={11} /> Configure
            </button>
            {p.isConfigured && !p.isDefault && (
              <button
                onClick={() => setDefaultProvider(p.name, p.defaultModel)}
                className="text-[11px] px-3 py-1.5 rounded transition"
                style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
                onMouseEnter={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
                onMouseLeave={e => { e.currentTarget.style.backgroundColor = ''; }}
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
