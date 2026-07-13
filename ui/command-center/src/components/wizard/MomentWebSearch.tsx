import { useState } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, GhostLink, Input, Glass, Particles } from './atoms';
import { SEARCH_PROVIDERS, saveAndEnableSearchProvider, type SearchProvider } from '../../lib/searchProviders';

interface Props {
  /** Configured persona display name, if already chosen earlier in the wizard. */
  personaName?: string;
  onAdvance: () => void;
  onBack: () => void;
}

type RowState = { input: string; saved: boolean; busy: boolean; error: string };

export function MomentWebSearch({ personaName, onAdvance, onBack }: Props) {
  const { colors } = useTheme();
  const who = personaName?.trim() || 'your agent';
  const [rows, setRows] = useState<Record<string, RowState>>(
    () => Object.fromEntries(SEARCH_PROVIDERS.map(p => [p.id, { input: '', saved: false, busy: false, error: '' }])),
  );
  const patch = (id: string, p: Partial<RowState>) =>
    setRows(prev => ({ ...prev, [id]: { ...prev[id], ...p } }));

  const anySaved = Object.values(rows).some(r => r.saved);

  const save = async (p: SearchProvider) => {
    const key = rows[p.id].input.trim();
    if (!key) return;
    patch(p.id, { busy: true, error: '' });
    try {
      await saveAndEnableSearchProvider(p, key);
      patch(p.id, { input: '', saved: true, error: '' });
    } catch (e) {
      console.error('Failed to set up search provider:', e);
      patch(p.id, { error: e instanceof Error ? e.message : "Couldn't save key. Please try again." });
    } finally {
      patch(p.id, { busy: false });
    }
  };

  // During onboarding the wizard overlays the app, so the in-app browser isn't
  // visible — open the provider's key page in the system browser here. The
  // in-app-browser-guided flow is the on-demand path (Settings / asking later).
  const getKey = (p: SearchProvider) => window.open(p.keyPageUrl, '_blank', 'noopener,noreferrer');

  return (
    <div style={{ position: 'relative', height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: 24 }}>
      <Particles density={20} />
      <Mobius size={72} state="idle" glow={0.9} />
      <div style={{ fontFamily: font.display, fontSize: 28, fontWeight: 700, letterSpacing: '-0.01em', color: colors.text, marginTop: 18, textAlign: 'center' }}>
        Give {who} the web?
      </div>
      <div style={{ fontFamily: font.body, fontSize: 13, color: colors.textMuted, marginTop: 8, maxWidth: 440, textAlign: 'center', lineHeight: 1.5 }}>
        Add a key for Brave or Tavily (or both) and {who} can search the web when it helps. Both have free tiers. You can skip this and set it up later anytime.
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 22, width: '100%', maxWidth: 460 }}>
        {SEARCH_PROVIDERS.map(p => {
          const r = rows[p.id];
          return (
            <Glass key={p.id} padding={14}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <div style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text }}>{p.displayName}</div>
                {r.saved && <span style={{ fontSize: 11, color: colors.cyan }}>✓ Connected</span>}
              </div>
              <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted, marginTop: 2 }}>{p.description}</div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 10 }}>
                <Input value={r.input} onChange={(v) => patch(p.id, { input: v, error: '' })} type="password" placeholder={r.saved ? '(connected — enter new to replace)' : 'Paste API key'} style={{ flex: 1 }} />
                <button
                  onClick={() => save(p)}
                  disabled={r.busy || !r.input.trim()}
                  style={{
                    height: 44, padding: '0 16px', borderRadius: radius.md, whiteSpace: 'nowrap',
                    background: 'transparent', border: `1px solid ${colors.cyan}66`,
                    color: r.busy || !r.input.trim() ? colors.textDim : colors.cyan,
                    fontFamily: font.body, fontSize: 13, fontWeight: 600,
                    cursor: r.busy || !r.input.trim() ? 'not-allowed' : 'pointer',
                  }}
                >{r.busy ? 'Saving…' : 'Save'}</button>
              </div>
              {r.error && (
                <div role="alert" style={{ fontFamily: font.body, fontSize: 11, color: colors.danger, marginTop: 8 }}>
                  {r.error}
                </div>
              )}
              <div style={{ marginTop: 6 }}>
                <GhostLink onClick={() => getKey(p)} style={{ fontSize: 12 }}>Get a {p.keyPageLabel.replace(/ keys?$/i, '')} key ↗</GhostLink>
              </div>
            </Glass>
          );
        })}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginTop: 24 }}>
        <GhostLink onClick={onBack}>Back</GhostLink>
        <PrimaryButton onClick={onAdvance}>{anySaved ? 'Continue' : 'Skip for now'}</PrimaryButton>
      </div>
    </div>
  );
}
