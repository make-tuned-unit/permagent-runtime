import { useEffect, useState, useCallback, type CSSProperties } from 'react';
import { FiCheck, FiExternalLink } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { SEARCH_PROVIDERS, buildSearchExtensionQuery, saveAndEnableSearchProvider, type SearchProvider } from '../../lib/searchProviders';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Toggle } from '../common/Toggle';

interface ProviderRowState {
  configured: boolean;
  enabled: boolean;
  input: string;
  busy: boolean;
  /** The stored key, masked by the daemon (e.g. `BSAArB-L***…`). A boolean
   *  "Key saved" asks the user to take our word for it; showing the real prefix
   *  lets them recognize the key they pasted. Empty when nothing is stored. */
  masked: string;
  /** Result of the last "Test" probe. `null` = never tested this session — the
   *  honest default, since a stored key says nothing about whether it works. */
  probe: { ok: boolean; message: string } | null;
  probing: boolean;
  /** Last save/toggle failure — rendered inline (2026-07 wiring audit: the
   *  old console-only error made a failed key save look like a stuck form). */
  error: string;
}

const blank = (): ProviderRowState => ({ configured: false, enabled: false, input: '', busy: false, masked: '', probe: null, probing: false, error: '' });

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
      let masked = '';
      try {
        const r = await api.readSecretConfig(p.keyName);
        masked = r?.maskedValue ?? '';
        configured = masked.length > 0;
      } catch { /* key not set */ }
      patch(p.id, { configured, masked, enabled: enabledNames.has(p.displayName) });
    }));
  }, []);

  // `config_changed` → refetch: the API-key rows and the enabled flags here are
  // both config entries the agent can write.
  const configRev = useCommandCenter(s => s.configRev);
  useEffect(() => { refresh(); }, [refresh, configRev]);

  // Resolves `false` on failure — the Button contract's "it failed" signal.
  // This catch turns the throw into a row-level message, so without it a key
  // that never reached the keychain would still finish with a success tick.
  const saveKey = async (p: SearchProvider) => {
    const value = rows[p.id].input.trim();
    if (!value) return false;
    patch(p.id, { busy: true, error: '' });
    try {
      // Store the key as a keychain secret, then register (and enable) the MCP
      // connector — it reads the key back through env_keys.
      await saveAndEnableSearchProvider(p, value);
      patch(p.id, { input: '', configured: true, enabled: true });
      // Re-read so the badge shows the key the DAEMON actually stored, not what
      // we optimistically assumed — the save is only real once it reads back.
      await refresh();
      return true;
    } catch (e) {
      console.error('Failed to save search key:', e);
      patch(p.id, { error: `Couldn't save the key: ${e instanceof Error ? e.message : String(e)}` });
      return false;
    } finally {
      patch(p.id, { busy: false });
    }
  };

  /** Start the provider for real and report what it answered. A stored key and
   *  a WORKING key are different claims; only this can make the second one. */
  const testProvider = async (p: SearchProvider) => {
    patch(p.id, { probing: true, probe: null, error: '' });
    try {
      const r = await api.probeExtension(p.displayName);
      patch(p.id, {
        probe: r.ok
          ? { ok: true, message: `Working — ${r.toolCount} tool${r.toolCount === 1 ? '' : 's'} available` }
          : { ok: false, message: r.error || 'The provider did not answer.' },
      });
      // A probe that ran and came back "not working" is a failed test, not a
      // completed one: hand `false` back so the button does not tick over it.
      return r.ok;
    } catch (e) {
      patch(p.id, {
        probe: { ok: false, message: `Couldn't run the test: ${e instanceof Error ? e.message : String(e)}` },
      });
      return false;
    } finally {
      patch(p.id, { probing: false });
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
                    title={r.masked ? `Stored in your keychain as ${r.masked}` : undefined}
                  >
                    <FiCheck size={10} /> Key saved
                    {r.masked && (
                      <span style={{ fontFamily: font.mono, opacity: 0.8 }}>{r.masked}</span>
                    )}
                  </span>
                ) : (
                  <span
                    className="text-[10px] px-2 py-0.5 rounded"
                    style={{ backgroundColor: `${colors.textDim}26`, color: colors.textMuted }}
                  >No key</span>
                )}
                <Toggle on={r.enabled} onChange={(v) => toggleEnabled(p, v)} label={`Enable ${p.displayName}`} />
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
              <Button
                colors={colors}
                variant="ghostOn"
                onClick={() => saveKey(p)}
                disabled={r.busy || !r.input.trim()}
                style={{
                  '--pa-btn-border': `${colors.cyan}4D`,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.xs}px`,
                } as CSSProperties}
              >
                {r.busy ? 'Saving…' : 'Save'}
              </Button>
              <Button
                colors={colors}
                onClick={() => testProvider(p)}
                disabled={r.probing || !r.configured}
                title={r.configured ? 'Start the provider and list its tools' : 'Add a key first'}
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.xs}px`,
                } as CSSProperties}
              >
                {r.probing ? 'Testing…' : 'Test'}
              </Button>
              <Button
                colors={colors}
                onClick={() => openInBrowser(p.keyPageUrl)}
                title={`Open ${p.keyPageLabel} in the browser`}
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-hover': colors.fillHover,
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': `${radius.xs}px`,
                } as CSSProperties}
              >
                {/* `.pa-btn__label` is a plain span and Tailwind's preflight
                    makes every svg `display: block`, so an icon handed straight
                    to Button stacks on top of its own label. The row keeps its
                    own inline-flex until the primitive lays its label out. */}
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                  <FiExternalLink size={11} /> Get key
                </span>
              </Button>
            </div>
            {r.probe && (
              <div
                role="status"
                className="text-[11px] mt-2"
                style={{ fontFamily: font.body, color: r.probe.ok ? colors.success : colors.danger }}
              >
                {r.probe.ok ? '✓ ' : '✕ '}{r.probe.message}
              </div>
            )}
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
