import { memo, useLayoutEffect, useRef, useState } from 'react';
import { duration, ease, font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useOrchestratorName } from '../../world/shared/useOrchestratorName';
import { SectionTitle, EmptyNote } from '../atoms';
import { useCommandCenter } from '../../../lib/store';
import type { ActiveGoal } from '../../../lib/useLiveGoals';
import { autonomyChip, goalStateLabel } from '../../../lib/goalStateLabel';

import { Chip } from '../../common/Chip';
import { Tooltip } from '../../common/Tooltip';

/** A text line that truncates with an ellipsis instead of wrapping or being
 *  sliced by the tile edge. Every line of a row uses it: the title had an
 *  ellipsis before, but the "Ready · permag…" subtitle and the routing note
 *  did not, so the installed app showed them hard-cut mid-glyph (QA,
 *  qa-r1-02-inflight-crop.png). */
const oneLine = { whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 } as const;

/** The row tooltip's body. The shared bubble has no width cap and is
 *  `nowrap`; a held goal's note is a multi-paragraph plan, so the body caps
 *  its own width and wraps (breaking even a long path or hash) to stay
 *  readable on screen. */
const tipBody = {
  display: 'block', maxWidth: 360, whiteSpace: 'normal', overflowWrap: 'anywhere',
} as const;

/**
 * The height, from the top of the list, of the last goal row that fits
 * whole inside `available` pixels.
 *
 * The card's height is set by the dashboard grid, not by its content. A list
 * that simply fills that height is cut wherever the tile ends, which on Home
 * was straight through the second row's subtitle glyphs. Sizing the list's
 * viewport to a whole number of rows means the rows that do not fit are left
 * below the fold, reachable by scrolling, instead of half-drawn.
 *
 * `boxes` are the rows' offsets inside the list. If not even the first row
 * fits, there is nothing whole to show, so the list keeps the full height.
 */
export function wholeRowsHeight(
  available: number,
  boxes: ReadonlyArray<{ top: number; height: number }>,
): number {
  let best = 0;
  for (const b of boxes) {
    const bottom = b.top + b.height;
    if (bottom <= available && bottom > best) best = bottom;
  }
  return best > 0 ? best : available;
}

interface Props {
  goals: ActiveGoal[];
}

