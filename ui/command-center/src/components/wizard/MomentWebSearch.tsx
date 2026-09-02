import { useEffect, useState, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, GhostLink, Input, Glass, Particles, WizardHeading, WizardSubhead } from './atoms';
import { api } from '../../lib/api';
import { SEARCH_PROVIDERS, saveAndEnableSearchProvider, type SearchProvider } from '../../lib/searchProviders';

interface Props {
  /** Configured persona display name, if already chosen earlier in the wizard. */
  personaName?: string;
  onAdvance: () => void;
  onBack: () => void;
}

type Verify = { ok: boolean; message: string } | null;
type RowState = {
  input: string;
  saved: boolean;
  /** Key saved AND the provider answered a live probe. */
  verified: boolean;
  busy: boolean;
  /** Card expanded into the guided walkthrough (auto on "Open the key page"). */
  guiding: boolean;
  error: string;
  verify: Verify;
};

const freshRow = (): RowState =>
  ({ input: '', saved: false, verified: false, busy: false, guiding: false, error: '', verify: null });

// Tavily first in the wizard: it's the one-click, no-card path, and a user who
// will struggle should meet the easiest door first. Settings keeps lib order.
const WIZARD_ORDER = [...SEARCH_PROVIDERS].sort(p => (p.id === 'tavily' ? -1 : 1));

