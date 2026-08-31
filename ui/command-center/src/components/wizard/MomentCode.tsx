import { useEffect, useState, type CSSProperties } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, GhostLink, Input, Glass, Particles, WizardHeading, WizardSubhead } from './atoms';
import { api } from '../../lib/api';

interface Props {
  personaName?: string;
  onAdvance: () => void;
  onBack: () => void;
}

type Check = { resolved: string; exists: boolean; has_repositories: boolean };

/**
 * Merge what the user already confirmed with what discovery proposes.
 *
 * Two rules, both load-bearing on re-entry:
 *  - a confirmed root is listed first and stays ticked, because it is an answer
 *    the user gave, not a suggestion competing with fresh guesses;
 *  - discovery only PRE-TICKS when there is nothing confirmed. Otherwise a
 *    re-run of the wizard would silently re-add a root the user had removed.
 */
export function mergeRoots(confirmed: string[], discovered: string[]): {
  candidates: string[];
  preselected: string[];
} {
  const extra = discovered.filter(d => !confirmed.includes(d));
  return {
    candidates: [...confirmed, ...extra],
    preselected: confirmed.length ? confirmed : discovered,
  };
}

/**
 * Ask where the user keeps their code.
 *
 * This step exists because four features guessed `~/dev` and all four were
 * wrong on the same machine — and each failed by finding NOTHING, which looks
 * identical to a clean disk, an unstarted project, or a feature with nothing to
 * say. A wrong path never announces itself, so the only fix is to ask.
 *
 * Discovery proposes; the user confirms. Proposals are evidence-based (a `.git`
 * was really found), pre-ticked because they are usually right, and every one is
 * shown with its full path so confirming is a reading task rather than an act of
 * faith.
 */
