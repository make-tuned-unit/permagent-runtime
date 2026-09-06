import { useEffect, useMemo, useState } from 'react';
import { api, type HarnessRunView } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { font, radius, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';

const POLL_MS = 4_000;

/**
 * One calm escalation inside Build. A zero-token classifier on the daemon
 * recommends the Council; only this explicit click may start the paid pass.
 * Dismissal is per-run and local, so a heartbeat cannot turn into a nag loop.
 */
export function CouncilEscalation() {
  const { colors } = useTheme();
  const [runs, setRuns] = useState<HarnessRunView[]>([]);
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set());
  const [state, setState] = useState<'idle' | 'starting' | 'started' | 'error'>('idle');
  const [error, setError] = useState('');

  useEffect(() => {
    let alive = true;
    const refresh = () => {
      void api.getActiveHarnessRuns()
        .then(next => { if (alive) setRuns(next); })
        .catch(() => { /* Build remains usable when the daemon is restarting. */ });
    };
    refresh();
    const timer = window.setInterval(refresh, POLL_MS);
    return () => { alive = false; window.clearInterval(timer); };
  }, []);

  const candidate = useMemo(
    () => runs.find(run =>
      run.councilRecommendation?.recommended
      && Boolean(run.promptContext?.trim())
      && !dismissed.has(run.runId)),
    [runs, dismissed],
  );

  useEffect(() => {
    if (candidate) {
      setState('idle');
      setError('');
    }
  }, [candidate?.runId]);

  if (!candidate) return null;

  const dismiss = () => {
    setDismissed(previous => new Set(previous).add(candidate.runId));
  };
  const convene = async () => {
    setState('starting');
    setError('');
    try {
      await api.conveneCouncil(candidate.promptContext!, candidate.project);
      setState('started');
    } catch (cause) {
      setState('error');
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return (
    <div
      role="status"
      data-testid="council-escalation"
      style={{
        margin: '10px 18px 0', padding: '10px 12px', borderRadius: radius.md,
        border: `1px solid ${colors.purple ?? colors.borderHi}`,
        background: colors.surfaceHi, display: 'flex', alignItems: 'center', gap: 12,
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ color: colors.text, fontFamily: font.body, fontSize: textSize.caption, fontWeight: 700 }}>
          {state === 'started' ? 'Council is preparing the DAG' : 'This request may benefit from the Council'}
        </div>
        <div style={{ color: state === 'error' ? colors.danger : colors.textMuted, fontSize: textSize.micro, marginTop: 2 }}>
          {state === 'error'
            ? `Couldn’t start: ${error}`
            : state === 'started'
              ? 'It received the live request and project brief. Results will arrive through the Council report and approval flow.'
              : `${candidate.promptTitle} · ${candidate.councilRecommendation.reason} Your redacted request, project direction, and associated memories will be shared with the configured Council providers.`}
        </div>
      </div>
      {state !== 'started' && (
        <>
          <Button colors={colors} onClick={dismiss} disabled={state === 'starting'}>Not now</Button>
          <Button colors={colors} variant="primary" onClick={convene} disabled={state === 'starting'}>
            {state === 'starting' ? 'Starting…' : 'Convene Council'}
          </Button>
        </>
      )}
    </div>
  );
}
