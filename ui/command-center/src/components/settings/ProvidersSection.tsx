import { useCallback, useEffect, useState } from 'react';
import { FiAlertTriangle, FiCheck, FiKey, FiPlus, FiSettings, FiStar, FiTrash2 } from 'react-icons/fi';
import { api } from '../../lib/api';
import type { SecretSourcesResponse } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AddCustomProviderModal } from './AddCustomProviderModal';
import { ConfigureProviderModal } from './ConfigureProviderModal';
import { findKeySource, keyStatusMessage } from './secretSource';

export function ProvidersSection() {
  const { colors } = useTheme();
  const providers = useCommandCenter(s => s.providers);
  const providersError = useCommandCenter(s => s.providersError);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const [configuring, setConfiguring] = useState<ProviderInfo | null>(null);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [secretSources, setSecretSources] = useState<SecretSourcesResponse | undefined>();

  // Fetched here rather than in the modal so the card badge and the modal
  // cannot disagree about where a key comes from. Failure is silent on purpose:
  // an older daemon has no such route, and every key is then on the keychain —
  // which is exactly what an absent badge means.
  const loadSecretSources = useCallback(() => {
    api.getSecretSources()
      .then(setSecretSources)
      .catch(() => setSecretSources(undefined));
  }, []);

  useEffect(() => { loadSecretSources(); }, [loadSecretSources]);

  // loadProviders never rejects (it sets providersError internally), so `.finally`
  // is enough to end the loading state; the flag distinguishes failure from empty.
  const load = useCallback(() => {
    setLoading(true);
    Promise.resolve(loadProviders()).finally(() => setLoading(false));
  }, [loadProviders]);

  useEffect(() => { load(); }, [load]);

  // Only user-defined ("Custom") providers can be removed — built-in ones have
  // no on-disk definition to delete.
  const handleRemove = useCallback(async (p: ProviderInfo) => {
    if (!confirm(`Remove custom provider "${p.displayName}"? This deletes its saved configuration.`)) return;
    setRemoving(p.name);
    try {
      await api.removeCustomProvider(p.name);
      await loadProviders();
    } catch (e) {
      console.error('Failed to remove custom provider:', e);
    } finally {
      setRemoving(null);
    }
  }, [loadProviders]);

  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between gap-3">
        <p className="text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>Configure LLM providers and API keys. The default provider is used for new chat sessions.</p>
        <button
          onClick={() => setAdding(true)}
          className="flex items-center gap-1.5 text-[11px] px-3 py-1.5 rounded transition shrink-0"
          style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
          onMouseEnter={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
          onMouseLeave={e => { e.currentTarget.style.backgroundColor = ''; }}
        >
          <FiPlus size={11} /> Add custom provider
        </button>
      </div>

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

      {providers.map(p => {
        // The badge names the SOURCE only when it is not the keychain — the
        // default needs no label, and labelling it would bury the one case
        // worth noticing.
        const keyName = p.configKeys.find(k => k.secret)?.name ?? '';
        const sourceEntry = findKeySource(secretSources?.keys, keyName);
        const sourceProblem = keyStatusMessage(sourceEntry);
        return (
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
              {sourceEntry && (
                <span
                  className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded"
                  title={sourceProblem ?? `${keyName} is read from ${sourceEntry.label} (${sourceEntry.reference})`}
                  style={
                    sourceProblem
                      ? { backgroundColor: `${colors.danger}26`, color: colors.danger }
                      : { backgroundColor: `${colors.textMuted}26`, color: colors.textMuted }
                  }
                >
                  {sourceProblem ? <FiAlertTriangle size={10} /> : <FiKey size={10} />} {sourceEntry.label}
                </span>
              )}
              {p.isDefault && (
                <span
                  className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded"
                  style={{ backgroundColor: colors.cyanSoft, color: colors.cyan }}
                >
                  <FiStar size={10} /> Default
                </span>
              )}
              {p.isConfigured ? (
                <span
                  className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded"
                  style={{ backgroundColor: `${colors.success}26`, color: colors.success }}
                >
                  <FiCheck size={10} /> Connected
                </span>
              ) : (
                <span
                  className="text-[10px] px-2 py-0.5 rounded"
                  style={{ backgroundColor: `${colors.textMuted}26`, color: colors.textMuted }}
                >
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
            {p.providerType === 'Custom' && (
              <button
                onClick={() => handleRemove(p)}
                disabled={removing === p.name}
                className="flex items-center gap-1.5 text-[11px] px-3 py-1.5 rounded transition disabled:opacity-50 ml-auto"
                style={{ border: `1px solid ${colors.danger}4D`, color: colors.danger }}
                onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.danger}1A`; }}
                onMouseLeave={e => { e.currentTarget.style.backgroundColor = ''; }}
              >
                <FiTrash2 size={11} /> {removing === p.name ? 'Removing…' : 'Remove'}
              </button>
            )}
          </div>

          {/* A configured reference the daemon cannot read. Stated on the card,
              not just inside the modal, because the whole point is that the
              user learns it here instead of from an unexplained provider error
              in the middle of a chat turn. */}
          {sourceProblem && (
            <div className="text-[11px] mt-2" style={{ fontFamily: font.body, color: colors.danger }}>
              {sourceProblem}
            </div>
          )}
        </div>
        );
      })}

      {configuring && (
        <ConfigureProviderModal
          provider={configuring}
          secretSources={secretSources}
          onSourcesChanged={loadSecretSources}
          onClose={() => { setConfiguring(null); loadProviders(); loadSecretSources(); }}
        />
      )}

      {adding && (
        <AddCustomProviderModal
          onClose={() => { setAdding(false); loadProviders(); }}
        />
      )}
    </div>
  );
}