export function MomentCode({ personaName, onAdvance, onBack }: Props) {
  const { colors } = useTheme();
  const who = personaName?.trim() || 'your agent';

  const [loading, setLoading] = useState(true);
  const [home, setHome] = useState('');
  const [candidates, setCandidates] = useState<string[]>([]);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [manual, setManual] = useState('');
  const [check, setCheck] = useState<Check | null>(null);
  const [checking, setChecking] = useState(false);
  const [saveError, setSaveError] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const r = await api.getDevRoots();
        if (!alive) return;
        setHome(r.home);
        const { candidates, preselected } = mergeRoots(r.confirmed, r.discovered);
        setCandidates(candidates);
        setChosen(new Set(preselected));
      } catch {
        // A failed scan is not a reason to block setup — the manual field below
        // still works, and it is the honest path anyway.
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => { alive = false; };
  }, []);

  const toggle = (p: string) =>
    setChosen(prev => {
      const next = new Set(prev);
      if (!next.delete(p)) next.add(p);
      return next;
    });

  // Returns the outcome so the Button primitive cannot tick success over the
  // "there's no folder at …" notice this same call puts on screen.
  const addManual = async () => {
    const raw = manual.trim();
    if (!raw) return false;
    setChecking(true);
    setCheck(null);
    try {
      const r = await api.checkDevRoot(raw);
      setCheck(r);
      if (r.exists) {
        // Accepted even with no repositories in it — the user may be pointing
        // at a directory they're about to fill — but the notice below says so.
        setCandidates(prev => (prev.includes(r.resolved) ? prev : [...prev, r.resolved]));
        setChosen(prev => new Set(prev).add(r.resolved));
        setManual('');
      }
      return r.exists;
    } catch (e) {
      setCheck({ resolved: raw, exists: false, has_repositories: false });
      console.error('dev-root check failed:', e);
      return false;
    } finally {
      setChecking(false);
    }
  };

  const save = async (paths: string[]) => {
    setSaving(true);
    setSaveError('');
    try {
      await api.upsertConfig('dev_roots', paths);
      onAdvance();
    } catch (e) {
      // Never advance on a failed write: a wizard that says "saved" and didn't
      // reproduces the silent-empty-result bug this whole step exists to end.
      setSaveError(e instanceof Error ? e.message : String(e));
      setSaving(false);
    }
  };

  const selected = candidates.filter(c => chosen.has(c));

  return (
    <div style={{ position: 'relative', height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: 24, overflowY: 'auto' }}>
      <Particles density={20} />
      <Mobius size={72} state="idle" glow={0.9} />
      <WizardHeading style={{ marginTop: 18 }}>Where do you keep your code?</WizardHeading>
      <WizardSubhead style={{ maxWidth: 470 }}>
        {who} needs this to find your projects, reclaim disk space from old build
        caches, and work on the right checkout. Everyone lays their machine out
        differently, so I'd rather ask than assume — a wrong guess here doesn't
        fail loudly, it just quietly finds nothing.
      </WizardSubhead>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 22, width: '100%', maxWidth: 480 }}>
        {loading && (
          <div style={{ fontFamily: font.body, fontSize: 12, color: colors.textMuted, textAlign: 'center' }}>
            Looking for repositories…
          </div>
        )}

        {!loading && candidates.length === 0 && (
          <Glass padding={14}>
            <div style={{ fontFamily: font.body, fontSize: 12, color: colors.text }}>
              I looked under {home || 'your home folder'} and didn't find any git
              repositories.
            </div>
            <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted, marginTop: 4 }}>
              That's fine — type the folder below, or skip and tell me later in
              Settings.
            </div>
          </Glass>
        )}

        {!loading && candidates.map(p => {
          const on = chosen.has(p);
          return (
            <Glass key={p} padding={12}>
              <label style={{ display: 'flex', alignItems: 'center', gap: 10, cursor: 'pointer' }}>
                <input
                  type="checkbox"
                  checked={on}
                  onChange={() => toggle(p)}
                  style={{ accentColor: colors.cyan, width: 16, height: 16, flexShrink: 0 }}
                />
                <span style={{ fontFamily: font.mono, fontSize: 12, color: on ? colors.text : colors.textMuted, wordBreak: 'break-all' }}>
                  {p}
                </span>
              </label>
            </Glass>
          );
        })}

        {!loading && (
          <div style={{ marginTop: 4 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Input
                value={manual}
                onChange={(v) => { setManual(v); setCheck(null); }}
                placeholder="Somewhere else? e.g. ~/Documents/dev"
                style={{ flex: 1 }}
              />
              <Button
                colors={colors}
                variant="ghostOn"
                type="button"
                onClick={addManual}
                disabled={checking || !manual.trim()}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': checking || !manual.trim() ? colors.textDim : colors.cyan,
                  '--pa-btn-border': `${colors.cyan}66`,
                  '--pa-btn-bg-hover': colors.cyanSoft,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-pad': '0 16px',
                  '--pa-btn-radius': `${radius.md}px`,
                  '--pa-btn-weight': 600,
                  height: 44, whiteSpace: 'nowrap',
                  fontFamily: font.body, fontSize: 13,
                } as CSSProperties}
              >{checking ? 'Checking…' : 'Add'}</Button>
            </div>

            {check && !check.exists && (
              <div role="alert" style={{ fontFamily: font.body, fontSize: 11, color: colors.danger, marginTop: 8 }}>
                There's no folder at {check.resolved}. Check the spelling — I'd
                rather tell you now than accept it and find nothing later.
              </div>
            )}
            {check?.exists && !check.has_repositories && (
              <div role="status" style={{ fontFamily: font.body, fontSize: 11, color: colors.warning, marginTop: 8 }}>
                Added {check.resolved} — though I don't see any git repositories
                in there yet.
              </div>
            )}
          </div>
        )}
      </div>

      {saveError && (
        <div role="alert" style={{ fontFamily: font.body, fontSize: 11, color: colors.danger, marginTop: 12, maxWidth: 470, textAlign: 'center' }}>
          Couldn't save that ({saveError}). Try again, or skip and set it in
          Settings.
        </div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginTop: 24 }}>
        <GhostLink onClick={onBack}>Back</GhostLink>
        {selected.length > 0
          ? (
            <PrimaryButton onClick={() => save(selected)} disabled={saving}>
              {saving ? 'Saving…' : `Use ${selected.length === 1 ? 'this folder' : `these ${selected.length} folders`}`}
            </PrimaryButton>
          )
          : <PrimaryButton onClick={onAdvance}>Skip for now</PrimaryButton>}
      </div>
    </div>
  );
}