export const InFlightCard = memo(function InFlightCard({ goals }: Props) {
  const { colors } = useTheme();
  const persona = useOrchestratorName() ?? 'your agent';
  // The region the grid gives the list, and the list itself: measured so the
  // list's scroll viewport ends on a row boundary (see `wholeRowsHeight`).
  const regionRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const [viewportHeight, setViewportHeight] = useState<number | null>(null);
  useLayoutEffect(() => {
    const region = regionRef.current;
    const grid = gridRef.current;
    if (!region || !grid) return;
    const measure = () => {
      const boxes = Array.from(grid.children).map(c => {
        const el = c as HTMLElement;
        // offsetHeight rounds to whole pixels; a 70.4px row reported as 70
        // left the list 1px short of the last row, clipping its bottom
        // border (seen at real size). Round the painted height UP instead.
        const painted = Math.ceil(el.getBoundingClientRect().height);
        return { top: el.offsetTop, height: Math.max(el.offsetHeight, painted) };
      });
      // jsdom (and a tile not yet laid out) reports 0: leave the list at the
      // region's full height rather than collapse it to nothing.
      const next = region.clientHeight > 0 ? wholeRowsHeight(region.clientHeight, boxes) : null;
      setViewportHeight(prev => (prev === next ? prev : next));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    // The region resizes with the tile; the grid's rows change height when a
    // wider tile fits a second column or a row's note appears.
    const ro = new ResizeObserver(measure);
    ro.observe(region);
    ro.observe(grid);
    return () => ro.disconnect();
  }, [goals.length]);
  return (
    <div data-testid="inflight-card" style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="In flight" right={goals.length > 0 ? `${goals.length} active` : undefined} />
      {goals.length === 0 ? (
        <EmptyNote hint={`Goals ${persona} is working on appear here`}>
          Nothing in flight
        </EmptyNote>
      ) : (
        // `minHeight: 0` lets this region be exactly what the tile has left
        // under the title, whatever the list holds; it is what gets measured.
        <div ref={regionRef} data-testid="inflight-region" style={{ flex: '1 1 0', minHeight: 0, overflow: 'hidden' }}>
          <div
            data-testid="inflight-list"
            style={{
              height: viewportHeight ?? '100%',
              // Vertical only. Horizontal overflow is never a scroll: a row
              // wider than the tile is a layout bug, and letting the list scroll
              // sideways is what hid the right edge of every row on Home.
              overflowY: 'auto', overflowX: 'hidden',
              // A scrolled list comes to rest on a row start too. Because every
              // row is the same height (the note line is always reserved, see
              // GoalCard), the viewport is a whole number of row pitches, so
              // any row it snaps to shows whole rows and nothing sliced.
              scrollSnapType: 'y mandatory',
            }}
          >
            <div
              ref={gridRef}
              style={{
                display: 'grid',
                // `min(280px, 100%)`: a column is never wider than the tile.
                // A bare `minmax(280px, 1fr)` forced a 280px column into a
                // narrower Home tile, so each row overflowed to the right and
                // was cut through its title ("Find and analyz") with the
                // ellipsis drawn off-screen where nobody could see it.
                gridTemplateColumns: 'repeat(auto-fill, minmax(min(280px, 100%), 1fr))',
                gap: space.md,
                // offsetTop of each row is measured against the grid.
                position: 'relative',
              }}
            >
              {goals.map(goal => <GoalCard key={goal.id} goal={goal} />)}
            </div>
          </div>
        </div>
      )}
    </div>
  );
});

const GoalCard = memo(function GoalCard({ goal }: { goal: ActiveGoal }) {
  const { colors, reduceMotion } = useTheme();
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  // Clickable tile with no hover/press feedback before this (D10): the whole
  // card is a button and looked exactly like one that wasn't.
  const [hover, setHover] = useState(false);
  const [pressed, setPressed] = useState(false);
  const status = `${goalStateLabel({ stateBinding: goal.state, holdNote: goal.hold_note })}${goal.assigned_to ? ` · ${goal.assigned_to}` : ''}`;
  const note = goal.hold_note || goal.routing_note;
  return (
    // Every line of the row now truncates, so the tip carries the full text
    // of each one: without it a long title or note could not be read from
    // Home at all. The shared bubble is `nowrap` with no width cap, and a
    // hold note is not a label: it is the router's whole last plan, several
    // paragraphs with a "Tracked modifications:" git status appended. Left to
    // the bubble's defaults it became one line running off the screen, so
    // the tip bounds its own width and wraps, keeping the note's line breaks.
    <Tooltip content={
      <span data-testid="inflight-row-tip" style={tipBody}>
        <span style={{ display: 'block', fontWeight: 600 }}>{goal.title}</span>
        <span style={{ display: 'block' }}>{status}</span>
        {note && (
          <span data-testid="inflight-row-tip-note" style={{ display: 'block', whiteSpace: 'pre-wrap', marginTop: space.xs }}>
            {note}
          </span>
        )}
        <span style={{ display: 'block', opacity: 0.7, marginTop: space.xs }}>View goal detail</span>
      </span>
    }>
      <div
        data-testid="inflight-row"
        onClick={() => openGoalDetail(goal.project_id, goal.id)}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => { setHover(false); setPressed(false); }}
        onPointerDown={() => setPressed(true)}
        onPointerUp={() => setPressed(false)}
        style={{
          padding: `${space.md}px ${space.lg}px`, borderRadius: radius.md, cursor: 'pointer',
          boxSizing: 'border-box', minWidth: 0,
          scrollSnapAlign: 'start',
          background: pressed ? colors.fillActive : hover ? colors.fillHover : colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
          transform: !reduceMotion && pressed ? 'scale(0.98)' : 'scale(1)',
          transition: reduceMotion
            ? 'none'
            : `background ${duration.fast}ms ${ease.out}, transform ${duration.fast}ms ${ease.out}`,
        }}
      >
        {/* The row used to lead with an animated Mobius at `size={36}`. That
            prop is the logo's HEIGHT: the active frames are 1024x485, so it
            drew 76px wide. In Home's real ~180px tile the row has ~104px
            inside its padding, which left the text column ~16px, every line
            an ellipsis, and a 20px row padding made each row ~96px tall, so
            only one fit under the title. QA saw "2 active" over a box holding
            just the logo. The row is text only now, with tighter padding, so
            two rows fit a 230px tile and each line has the tile's width. */}
        <div style={{ display: 'flex', alignItems: 'center', gap: space.sm, minWidth: 0 }}>
          <div data-testid="inflight-row-text" style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontFamily: font.body, fontSize: textSize.small, fontWeight: 600, color: colors.text,
              ...oneLine,
            }}>{goal.title}</div>
            <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.cyan, ...oneLine }}>
              {status}
            </div>
            {/* Was free-wrapping: a long routing note made one row several
                lines taller than its neighbours, which is what left a row
                straddling the bottom of the tile. The line is also always
                there, blank when the goal has no note, so every row is the
                same height. The whole-row viewport is sized from the first
                rows and the list snaps row by row; if a note-less row were a
                line shorter, snapping to it would pull the next row's top
                slice into view, cut mid-glyph, the QA crop again after a
                scroll. */}
            <div
              data-testid="inflight-row-note"
              aria-hidden={note ? undefined : true}
              style={{
                fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted,
                marginTop: space.xxs, lineHeight: 1.4, ...oneLine,
              }}
            >
              {note || '\u00a0'}
            </div>
          </div>
          {/* Coordinator ruling (dagK): the same Auto-approve / Needs review
              chip the goal detail modal shows, off the same `auto_approve`
              field \u2014 autonomy visible without opening the goal. A sibling of
              `inflight-row-text`, not a fourth line inside it: that block's
              three lines (title/status/note) are the fixed unit the whole-row
              height math and truncation contract are built on. `quiet`:
              fifteen rows of this chip must read as a rhythm, not fifteen
              alerts (Chip.tsx's own density rule). */}
          <Chip kind="static" tone={autonomyChip(goal.auto_approve).tone} quiet style={{ flexShrink: 0 }}>
            {autonomyChip(goal.auto_approve).label}
          </Chip>
        </div>
      </div>
    </Tooltip>
  );
}, (a, b) =>
  a.goal.id === b.goal.id
  && a.goal.state === b.goal.state
  && a.goal.title === b.goal.title
  && a.goal.assigned_to === b.goal.assigned_to
  // The row renders both notes; comparing only the four fields above left a
  // newly held goal showing its old status until something else changed.
  && a.goal.hold_note === b.goal.hold_note
  && a.goal.routing_note === b.goal.routing_note
  && a.goal.project_id === b.goal.project_id,
);
