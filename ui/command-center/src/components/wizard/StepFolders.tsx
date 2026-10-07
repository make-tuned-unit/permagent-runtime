import { useEffect, useId, useState } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api } from '../../lib/api';
import { FieldLabel, GhostLink, Input, SecondaryButton, StatusLine } from './atoms';
import { StepFrame } from './StepFrame';
import { safeWizardError } from './wizardErrors';

interface Props {
  agentName: string;
  onNext: () => void;
}

type Check = { resolved: string; exists: boolean; has_repositories: boolean };

/**
 * Merge what the user already confirmed with what discovery proposes.
 *
 *  - a confirmed root is listed first and stays ticked: it is an answer the
 *    user gave, not a suggestion competing with fresh guesses;
 *  - discovery only PRE-TICKS when nothing is confirmed, so re-running setup
 *    never silently re-adds a folder the user removed.
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
 * Where the person keeps their code. Asked, not guessed: a wrong guess never
 * fails loudly, it just finds nothing. Optional — "I don't write code" saves an
 * empty list, which also clears a list saved by an earlier run.
 */
export function StepFolders({ agentName, onNext }: Props) {
  const { colors } = useTheme();
  const manualId = useId();
  const [loading, setLoading] = useState(true);
  const [home, setHome] = useState('');
  const [candidates, setCandidates] = useState<string[]>([]);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [manual, setManual] = useState('');
  const [check, setCheck] = useState<Check | null>(null);
  const [checking, setChecking] = useState(false);
  const [saveError, setSaveError] = useState('');
  const [saving, setSaving] = useState(false);
  const [scanError, setScanError] = useState('');
  const [scanNonce, setScanNonce] = useState(0);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setScanError('');
    api.getDevRoots()
      .then(r => {
        if (!alive) return;
        setHome(r.home);
        const merged = mergeRoots(r.confirmed, r.discovered);
        setCandidates(merged.candidates);
        setChosen(new Set(merged.preselected));
      })
      .catch(e => { if (alive) setScanError(e instanceof Error ? e.message : 'Could not look for repositories.'); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [scanNonce]);

  const toggle = (p: string) => setChosen(prev => {
    const next = new Set(prev);
    if (!next.delete(p)) next.add(p);
    return next;
  });

  const addManual = async () => {
    const raw = manual.trim();
    if (!raw) return false;
    setChecking(true);
    setCheck(null);
    try {
      const r = await api.checkDevRoot(raw);
      setCheck(r);
      if (r.exists) {
        setCandidates(prev => (prev.includes(r.resolved) ? prev : [...prev, r.resolved]));
        setChosen(prev => new Set(prev).add(r.resolved));
        setManual('');
      }
      return r.exists;
    } catch (e) {
      setCheck({ resolved: raw, exists: false, has_repositories: false });
      console.error('dev-root check failed:', safeWizardError(e, 'repository path check failed'));
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
      onNext();
    } catch (e) {
      // Never advance on a failed write.
      setSaveError(e instanceof Error ? e.message : String(e));
      setSaving(false);
    }
  };

  const selected = candidates.filter(c => chosen.has(c));
  const label = selected.length === 0
    ? 'Continue'
    : `Use ${selected.length === 1 ? 'this folder' : `${selected.length} folders`}`;

  return (
    <StepFrame
      title="Where is your code?"
      why={`Optional. Tell ${agentName} which folders hold your projects so it works in the right place.`}
      primary={{ label, pending: saving, pendingLabel: 'Saving…', disabled: selected.length === 0 }}
      onPrimary={() => save(selected)}
      secondary={<GhostLink onClick={() => save([])}>I don't write code — skip</GhostLink>}
    >
      {loading && <StatusLine tone="info">Looking for code folders under your home folder…</StatusLine>}

      {!loading && scanError && (
        <StatusLine tone="error">
          Couldn't look for repositories ({scanError}). Add a folder below or skip.{' '}
          <GhostLink onClick={() => setScanNonce(n => n + 1)}>Look again</GhostLink>
        </StatusLine>
      )}

      {!loading && !scanError && candidates.length === 0 && (
        <StatusLine tone="info">No git repositories found under {home || 'your home folder'}. Add a folder below, or skip.</StatusLine>
      )}

      {!loading && candidates.length > 0 && (
        <div role="group" aria-label="Folders with code" style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
          <FieldLabel>Found on this Mac</FieldLabel>
          {candidates.map((p, i) => {
            const on = chosen.has(p);
            return (
              <label key={p} style={{
                display: 'flex', alignItems: 'center', gap: space.lg, cursor: 'pointer',
                padding: `${space.md}px ${space.lg}px`, borderRadius: radius.md,
                background: on ? colors.cyanSoft : colors.fillSubtle,
              }}>
                <input type="checkbox" checked={on} onChange={() => toggle(p)}
                  {...(i === 0 ? { 'data-autofocus': '' } : {})}
                  style={{ accentColor: colors.cyan, width: 16, height: 16, flexShrink: 0 }} />
                <span style={{ fontFamily: font.mono, fontSize: textSize.small, color: on ? colors.text : colors.textMuted, wordBreak: 'break-all' }}>{p}</span>
              </label>
            );
          })}
        </div>
      )}

      {!loading && (
        <div>
          <FieldLabel htmlFor={manualId}>Add a folder</FieldLabel>
          <div style={{ display: 'flex', gap: space.md }}>
            <Input id={manualId} value={manual}
              onChange={v => { setManual(v); setCheck(null); }}
              onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); void addManual(); } }}
              placeholder="Folder path" autoFocus={candidates.length === 0} />
            <SecondaryButton onClick={addManual} disabled={checking || !manual.trim()}>{checking ? 'Checking…' : 'Add'}</SecondaryButton>
          </div>
          {check && !check.exists && (
            <StatusLine tone="error" style={{ marginTop: space.sm }}>There's no folder at {check.resolved}. Check the spelling.</StatusLine>
          )}
          {check?.exists && !check.has_repositories && (
            <StatusLine tone="warn" style={{ marginTop: space.sm }}>Added {check.resolved}, though there are no git repositories in it yet.</StatusLine>
          )}
        </div>
      )}

      {saveError && <StatusLine tone="error">Couldn't save that ({saveError}). Try again, or skip and set it in Settings.</StatusLine>}
    </StepFrame>
  );
}
