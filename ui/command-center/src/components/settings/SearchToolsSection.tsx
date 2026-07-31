import { useEffect, useState, useCallback } from 'react';
import { FiCheck, FiExternalLink } from 'react-icons/fi';
import { api } from '../../lib/api';
import { SEARCH_PROVIDERS, buildSearchExtensionQuery, saveAndEnableSearchProvider, type SearchProvider } from '../../lib/searchProviders';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Toggle } from './atoms';

interface ProviderRowState {
  configured: boolean;
  enabled: boolean;
  input: string;
  busy: boolean;
  /** Last save/toggle failure — rendered inline (2026-07 wiring audit: the
   *  old console-only error made a failed key save look like a stuck form). */
  error: string;
}

const blank = (): ProviderRowState => ({ configured: false, enabled: false, input: '', busy: false, error: '' });

/**
 * Generic "Search & tools" credential section — the bridge from #226/#352,
 * which scoped LLM-provider keys to that lane and routed non-model service keys
 * (search) here. Registry-free: each entry stores its key as a plain secret via
 * /config/upsert and registers/toggles the matching MCP connector. No provider-
 * model chrome. Web search is OFF until a key is added and the entry enabled —
 * the v1 egress opt-in, visible here and in Tools & MCPs.
 */
export function SearchToolsSection() {
  const { colors } = useTheme();
  const openInBrowser = useBrowserNavigate();
  const [rows, setRows] = useState<Record<string, ProviderRowState>>(
    () => Object.fromEntries(SEARCH_PROVIDERS.map(p => [p.id, blank()])),
  );

  const patch = (id: string, p: Partial<ProviderRowState>) =>
    setRows(prev => ({ ...prev, [id]: { ...prev[id], ...p } }));

  const refresh = useCallback(async () => {
    let enabledNames = new Set<string>();
    try {
      const { extensions } = await api.getExtensions();
      enabledNames = new Set(extensions.filter(e => e.enabled).map(e => e.name));
    } catch { /* extension list unavailable */ }

    await Promise.all(SEARCH_PROVIDERS.map(async p => {
      let configured = false;
      try {
        const r = await api.readConfig(p.keyName, true);
        configured = !!(r?.maskedValue || r?.value);
      } catch { /* key not set */ }
      patch(p.id, { configured, enabled: enabledNames.has(p.displayName) });
    }));
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const saveKey = async (p: SearchProvider) => {
    const value = rows[p.id].input.trim();
    if (!value) return;
    patch(p.id, { busy: true, error: '' });
    try {
      // Store the key as a keychain secret, then register (and enable) the MCP
      // connector — it reads the key back through env_keys.
      await saveAndEnableSearchProvider(p, value);
      patch(p.id, { input: '', configured: true, enabled: true });
    } catch (e) {
      console.error('Failed to save search key:', e);
      patch(p.id, { error: `Couldn't save the key: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      patch(p.id, { busy: false });
    }
  };

  const toggleEnabled = async (p: SearchProvider, on: boolean) => {
    patch(p.id, { busy: true, enabled: on, error: '' });
    try {
      // Keep the entry (preserves config); just flip its enabled flag.
      await api.addExtension(await buildSearchExtensionQuery(p, on));
    } catch (e) {
      console.error('Failed to toggle search provider:', e);
      patch(p.id, {
        enabled: !on,
        error: `Couldn't ${on ? 'enable' : 'disable'} ${p.displayName}: ${e instanceof Error ? e.message : String(e)}`,
      });
    } finally {
      patch(p.id, { busy: false });
    }
  };

  return (
    <div className="space-y-3">
      <p className="text-xs" style={{ fontFamily: font.body, color: colors.textMuted }}>
        Keys for web search and other service tools. Stored encrypted in your system keychain — they never leave your device. Search stays off until a key is added.
      </p>
      {SEARCH_PROVIDERS.map(p => {
        const r = rows[p.id];
        return (
          <div
            key={p.id}
            className="rounded-lg p-4"
            style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}
          >
            <div className="flex items-center justify-between">
              <div>
                <div className="text-sm" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>{p.displayName}</div>
                <div className="text-[10px]" style={{ fontFamily: font.body, color: colors.textMuted }}>{p.description}</div>
              </div>
              <div className="flex items-center gap-2">
                {r.configured ? (
                  <span
                    className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded"
                    style={{ backgroundColor: `${colors.success}26`, color: colors.success }}
                  >
                    <FiCheck size={10} /> Key saved
                  </span>
                ) : (
                  <span
                    className="text-[10px] px-2 py-0.5 rounded"
                    style={{ backgroundColor: `${colors.textDim}26`, color: colors.textMuted }}
                  >No key</span>
                )}
                <Toggle on={r.enabled} onChange={(v) => toggleEnabled(p, v)} />
              </div>
            </div>

            <div className="flex items-center gap-2 mt-3">
              <input
                type="password"
                value={r.input}
                onChange={e => patch(p.id, { input: e.target.value })}
                placeholder={r.configured ? '(key saved — enter new to replace)' : 'Paste API key'}
                className="flex-1 text-[11px] px-3 py-1.5 rounded outline-none"
                style={{ fontFamily: font.mono, background: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text }}
              />
              <button
                onClick={() => saveKey(p)}
                disabled={r.busy || !r.input.trim()}
                className="text-[11px] px-3 py-1.5 rounded transition disabled:opacity-40"
                style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
              >
                {r.busy ? 'Saving…' : 'Save'}
              </button>
              <button
                onClick={() => openInBrowser(p.keyPageUrl)}
                title={`Open ${p.keyPageLabel} in the browser`}
                className="flex items-center gap-1 text-[11px] px-3 py-1.5 rounded hover:bg-white/5 transition"
                style={{ border: `1px solid ${colors.border}`, color: colors.textMuted }}
              >
                <FiExternalLink size={11} /> Get key
              </button>
            </div>
            {r.error && (
              <div role="alert" className="text-[11px] mt-2" style={{ fontFamily: font.body, color: colors.danger }}>
                {r.error}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
