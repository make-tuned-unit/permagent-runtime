import { useEffect, useState } from 'react';
import { duration, ease, font, textSize } from '../../styles/tokens';
import { Mobius, type MobiusState } from '../mobius/Mobius';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';
import { Button } from '../common/Button';
import { api } from '../../lib/api';

interface Props {
  /** Called once the daemon answers a config read. `wizardDone` mirrors
   *  `config.wizard_complete`, so the caller can route to the wizard or the
   *  running app without re-reading anything. */
  onReady: (wizardDone: boolean) => void;
}

/** Same budget the old inline retry loop used: ~10s of polling before this
 *  is an honest failure rather than a transient "still starting" blip. */
const MAX_ATTEMPTS = 10;
const RETRY_DELAY_MS = 1000;

type Status = 'connecting' | 'retrying' | 'failed';

/**
 * The daemon-connecting boot state — shown between the logo splash and the
 * running app while the frontend waits for the local daemon to answer.
 *
 * Previously this was a blank filled `<div>` for up to ~10s, and a silent
 * failure fell through to the wizard with no explanation. Neither state told
 * the person anything true about what was happening, which is the thing this
 * component exists to fix: honest copy for "connecting", "still waiting", and
 * — the state that did not exist before — "it did not come up", with a real
 * retry affordance instead of a guess. No fake progress bar: there is nothing
 * to show a fraction of, only an attempt count, which is what is shown.
 */
export function BootScreen({ onReady }: Props) {
  const { colors, reduceMotion } = useTheme();
  const [status, setStatus] = useState<Status>('connecting');
  const [attempt, setAttempt] = useState(0);
  const [mounted, setMounted] = useState(false);
  // Bumping this re-runs the connect effect below — the Retry button's entire
  // job.
  const [retryKey, setRetryKey] = useState(0);

  useEffect(() => {
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setStatus('connecting');
    setAttempt(0);

    (async () => {
      for (let n = 1; n <= MAX_ATTEMPTS; n += 1) {
        if (cancelled) return;
        setAttempt(n);
        setStatus(n === 1 ? 'connecting' : 'retrying');
        try {
          const config = await api.getConfig();
          if (cancelled) return;
          onReady((config as { config?: { wizard_complete?: boolean } })?.config?.wizard_complete === true);
          return;
        } catch {
          if (n < MAX_ATTEMPTS) await new Promise(r => setTimeout(r, RETRY_DELAY_MS));
        }
      }
      if (!cancelled) setStatus('failed');
    })();

    return () => { cancelled = true; };
  }, [onReady, retryKey]);

  const copy = status === 'failed'
    ? {
        title: 'Could not reach the daemon',
        sub: 'It may still be starting, or it failed to launch. Check that Permagent is running, then try again.',
      }
    : status === 'retrying'
      ? { title: 'Still connecting...', sub: `Waiting on the daemon (attempt ${attempt} of ${MAX_ATTEMPTS}).` }
      : { title: 'Connecting to Permagent...', sub: '' };

  // D9: spring token, under 500ms, gated on reduce motion — a plain instant
  // cut with the setting on, same as the splash it follows.
  const fade = reduceMotion ? 'none' : `opacity ${duration.smooth}ms ${ease.smooth}`;
  // Same Mobius reduce-motion mitigation as Splash.tsx: routing through
  // 'idle' uses the one state Mobius.tsx already gates correctly. See
  // LANE_LOG.md / PR body "requests for other lanes" for the underlying bug.
  const mobiusState: MobiusState = reduceMotion ? 'idle' : (status === 'failed' ? 'sleeping' : 'calibrating');

  return (
    <div
      style={{
        position: 'fixed', inset: 0, paddingTop: TITLEBAR_HEIGHT,
        background: `radial-gradient(ellipse 70% 50% at 50% 45%, ${colors.cyanWash} 0%, ${colors.bg} 70%)`,
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        opacity: mounted ? 1 : 0,
        transition: fade,
      }}
    >
      <Mobius size={120} state={mobiusState} glow={status === 'failed' ? 0 : 1} />
      <div style={{ marginTop: 24, textAlign: 'center', maxWidth: 320 }}>
        <p style={{
          fontFamily: font.display, fontSize: textSize.body, fontWeight: 600,
          color: colors.text,
        }}>
          {copy.title}
        </p>
        {copy.sub && (
          <p style={{
            fontFamily: font.body, fontSize: textSize.small, fontWeight: 400,
            color: colors.textMuted, marginTop: 8,
          }}>
            {copy.sub}
          </p>
        )}
        {status === 'failed' && (
          <div style={{ marginTop: 16, display: 'flex', justifyContent: 'center' }}>
            <Button colors={colors} variant="primary" onClick={() => setRetryKey(k => k + 1)}>
              Retry
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
