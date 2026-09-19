/**
 * Decision Inbox — home card ("decisions" registry entry, Lane L4).
 *
 * THE canonical rendering of the pending-decision count (J3). Home is the
 * landing page and "what needs me" is a landing-page fact; the other placements
 * (Settings → Autonomy, the chat dock, the World's Petition Basin, the Council
 * card) are labelled references to this one, and they take their words from the
 * shared `summarizeDecisions` rather than writing their own.
 *
 * Glanceable only: count + plain words; the whole card is tappable and opens
 * the inbox overlay. No actions live on the card itself.
 */

import { useEffect, useState } from 'react';
import { duration, ease, font, radius, space, tabularNums, textSize, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { summarizeDecisions } from '../decisions/summary';
import { usePersona } from '../../settings/useSettings';
import { useCommandCenter } from '../../../lib/store';
import { Tooltip } from '../../common/Tooltip';

/** A detail line that truncates instead of wrapping: a wrapped line is what
 *  pushed "Review →" off the bottom of a grid-sized tile.
 *
 *  `flexShrink: 0` matters as much as the ellipsis. `overflow: hidden` makes
 *  each line a scroll container, whose automatic min-height is 0, so with the
 *  default `flex-shrink: 1` a too-short tile would squash EVERY line a little
 *  and cut the bottom off the glyphs of all of them at once. Unshrinkable, the
 *  lines keep their full height — and `width: 100%` is half of how the body
 *  drops a line that does not fit whole (see the body's comment).
 *
 *  Each truncated line also carries its full text in the shared glass
 *  `Tooltip` (never a native `title=`, which the app has retired): an ellipsis
 *  hides the rest of the line, and the card's `aria-label` replaces its child
 *  text for assistive tech, so without it a cut line could not be read in full
 *  from the card at all. */
const oneLine = { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0, flexShrink: 0, width: '100%' } as const;

interface Props {
  /** Active-goal count from the shared useLiveGoals source, so this stat agrees
   *  with the In-Flight card and Hero status. Falls back to the decisions
   *  payload's own count when not supplied (e.g. standalone use). */
  activeCount?: number;
}

export function DecisionsCard({ activeCount }: Props = {}) {
  const { colors, reduceMotion } = useTheme();
  const inbox = useDecisions();
  const { data } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);

  const s = summarizeDecisions(data, agentName, activeCount);
  const { count } = s;
  const empty = s.allClear;

  // A reference elsewhere in the app asked to be brought here (Settings →
  // Autonomy's strip, per J3). Open the canonical surface, then clear the
  // request so it can never fire again later on its own.
  const pendingOpen = useCommandCenter(st => st.pendingDecisionInbox);
  const clearPendingDecisionInbox = useCommandCenter(st => st.clearPendingDecisionInbox);
  useEffect(() => {
    if (!pendingOpen) return;
    setOpen(true);
    clearPendingDecisionInbox();
  }, [pendingOpen, clearPendingDecisionInbox]);

  return (
    <>
      <div
        data-testid="decisions-card"
        role="button"
        tabIndex={0}
        aria-label="Open the decision inbox"
        onClick={() => setOpen(true)}
        onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setOpen(true); } }}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => { setHover(false); setPressed(false); }}
        onPointerDown={() => setPressed(true)}
        onPointerUp={() => setPressed(false)}
        style={{
          height: '100%', boxSizing: 'border-box',
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${hover ? colors.borderHi : colors.border}`,
          // Press settles back to the resting raised step; hover lifts one
          // step (overlay) — the same physical "pick up / put down" the drag
          // handle uses, just without the scale (D10).
          boxShadow: pressed
            ? colors.elevationRaised
            : hover
              ? [colors.elevationOverlay, colors.cardHighlight].filter(Boolean).join(', ')
              : [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
          padding: `${space.xxxl}px`,
          display: 'flex', flexDirection: 'column',
          overflow: 'hidden',
          cursor: 'pointer',
          transition: reduceMotion ? 'none' : `border-color ${duration.fast}ms ${ease.out}, box-shadow ${duration.fast}ms ${ease.out}`,
        }}
      >
        {/* Kicker */}
        <div style={{
          fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
          letterSpacing: '0.10em', textTransform: 'uppercase',
          color: colors.textDim, marginBottom: space.sm,
        }}>
          Needs you
        </div>

        {empty ? (
          // One line and a small tick, top-aligned. "All clear" is genuinely
          // low-information — it should cost a line, not a whole card.
          <div style={{ display: 'flex', alignItems: 'center', gap: space.md, paddingTop: space.xs }}>
            <span style={{
              width: 18, height: 18, borderRadius: '50%', flexShrink: 0,
              background: colors.success + '26', color: colors.success,
              display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: textSize.micro,
            }}>✓</span>
            <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>
              {s.allClearLabel}
            </span>
          </div>
        ) : (
          <>
            {/* The card's height is set by the dashboard grid, not by its
                content, and the card clips (`overflow: hidden`) so a tile never
                spills onto its neighbour. With every detail line present the
                stack was taller than the tile and "Review →" — the one line
                that says what tapping does — was the line that got clipped at
                the bottom edge. So the details live in a region that is allowed
                to shrink (`flex: 1`, `minHeight: 0`) and clips its own overflow
                with ellipses, and "Review →" sits outside it and never shrinks.

                A plain clip was not enough: measured in WebKit across tile
                heights, the body's bottom edge fell mid-line at most of them, so
                the last visible line lost the bottom of its glyphs. So the body
                wraps as a COLUMN: a line that does not fit whole moves to the
                next column, and every item is the body's full width, so that
                column starts past the right edge and is clipped away entirely.
                A first item never wraps, so the zero-height spacer below goes
                first — without it the big count would be sliced instead of
                dropped when the tile is too short even for it. */}
            <div
              data-testid="decisions-card-body"
              style={{ flex: '1 1 auto', minHeight: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column', flexWrap: 'wrap', alignContent: 'flex-start' }}
            >
              <span aria-hidden="true" data-testid="decisions-card-wrap-anchor" style={{ width: '100%', height: 0, flexShrink: 0 }} />
              {/* Stat-style count */}
              <div style={{
                fontFamily: font.display, fontSize: textSize.display, fontWeight: 600,
                letterSpacing: '-0.02em', ...tabularNums,
                // The ramp's own display line height. Without it the count
                // inherited the page's 1.5 (48px), half a pixel taller than the
                // body of a two-row tile (47.5px measured in WebKit), so the
                // wrap would drop the count from exactly the tile it fits in.
                lineHeight: type.display.lineHeight,
                color: count > 0 ? colors.cyan : colors.text,
                flexShrink: 0, width: '100%',
              }}>
                {data === null ? '—' : count}
              </div>
              <Tooltip content={s.headline}>
                <div style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text, marginTop: space.xxs, ...oneLine }}>
                  {s.headline}
                </div>
              </Tooltip>
              {s.oldestLabel && (
                <Tooltip content={s.oldestLabel}>
                  <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, marginTop: space.sm, ...oneLine }}>
                    {s.oldestLabel}
                  </div>
                </Tooltip>
              )}
              {s.attentionLabel && (
                // Was a raw hex, hardcoded regardless of theme — the one gap in
                // this screen's otherwise-clean token grade. "Needs your
                // attention" is the warning semantic, not a bespoke amber.
                <Tooltip content={s.attentionLabel}>
                  <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.warning, marginTop: space.sm, ...oneLine }}>
                    {s.attentionLabel}
                  </div>
                </Tooltip>
              )}
              {s.handledLabel && (
                <Tooltip content={s.handledLabel}>
                  <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginTop: space.lg, ...oneLine }}>
                    {s.handledLabel} →
                  </div>
                </Tooltip>
              )}
            </div>
            <div
              data-testid="decisions-card-review"
              style={{ flexShrink: 0, paddingTop: space.sm, fontFamily: font.body, fontSize: textSize.caption, fontWeight: 500, color: colors.cyan }}
            >
              Review →
            </div>
          </>
        )}
      </div>

      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}
