import { useEffect, useState } from 'react';
import { duration, ease, font, space, textSize } from '../../styles/tokens';
import { Mobius, type MobiusState } from '../mobius/Mobius';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';
import { Button } from '../common/Button';
import { api } from '../../lib/api';
import { getDaemonTokenError } from '../../lib/daemonAuth';
import { classifyStartupConfig } from '../../lib/startupConfig';

interface Props {
  /** Called once the background service answers with a valid config: `app`
   *  when setup is complete, `wizard` for a first run. */
  onReady: (route: 'app' | 'wizard') => void;
  /** Called when it answers with a config that is not valid. That is not a
   *  first run, so it must not open setup over it. */
  onMalformed: () => void;
}

/** Same budget the old inline retry loop used: ~10s of polling before this
 *  is an honest failure rather than a transient "still starting" blip. */
const MAX_ATTEMPTS = 10;
const RETRY_DELAY_MS = 1000;

type Status = 'connecting' | 'retrying' | 'failed' | 'unpaired';

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
export function BootScreen({ onReady, onMalformed }: Props) {
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
          const config: unknown = await api.getConfig();
          if (cancelled) return;
          const route = classifyStartupConfig(config);
          if (route === 'malformed') onMalformed();
          else onReady(route);
          return;
        } catch (e) {
          // No credential (or a dead pairing link): retrying cannot help, and
          // ten seconds of "still connecting" for a browser that was never
          // paired is a lie. Say it now (QA 2026-10-07 Q1/Q24).
          if ((e as { status?: number })?.status === 401 && getDaemonTokenError()) {
            if (!cancelled) setStatus('unpaired');
            return;
          }
          if (n < MAX_ATTEMPTS) await new Promise(r => setTimeout(r, RETRY_DELAY_MS));
        }
      }
      if (!cancelled) setStatus('failed');
    })();

    return () => { cancelled = true; };
  }, [onReady, onMalformed, retryKey]);

  const copy = status === 'unpaired'
    ? {
        title: /pairing link/i.test(getDaemonTokenError() ?? '')
          ? 'This pairing link is invalid or has expired'
          : "This browser isn't paired",
        sub: /pairing link/i.test(getDaemonTokenError() ?? '')
          ? 'Pairing links work once. Create a new one in Settings → Devices and open it in this browser.'
          : 'Open Permagent on your hub and pair this device from Settings → Devices.',
      }
    : status === 'failed'
    ? {
        title: "Couldn't reach Permagent's background service",
        sub: 'It may still be starting, or it failed to launch. Your setup and data are untouched. Try again in a moment.',
      }
    : status === 'retrying'
      ? { title: 'Still connecting…', sub: `Waiting for Permagent's background service (attempt ${attempt} of ${MAX_ATTEMPTS}).` }
      : { title: 'Connecting to Permagent…', sub: '' };

  // D9: spring token, under 500ms, gated on reduce motion — a plain instant
  // cut with the setting on, same as the splash it follows.
  const fade = reduceMotion ? 'none' : `opacity ${duration.smooth}ms ${ease.smooth}`;
  // No reduce-motion routing needed here: Mobius freezes every state under
  // the setting at the source (see the `motionDisabled` gate in Mobius.tsx),
  // so 'calibrating'/'sleeping' are safe to use directly.
  const mobiusState: MobiusState = status === 'failed' || status === 'unpaired' ? 'sleeping' : 'calibrating';

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
      <Mobius size={120} state={mobiusState} glow={status === 'failed' || status === 'unpaired' ? 0 : 1} />
      <div style={{ marginTop: space.huge, textAlign: 'center', maxWidth: 320 }}>
        <p style={{
          fontFamily: font.display, fontSize: textSize.body, fontWeight: 600,
          color: colors.text,
        }}>
          {copy.title}
        </p>
        {copy.sub && (
          <p style={{
            fontFamily: font.body, fontSize: textSize.small, fontWeight: 400,
            color: colors.textMuted, marginTop: space.md,
          }}>
            {copy.sub}
          </p>
        )}
        {(status === 'failed' || status === 'unpaired') && (
          <div style={{ marginTop: space.xxl, display: 'flex', justifyContent: 'center' }}>
            <Button colors={colors} variant="primary" onClick={() => setRetryKey(k => k + 1)}>
              Retry
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
