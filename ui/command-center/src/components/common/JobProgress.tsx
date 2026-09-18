/**
 * `<JobProgress>` — how a long-running job looks, said the same way everywhere.
 *
 * `useLongRunningJob` owns the phases; this owns their appearance, so there is
 * one place to change what "still working" looks like and one place to change
 * what a failure looks like. The look is lifted from the wizard's Ollama pull
 * (`wizard/MomentHardware.tsx:327-353`), which is where the determinate bar,
 * the truncated live status and the mono percentage were first drawn.
 *
 * ── THE CONTRACT ────────────────────────────────────────────────────────────
 *
 * - **Idle renders nothing.** The trigger button is the idle state; a job strip
 *   that is always on screen would make "not started" and "finished" look alike.
 * - **`percent === null` means the backend never said how big the work is**, and
 *   the bar is drawn indeterminate — a sweeping band, not a fabricated fill.
 *   Nothing here invents progress.
 * - **Every terminal phase is written out.** `succeeded` names what completed,
 *   `failed` prints the backend's own message, `stopped` says the user stopped
 *   it. A spinner that simply disappears is not a success state.
 * - **`data-phase` on the root** is the whole state machine, so a screenshot or
 *   a test can name the phase without reading the copy.
 *
 * ── FOR THE QWEN BRING-UP LANE (harness-dag D2) ─────────────────────────────
 *
 * `reading.stage` is rendered verbatim as the first tier of disclosure and
 * `reading.status` as the second, which is exactly D2's "parsed phase + raw
 * detail" split. Pass `onStop` only when the backend can genuinely cancel —
 * an unresponsive Stop is its own defect (U3 G3). If D2 needs a scrolling raw
 * log it should extend `JobReading` in the hook rather than fork this.
 */

import { type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from './Button';
import type { LongRunningJob } from '../../hooks/useLongRunningJob';

export interface JobProgressProps {
  job: LongRunningJob<unknown>;
  /** What the work is, in the imperative — "Scanning the universe". Used as
   *  the fallback headline for every phase the job did not name itself. */
  label: string;
  /** Wire it only when the backend can really cancel. Omitted = no Stop shown. */
  onStop?: () => void;
  /** Overrides "run it again" for the failed/stopped phases. Defaults to
   *  `job.start`; pass `null` to hide the retry entirely. */
  onRetry?: (() => void) | null;
  style?: CSSProperties;
}

/** Truncated the way the wizard truncates: a status line is a signal, not prose. */
function short(text: string, max = 60): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

export function JobProgress({ job, label, onStop, onRetry, style }: JobProgressProps) {
  const { colors } = useTheme();
  const { phase, reading, percent } = job;

  if (phase === 'idle') return null;

  const retry = onRetry === null ? null : (onRetry ?? (() => { void job.start(); }));
  const determinate = percent !== null;

  const line: CSSProperties = {
    display: 'flex', alignItems: 'center', gap: space.md,
    fontFamily: font.body, fontSize: textSize.micro,
  };

  return (
    <div
      data-testid="job-progress"
      data-phase={phase}
      role="status"
      aria-live="polite"
      style={{ display: 'flex', flexDirection: 'column', gap: space.sm, ...style }}
    >
      {job.running && (
        <>
          <div style={{
            width: '100%', height: 6, borderRadius: radius.pill,
            background: colors.surfaceHi, overflow: 'hidden',
          }}>
            <div
              data-testid="job-progress-bar"
              data-determinate={determinate ? 'true' : 'false'}
              className={determinate ? undefined : 'pa-job-sweep'}
              style={{
                width: determinate ? `${percent}%` : '35%',
                height: '100%', borderRadius: radius.pill,
                background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`,
                transition: determinate ? 'width 300ms ease-out' : undefined,
              }}
            />
          </div>
          <div style={{ ...line, justifyContent: 'space-between' }}>
            <span style={{ fontFamily: font.mono, color: colors.textMuted, minWidth: 0 }}>
              {short([reading.stage, reading.status ?? label].filter(Boolean).join(' · '))}
            </span>
            <span style={{ display: 'flex', alignItems: 'center', gap: space.md, flexShrink: 0 }}>
              {determinate && (
                <span
                  data-testid="job-progress-percent"
                  style={{ fontFamily: font.mono, color: colors.cyan, fontVariantNumeric: 'tabular-nums' }}
                >
                  {percent}%
                </span>
              )}
              {onStop && (
                <Button
                  colors={colors}
                  variant="bare"
                  data-testid="job-progress-stop"
                  onClick={onStop}
                  minPendingMs={0}
                  flashSuccess={false}
                  style={{ fontSize: textSize.micro, color: colors.textMuted }}
                >
                  Stop
                </Button>
              )}
            </span>
          </div>
        </>
      )}

      {phase === 'succeeded' && (
        <div style={{ ...line, color: colors.success }}>
          <span aria-hidden="true">✓</span>
          <span>{job.summary ?? `${label} — done`}</span>
        </div>
      )}

      {(phase === 'failed' || phase === 'stopped') && (
        <div style={{ ...line, flexWrap: 'wrap' }}>
          <span style={{ color: phase === 'failed' ? colors.danger : colors.textMuted }}>
            {phase === 'failed'
              // The backend's own words. A generic "something went wrong" here
              // would be the silent catch this whole primitive exists to ban.
              ? (job.error ?? `${label} failed`)
              : `Stopped — ${label.toLowerCase()} did not finish.`}
          </span>
          {retry && (
            <Button
              colors={colors}
              variant="bare"
              data-testid="job-progress-retry"
              onClick={retry}
              style={{ fontSize: textSize.micro, color: colors.cyan }}
            >
              Try again
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
