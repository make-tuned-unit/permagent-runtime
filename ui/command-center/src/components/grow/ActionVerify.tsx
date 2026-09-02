/**
 * "Verify change" plus everything the verdict has to say — the honest half of
 * an action card.
 *
 * Split out of GrowView.tsx (R9), unchanged. It keeps its own file because the
 * rules it encodes are pinned line-by-line against the source by
 * `growActions.verify.test.ts`, and because a verify result that outlives the
 * thing it was about is the most damaging thing this surface can show.
 */

import { useCallback, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { Button } from '../common/Button';
import { growSmall } from './growStyles';
import {
  FINAL_WINDOW_DAYS,
  FIRST_WINDOW_DAYS,
  TARGET_METRICS,
  verdictMeta,
  verifiedByMeta,
  windowDueAt,
} from './growthWindows';
import type { GrowthAction, GrowthVerifyResponse } from './growTypes';

/**
 * "Verify change" plus everything the verdict has to say — the honest half of
 * the card.
 *
 * Its own component, with its own state, for the reason
 * `analyticsPanelScope.test.ts` exists: a verify result that outlives the thing
 * it was about is the most damaging thing this surface can show. Keyed on the
 * action id by the caller, it is remounted whenever the action it describes
 * changes, so no stale verdict can be inherited.
 */
export function ActionVerify({
  projectId, action, colors, onChanged, readOnly = false,
}: {
  projectId: string;
  action: GrowthAction;
  colors: ThemeColors;
  /** Refetch the board. A pass moves the card from Actions to Completed, so
   *  the parent has to re-read rather than this component patching its own
   *  `result` state — that was the bug: the card verified on the daemon but
   *  visibly stayed in the suggestion list because nothing here ever told the
   *  parent to look again. Mirrors `ActionCard`'s `move()`. */
  onChanged: () => void;
  /**
   * The archived shelf. Every CONTROL disappears — a filed action is a record,
   * not a thing still asking to be done — but the verdict does not. Suppressing
   * the whole component instead would hide the measured outcome of exactly the
   * actions the archive exists to keep as data points, which is the opposite of
   * what filing one away is supposed to mean.
   */
  readOnly?: boolean;
}) {
  const [result, setResult] = useState<GrowthVerifyResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [metric, setMetric] = useState('');
  const [dir, setDir] = useState('');

  const identity = result?.identity ?? action.identity ?? null;

  const verify = useCallback((body: Record<string, unknown>) => {
    if (!identity) return;
    setBusy(true);
    apiFetch<GrowthVerifyResponse>(
      `/api/projects/${encodeURIComponent(projectId)}/growth-actions/`
      + `${encodeURIComponent(identity.id)}/verify`,
      { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) },
    )
      .then((res) => {
        setResult(res);
        // Verification moves the card between two lists (Actions →
        // Completed), so a local setResult alone decorates a card that is now
        // sitting in the wrong list — refetch tells the parent to look again.
        // Only on an actual pass: a failed or self-attest-eligible check
        // leaves the card exactly where it is, so there is nothing to refetch
        // for.
        if (res.verified) onChanged();
      })
      // A thrown fetch becomes a rendered, honest result rather than a dead
      // button — the rule the first-party install check already follows
      // (runVerify's catch, this file). A verify control that silently does
      // nothing is worse than one that says it failed.
      .catch((e) => setResult({
        verified: false,
        identity: null,
        checks: [],
        reason: `Could not run the check: ${e instanceof Error ? e.message : String(e)}`,
      }))
      .finally(() => setBusy(false));
  }, [projectId, identity, onChanged]);

  const rule: CSSProperties = {
    marginTop: 10, paddingTop: 8, borderTop: `1px solid ${colors.border}`,
    display: 'flex', flexDirection: 'column', gap: 8,
  };
  const label: CSSProperties = {
    fontFamily: font.mono, fontSize: 9, letterSpacing: '0.08em',
    textTransform: 'uppercase', color: colors.textDim,
  };
  // Set through `--pa-btn-*`: an inline `background`/`color` would outrank
  // `.pa-btn:hover`, and the dimming these carried by hand is now what
  // `.pa-btn:disabled` and `[data-pending]` say for themselves.
  const button = growSmall(colors);
  const select: CSSProperties = {
    background: colors.bgDeeper, border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '3px 6px', color: colors.text,
    fontFamily: font.body, fontSize: textSize.micro,
  };

  // On the shelf there is nothing to say unless a check actually confirmed
  // something: the controls are gone, so "this can't be verified" would be a
  // prompt to do something the card no longer offers.
  if (readOnly && (!identity || !identity.verifiedBy)) return null;

  // No row, no identity, nothing to attach a verdict to. Said out loud rather
  // than rendered as a missing button, which is indistinguishable from a
  // feature that was never built.
  if (!identity) {
    return (
      <div style={rule}>
        <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
          This action has no saved record yet, so it can’t be verified. Run “Review again” to
          save it.
        </span>
      </div>
    );
  }

  const provenance = verifiedByMeta(identity.verifiedBy);
  const target = identity.targetMetric
    ? TARGET_METRICS.find((m) => m.value === identity.targetMetric)?.label ?? identity.targetMetric
    : null;

  // The agent predicted this only when BOTH halves are present. A metric with
  // no direction is not a prediction — "bounce rate moves" is true either way —
  // so a half-filled pair falls back to asking rather than guessing "up".
  const predicted = !!identity.targetMetric && !!identity.targetDir;
  const predictedLabel = target ?? identity.targetMetric;

  // The claim every verify call must carry. The row's own pre-registration wins
  // whenever it exists; the selects below only ever fill in for a row that has
  // none. Without this, the self-attest and re-check buttons — which now render
  // for a predicted action too — would post `targetMetric: ''` and take a 400
  // from `parse_target`, which reads on screen as "the check is broken".
  const targetBody = (): Record<string, unknown> => {
    if (identity.targetMetric && identity.targetDir) {
      return { targetMetric: identity.targetMetric, targetDir: identity.targetDir };
    }
    return metric && dir ? { targetMetric: metric, targetDir: dir } : {};
  };

  return (
    <div style={rule}>
      {identity.verifiedBy ? (
        <>
          {/* HOW it was checked, never just THAT it was. A commit and a
              self-report are different claims, so they get different colour,
              different border and different words. */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
            border: `1px ${provenance.checked ? 'solid' : 'dashed'} ${provenance.checked ? colors.success : colors.warning}`,
            borderRadius: radius.sm, padding: '2px 8px',
            color: provenance.checked ? colors.success : colors.warning,
            ...label,
          }}>
            <span>{provenance.checked ? '✓' : '✎'}</span>
            <span>{provenance.label}</span>
          </div>
          {target && identity.targetDir && (
            <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
              Pre-registered before the baseline was frozen: {target} should go{' '}
              {identity.targetDir}
              {identity.verifiedAt && ` · verified ${new Date(identity.verifiedAt).toLocaleDateString()}`}
            </span>
          )}

          {identity.outcomes.map((o) => {
            const meta = verdictMeta(o.verdict, colors);
            // A percentage next to "not enough data to say" is the exact
            // failure the proposal names — "'this helped, +12%' off 40
            // pageviews is not measuring; it is pattern-matching noise and
            // presenting it as evidence" (proposal:35-39). So the number only
            // appears where a verdict actually rests on it.
            const showsDelta = (o.verdict === 'helped' || o.verdict === 'hindered')
              && o.deltaPct !== null;
            return (
              <div key={o.windowDays} style={{
                background: colors.bgDeeper, border: `1px solid ${colors.border}`,
                borderRadius: radius.md, padding: 10,
                display: 'flex', flexDirection: 'column', gap: 4,
              }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  <span style={{
                    ...label, color: meta.color, border: `1px solid ${meta.color}`,
                    borderRadius: radius.pill, padding: '1px 7px',
                  }}>{meta.label}</span>
                  <span style={{ ...label }}>{o.windowDays}-day window</span>
                  {o.windowDays < FINAL_WINDOW_DAYS && (
                    // Proposal open decision 2: early windows are read, but
                    // labelled provisional rather than presented as settled.
                    <span style={{ ...label, color: colors.textDim }}>provisional</span>
                  )}
                  {showsDelta && (
                    <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: meta.color }}>
                      {o.deltaPct! > 0 ? '+' : ''}{(o.deltaPct! * 100).toFixed(0)}%
                    </span>
                  )}
                </div>
                {/* The rationale is body text, always. It carries the numbers
                    the verdict rests on, and a verdict whose reasoning is
                    hidden in a tooltip cannot be argued with. */}
                <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>
                  {o.rationale}
                </div>
                {o.confounders.length > 0 && (
                  <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
                    Overlapping changes: {o.confounders.map((c) => c.title).join(', ')}
                  </div>
                )}
              </div>
            );
          })}

          {identity.outcomes.length === 0 && (() => {
            // Empty here means "not due yet", not "nothing found". Saying which
            // is the difference between a feature that is working and one that
            // looks broken.
            const due = windowDueAt(identity.verifiedAt, FIRST_WINDOW_DAYS);
            return (
              <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                Measuring. The first {FIRST_WINDOW_DAYS}-day reading is due
                {due ? ` ${due.toLocaleDateString()}` : ''}, then {14} and {FINAL_WINDOW_DAYS} days.
              </span>
            );
          })()}

          {/* The evidence a check found is NOT persisted — there is no
              `verified_detail` column — so a reload leaves the badge above with
              nothing behind it. This recovers it by re-running the checks.

              It is safe because `verify_mode` (growth_actions.rs) returns
              `Recheck` once `verified_at` is set and the handler then skips BOTH
              writes, returning the stored identity and the frozen baseline. It
              is NOT safe "by construction": only `baseline_json` coalesces.
              `record_verification` also writes `status = 'verified'` and
              `verified_at = now`, and `verified_at` is the pivot
              `metrics::pivot_date` measures every comparison window from — so
              without that guard this button would slide the after-windows
              forward against a baseline frozen days earlier and drag a judged
              action back into measurement. Do not delete one half without the
              other. */}
          {!readOnly && (
            <Button
              colors={colors}
              onClick={() => verify(targetBody())}
              disabled={busy}
              pending={busy}
              style={{ ...button, alignSelf: 'flex-start' }}
            >{busy ? 'Re-checking…' : 'Re-check'}</Button>
          )}
        </>
      ) : (
        <>
          {/* Pre-registration is a gate, not a form field: the backend refuses a
              verify without a target (growth_actions.rs) so a metric cannot be
              chosen once the result is visible.

              WHO fills it in is the point. The agent recommended this action, so
              the agent states what it expects to move — that claim is what the
              7/14/28-day sweep grades it against. Asking the user to supply it
              inverted the loop: they would be answering the question they came
              here to be advised on, and there would be no prediction of the
              agent's left to be right or wrong. The selects below are now the
              FALLBACK, for an action whose agent declined to predict (or one
              suggested before predictions existed). */}
          {predicted ? (
            <>
              <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                I expect this to move{' '}
                <strong style={{ color: colors.text }}>{predictedLabel}</strong>{' '}
                <strong style={{ color: colors.text }}>
                  {identity.targetDir === 'down' ? 'down' : 'up'}
                </strong>. I’ll check at 7, 14 and 28 days and record whether I was right.
              </span>
              {/* The one control here on purpose. There used to be a
                  "Measure something else" button beside it that revealed the
                  selects below and let the user replace the agent's target.
                  It is gone: the target is the AGENT's prediction and this loop
                  exists to grade the agent, so measuring a claim the agent
                  never made produces a verdict about nobody — the exact
                  unfalsifiability the pre-registration gate was built to stop.
                  The selects survive only for a row that genuinely carries no
                  prediction. */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                <Button
                  colors={colors}
                  onClick={() => verify(targetBody())}
                  disabled={busy}
                  pending={busy}
                  style={button}
                >{busy ? 'Checking…' : 'I did this — start measuring'}</Button>
              </div>
            </>
          ) : (
            <>
              <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                {identity.targetMetric || identity.targetDir
                  ? 'I couldn’t say what this should move, so pick the metric before checking it.'
                  : 'Say what this should move before checking it — a metric picked after the result is known can’t be wrong.'}
              </span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                <select
                  aria-label="Target metric"
                  value={metric}
                  onChange={(e) => setMetric(e.target.value)}
                  style={select}
                >
                  <option value="">what should move…</option>
                  {TARGET_METRICS.map((m) => (
                    <option key={m.value} value={m.value}>{m.label}</option>
                  ))}
                </select>
                <select
                  aria-label="Target direction"
                  value={dir}
                  onChange={(e) => setDir(e.target.value)}
                  style={select}
                >
                  <option value="">which way…</option>
                  <option value="up">should go up</option>
                  <option value="down">should go down</option>
                </select>
                <Button
                  colors={colors}
                  onClick={() => verify({ targetMetric: metric, targetDir: dir })}
                  disabled={busy || !metric || !dir}
                  pending={busy}
                  style={button}
                >{busy ? 'Checking…' : 'Verify change'}</Button>
              </div>
            </>
          )}
        </>
      )}

      {/* OUTSIDE the verified/not-verified ternary on purpose. This used to
          render only under `result && !result.verified`, so a PASS showed a
          bare badge and threw away the one thing the user could audit — which
          commit, which path, which string on which page. A pass that also says
          "live page does not contain it" teaches something the badge hides. */}
      {result && (
        <div style={{
          background: colors.bgDeeper, border: `1px solid ${colors.border}`,
          borderRadius: radius.md, padding: 10,
          display: 'flex', flexDirection: 'column', gap: 6,
        }}>
          {/* "Could not confirm" is not "not done", and the checks say which
              one it was (growth_verify.rs:9-11). */}
          {/* "What confirmed it" only when something below actually did. A
              re-check of a self-attested action re-runs the real strategies and
              they can all come back empty — the action IS verified, on the
              user's word, and heading that list with "what confirmed it" would
              dress four failed checks up as corroboration. */}
          <span style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>
            {result.verified
              ? ((result.checks ?? []).some((c) => c.passed)
                ? 'What confirmed it'
                : 'What the checks found')
              : result.reason ?? 'Nothing could confirm the change landed.'}
          </span>
          {/* `?? []` because this block now renders on a PASS too, and a
              payload from an older daemon — or a truncated one — carries no
              `checks`. A missing list must cost the evidence line, not take the
              whole Grow tab down with a TypeError. */}
          {(result.checks ?? []).map((c) => (
            <div key={c.id} style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
              <span style={{ color: c.passed ? colors.success : colors.textDim }}>
                {c.passed ? '✓' : '·'}
              </span>{' '}
              {c.label} — {c.detail}
            </div>
          ))}
          {!result.verified && (
            <Button
              colors={colors}
              onClick={() => verify({ ...targetBody(), selfAttested: true })}
              disabled={busy}
              pending={busy}
              style={{ ...button, alignSelf: 'flex-start' }}
            >It did land — record my word</Button>
          )}
        </div>
      )}
    </div>
  );
}
