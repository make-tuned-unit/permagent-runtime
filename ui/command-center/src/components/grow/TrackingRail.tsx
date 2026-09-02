/**
 * The measurement rail on a Tracking card: the frozen baseline, and how far
 * through the 7/14/28-day windows this action is.
 *
 * Split out of GrowView.tsx (R9), unchanged.
 */

import type { CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import {
  metricValue,
  verdictMeta,
  verifiedByMeta,
  windowProgress,
} from './growthWindows';
import type { ActionIdentity } from './growTypes';

/**
 * The measurement rail on a Tracking card: the frozen baseline, and how far
 * through the 7/14/28-day windows this action is.
 *
 * `ActionVerify` already renders the verdicts themselves, so this deliberately
 * does not repeat them — it renders what the card was missing, which is the
 * "before" every verdict is computed against and the windows that have not
 * reported yet. An empty outcome list with no rail reads as "the measurement
 * found nothing"; the truth is almost always "it is not due until the 26th".
 */
export function TrackingRail({ identity, colors }: { identity: ActionIdentity; colors: ThemeColors }) {
  const metric = identity.targetMetric ?? identity.baseline?.metric ?? null;
  const progress = windowProgress(identity);
  const baselineByWindow = new Map(
    (identity.baseline?.windows ?? []).map((w) => [w.windowDays, w]),
  );

  const label: CSSProperties = {
    fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
    textTransform: 'uppercase', color: colors.textDim,
  };

  return (
    <div style={{
      marginTop: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      borderRadius: radius.md, padding: 10,
      display: 'flex', flexDirection: 'column', gap: 8,
    }}>
      {/* The receipt for "this shipped". A commit chip only when the check
          that passed was `git` — `verifiedCommit` is omitted from the JSON
          for every other strategy (routes/growth_actions.rs), so its absence
          here falls back to naming the strategy itself (`verifiedByMeta`)
          rather than rendering nothing, which would read as "we don't know
          how this was confirmed" when the truth is "not from a commit". */}
      {identity.verifiedBy && (
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap' }}>
          {identity.verifiedCommit ? (
            <span style={{
              fontFamily: font.mono, fontSize: 10, color: colors.text,
              border: `1px solid ${colors.border}`, borderRadius: radius.sm,
              padding: '1px 6px',
            }}>commit {identity.verifiedCommit.slice(0, 8)}</span>
          ) : (
            <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
              {verifiedByMeta(identity.verifiedBy).label}
            </span>
          )}
          {identity.verifiedAt && (
            <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
              verified {new Date(identity.verifiedAt).toLocaleDateString()}
            </span>
          )}
        </div>
      )}
      {/* Stored, not recomputed — see `verifiedDetail` on `ActionIdentity`. */}
      {identity.verifiedDetail && (
        <span style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          {identity.verifiedDetail}
        </span>
      )}

      <span style={label}>Measuring against</span>
      {identity.baseline ? (
        <span style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          Baseline frozen {new Date(identity.baseline.takenAt).toLocaleDateString()}. Windows
          start {identity.baseline.pivot} — the change day itself is in neither half.
        </span>
      ) : (
        // Never a zero. A baseline of nought would render as "there was no
        // traffic before the change", which is a claim nothing here can make.
        <span style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          No baseline was frozen for this action, so its windows cannot be compared.
        </span>
      )}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {progress.map((w) => {
          const before = baselineByWindow.get(w.days) ?? null;
          return (
            <div key={w.days} style={{
              display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap',
              fontSize: textSize.micro, color: colors.textDim,
            }}>
              <span style={{ ...label, minWidth: 52 }}>{w.days}-day</span>
              <span style={{
                color: w.state === 'judged' ? colors.text : colors.textDim,
              }}>
                {w.state === 'judged'
                  ? `read ${new Date(w.outcome!.judgedAt).toLocaleDateString()} — ${verdictMeta(w.outcome!.verdict, colors).label.toLowerCase()}`
                  : w.state === 'due'
                    // The sweep is nightly, so "due" is a real state and saying
                    // so is honest. Silence here reads as a stuck experiment.
                    ? 'window closed — the next nightly sweep will read it'
                    : `closes ${w.dueAt ? w.dueAt.toLocaleDateString() : 'once verified'}`}
              </span>
              {before && metric && (
                <span style={{ fontFamily: font.mono, color: colors.textDim }}>
                  before: {metricValue(metric, before.value)}
                  {before.denominator > 0 && metric === 'bounce_rate'
                    ? ` of ${before.denominator.toLocaleString()} sessions`
                    : ''}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
