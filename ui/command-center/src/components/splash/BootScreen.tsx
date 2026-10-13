import { useEffect, useRef, useState } from 'react';
import { duration, ease, font, space, textSize } from '../../styles/tokens';
import { Mobius, type MobiusState } from '../mobius/Mobius';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';
import { Button } from '../common/Button';
import { api } from '../../lib/api';
import { getDaemonTokenError } from '../../lib/daemonAuth';
import { classifyStartupConfig } from '../../lib/startupConfig';
import { useDaemonFailure } from './useDaemonFailure';

interface Props {
  /** Called once the background service answers with a valid config: `app`
   *  when setup is complete, `wizard` for a first run. */
  onReady: (route: 'app' | 'wizard') => void;
  /** Called when it answers with a config that is not valid. That is not a
   *  first run, so it must not open setup over it. */
  onMalformed: () => void;
}

export const BOOT_DEADLINE_MS = 10_000;
export const BOOT_ATTEMPT_MS = 2_000;
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
  const failureDetail = useDaemonFailure();
  const [attempt, setAttempt] = useState(0);
  const [mounted, setMounted] = useState(false);
  // Bumping this re-runs the connect effect below — the Retry button's entire
  // job.
  const [retryKey, setRetryKey] = useState(0);

  useEffect(() => {
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, []);

  const callbacks = useRef({ onReady, onMalformed });
  callbacks.current = { onReady, onMalformed };
  const [remaining, setRemaining] = useState(10);

  useEffect(() => {
    const deadline = performance.now() + BOOT_DEADLINE_MS;
    let finished = false;
    let current: AbortController | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let attemptTimer: ReturnType<typeof setTimeout> | undefined;
    const stop = () => {
      finished = true;
      clearTimeout(deadlineTimer);
      clearInterval(countdown);
      clearTimeout(retryTimer);
      clearTimeout(attemptTimer);
      current?.abort();
    };
    const fail = () => { stop(); setRemaining(0); setStatus('failed'); };
    // Independent of fetch/auth/body parsing: even an implementation that ignores
    // AbortSignal cannot hold the recovery screen hostage. Only an explicit Retry
    // starts a new deadline, never a request retry or changed parent callback.
    const deadlineTimer = setTimeout(fail, BOOT_DEADLINE_MS);
    const countdown = setInterval(() => {
      setRemaining(Math.max(0, Math.ceil((deadline - performance.now()) / 1000)));
    }, 250);
    setStatus('connecting');
    setAttempt(0);
    setRemaining(10);

    const connect = async (n: number) => {
      if (finished) return;
      const left = deadline - performance.now();
      if (left <= 0) { fail(); return; }
      setAttempt(n);
      setStatus(n === 1 ? 'connecting' : 'retrying');
      current = new AbortController();
      const controller = current;
      try {
        const config = await Promise.race([
          api.getConfig(controller.signal),
          new Promise<never>((_, reject) => {
            controller.signal.addEventListener('abort', () => reject(new Error('Connection timed out')), { once: true });
            attemptTimer = setTimeout(() => controller.abort(), Math.min(BOOT_ATTEMPT_MS, left));
          }),
        ]);
        if (finished) return;
        if (performance.now() >= deadline) { fail(); return; }
        const route = classifyStartupConfig(config);
        stop();
        if (route === 'malformed') callbacks.current.onMalformed();
        else callbacks.current.onReady(route);
      } catch (e) {
        clearTimeout(attemptTimer);
        if (finished) return;
        if ((e as { status?: number })?.status === 401 && getDaemonTokenError()) {
          stop(); setStatus('unpaired'); return;
        }
        if (n >= MAX_ATTEMPTS || performance.now() >= deadline) { fail(); return; }
        retryTimer = setTimeout(() => void connect(n + 1), Math.min(RETRY_DELAY_MS, deadline - performance.now()));
      }
    };
    void connect(1);
    return stop;
  }, [retryKey]);

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
        <p role="status" aria-live="polite" style={{
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
        {(status === 'connecting' || status === 'retrying') && <p style={{ color: colors.textMuted, marginTop: space.md }}>Connection check: {remaining}s remaining</p>}
        {failureDetail && (status === 'connecting' || status === 'retrying' || status === 'failed') && (
          <p style={{
            fontFamily: font.body, fontSize: textSize.small, fontWeight: 400,
            color: colors.textMuted, marginTop: space.md,
          }}>
            Background service: {failureDetail}
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