export function MomentWebSearch({ personaName, onAdvance, onBack }: Props) {
  const { colors } = useTheme();
  const who = personaName?.trim() || 'your agent';
  const [rows, setRows] = useState<Record<string, RowState>>(
    () => Object.fromEntries(SEARCH_PROVIDERS.map(p => [p.id, freshRow()])),
  );
  const patch = (id: string, p: Partial<RowState>) =>
    setRows(prev => ({ ...prev, [id]: { ...prev[id], ...p } }));

  // Read-back (2026-07 wiring audit): a key saved before an interrupted wizard
  // run is in the keychain — on re-entry, show it as connected instead of
  // pretending nothing was saved. Mirrors SearchToolsSection.refresh.
  useEffect(() => {
    let alive = true;
    (async () => {
      await Promise.all(SEARCH_PROVIDERS.map(async p => {
        try {
          const r = await api.readSecretConfig(p.keyName);
          if (alive && r?.maskedValue) patch(p.id, { saved: true });
        } catch { /* key not set */ }
      }));
    })();
    return () => { alive = false; };
  }, []);

  const anySaved = Object.values(rows).some(r => r.saved);

  // During onboarding the wizard overlays the app, so the in-app browser isn't
  // visible — open the provider's key page in the system browser and expand the
  // card into the numbered walkthrough so the two sit side by side. The
  // in-app-browser-guided flow is the on-demand path (Settings / asking later).
  const openKeyPage = (p: SearchProvider) => {
    window.open(p.keyPageUrl, '_blank', 'noopener,noreferrer');
    patch(p.id, { guiding: true, error: '', verify: null });
  };

  // One button, two claims: store the key AND prove it answers. A stored key
  // and a working key are different things; a new user can't tell them apart,
  // so we never report success on save alone.
  const saveAndTest = async (p: SearchProvider) => {
    const key = rows[p.id].input.trim();
    if (!key) return false;
    patch(p.id, { busy: true, error: '', verify: null });
    try {
      await saveAndEnableSearchProvider(p, key);
      patch(p.id, { input: '', saved: true });
    } catch (e) {
      console.error('Failed to set up search provider:', e);
      patch(p.id, { busy: false, error: e instanceof Error ? e.message : "Couldn't save the key. Please try again." });
      return false;
    }
    return runProbe(p);
  };

  // Returns the verdict so the Button primitive never ticks over a probe whose
  // warning is sitting right underneath it.
  const runProbe = async (p: SearchProvider) => {
    patch(p.id, { busy: true, verify: null });
    try {
      const r = await api.probeExtension(p.displayName);
      patch(p.id, r.ok
        ? { verified: true, verify: { ok: true, message: `Working — ${who} can search the web now.` } }
        : {
            verify: {
              ok: false,
              message: `The key is saved, but ${p.displayName} didn't answer${r.error ? ` (${r.error})` : ''}. ` +
                'Check you copied the whole key with no spaces — a brand-new key can also take a minute to activate.',
            },
          });
      return r.ok;
    } catch (e) {
      patch(p.id, {
        verify: {
          ok: false,
          message: `The key is saved but the test couldn't run (${e instanceof Error ? e.message : String(e)}). You can re-test any time in Settings → Search & tools.`,
        },
      });
      return false;
    } finally {
      patch(p.id, { busy: false });
    }
  };

  return (
    <div style={{ position: 'relative', height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: 24, overflowY: 'auto' }}>
      <Particles density={20} />
      <Mobius size={72} state="idle" glow={0.9} />
      <WizardHeading style={{ marginTop: 18 }}>Give {who} the web?</WizardHeading>
      <WizardSubhead style={{ maxWidth: 460 }}>
        With a free search key, {who} can look things up for you. Pick one below — I'll open the
        right page and walk you through it, step by step. Your key is stored in your
        system keychain and never leaves this device. You can also skip this and set it up later.
      </WizardSubhead>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginTop: 22, width: '100%', maxWidth: 480 }}>
        {WIZARD_ORDER.map(p => {
          const r = rows[p.id];
          const done = r.verified || (r.saved && !r.guiding);
          return (
            <Glass key={p.id} padding={14}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <div style={{ fontFamily: font.display, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{p.displayName}</div>
                  {p.id === 'tavily' && !done && (
                    <span style={{ fontFamily: font.body, fontSize: 10, fontWeight: 600, color: colors.cyan, border: `1px solid ${colors.cyan}55`, borderRadius: radius.sm, padding: '1px 6px' }}>
                      EASIEST — NO CARD
                    </span>
                  )}
                </div>
                {done && <span style={{ fontSize: textSize.micro, color: colors.cyan }}>{r.verified ? '✓ Working' : '✓ Connected'}</span>}
              </div>
              <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginTop: 2 }}>
                {p.description} {p.freeTierNote}
              </div>

              {!r.guiding && !done && (
                <div style={{ marginTop: 10 }}>
                  <PrimaryButton onClick={() => openKeyPage(p)} style={{ height: 38, fontSize: textSize.small }}>
                    Open the key page — I'll guide you
                  </PrimaryButton>
                </div>
              )}

              {r.guiding && (
                <>
                  <div style={{ marginTop: 10, padding: '10px 12px', borderRadius: radius.md, background: `${colors.cyan}0D`, border: `1px solid ${colors.cyan}22` }}>
                    <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, color: colors.text, marginBottom: 6 }}>
                      On the page that just opened:
                    </div>
                    {p.setupSteps.map((s, i) => (
                      <div key={i} style={{ display: 'flex', gap: 8, fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.7 }}>
                        <span style={{ color: colors.cyan, fontWeight: 600, flexShrink: 0 }}>{i + 1}.</span>
                        <span>{s}</span>
                      </div>
                    ))}
                    <GhostLink onClick={() => openKeyPage(p)} style={{ fontSize: textSize.micro, marginTop: 4 }}>
                      Page didn't open? Click to try again ↗
                    </GhostLink>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 10 }}>
                    <Input value={r.input} onChange={(v) => patch(p.id, { input: v, error: '' })} type="password" placeholder={r.saved ? '(connected — paste a new key to replace)' : 'Paste your API key here'} style={{ flex: 1 }} />
                    <Button
                      colors={colors}
                      variant="ghostOn"
                      type="button"
                      onClick={() => saveAndTest(p)}
                      disabled={r.busy || !r.input.trim()}
                      style={{
                        '--pa-btn-bg': 'transparent',
                        '--pa-btn-fg': r.busy || !r.input.trim() ? colors.textDim : colors.cyan,
                        '--pa-btn-border': `${colors.cyan}66`,
                        '--pa-btn-bg-hover': colors.cyanSoft,
                        '--pa-btn-border-hover': colors.cyan,
                        '--pa-btn-pad': '0 16px',
                        '--pa-btn-radius': `${radius.md}px`,
                        '--pa-btn-weight': 600,
                        height: 44, whiteSpace: 'nowrap',
                        fontFamily: font.body, fontSize: textSize.small,
                      } as CSSProperties}
                    >{r.busy ? 'Testing…' : 'Save & test'}</Button>
                  </div>
                </>
              )}

              {r.error && (
                <div role="alert" style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.danger, marginTop: 8 }}>
                  {r.error}
                </div>
              )}
              {r.verify && (
                <div role={r.verify.ok ? 'status' : 'alert'} style={{ fontFamily: font.body, fontSize: textSize.micro, color: r.verify.ok ? colors.cyan : colors.warning, marginTop: 8 }}>
                  {r.verify.ok ? '✓ ' : ''}{r.verify.message}
                  {!r.verify.ok && (
                    <>
                      {' '}
                      <GhostLink onClick={() => runProbe(p)} style={{ fontSize: textSize.micro }}>Test again</GhostLink>
                    </>
                  )}
                </div>
              )}
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
