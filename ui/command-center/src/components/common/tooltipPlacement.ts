/**
 * Viewport-edge placement for the shared Tooltip primitive.
 *
 * Prefer the caller's side; if that side would leave the bubble flush against
 * (or past) the viewport edge, flip to the opposite side. Horizontal midpoints
 * for top/bottom are clamped so a long label cannot spill past the window.
 *
 * MEASURE THE BUBBLE WHEN YOU CAN. Both halves of this file used to guess:
 *
 *  1. `sideFits` compared the room on a side against a flat `FLIP_ROOM_PX`
 *     (48) instead of the bubble's own height. The chat dock header sits
 *     ~50px down, so "top" passed the 48px guess — and a label that wrapped to
 *     two lines (~60px) then rendered from y = 50 - 8 - 60 = -18: drawn UP
 *     THROUGH the header chrome and butted against the icon row it belongs to
 *     ("Open chat in its own window" over the mute / detach / close buttons,
 *     reported 2026-09-18). The same guess is wrong in the other direction —
 *     a trigger with 40px of clearance has room for a one-line bubble (28 + 8)
 *     yet got flipped DOWN onto the transcript.
 *
 *  2. `positionFor` clamped the MIDPOINT into the viewport while the bubble is
 *     centred on it with `translate(-50%)`, so half a wide label still spilled
 *     past the edge — despite this comment promising it could not. The dock's
 *     header buttons are flush right, which is exactly where that shows.
 *
 * So `tip` is optional but preferred: given real dimensions we flip on the
 * space the bubble actually needs and clamp its EDGES, not its centre. Without
 * it (first paint, before the bubble has been laid out) we fall back to the
 * old heuristic rather than refusing to place anything.
 *
 * Sidebar labels that must also avoid the native browser webview do NOT use
 * this — they keep `sidebar/tooltipPlacement.ts` (`placeSidebarTooltip`), which
 * inverts around `browserPaneRect`. The shared bubble still renders them.
 */

export type TooltipSide = 'top' | 'bottom' | 'left' | 'right';

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Viewport {
  width: number;
  height: number;
}

/** Measured bubble box. Omit it only while the bubble has not been laid out. */
export interface TipSize {
  width: number;
  height: number;
}

export interface ViewportPlacement {
  left: number;
  top: number;
  side: TooltipSide;
  /** Resting transform once the spring settles. */
  transform: string;
  /** Transform at animation frame 0 (nudge from the anchor). */
  fromTransform: string;
}

/** Gap between the trigger edge and the bubble. Matches space.md. */
export const TOOLTIP_GAP_PX = 8;

/** Soft margin from the viewport edge before we flip or clamp. */
export const VIEWPORT_EDGE_PX = 8;

/** Rough room needed on a side before we trust it without measuring the bubble. */
export const FLIP_ROOM_PX = 48;

function restingTransform(side: TooltipSide): string {
  switch (side) {
    case 'top': return 'translate(-50%, -100%)';
    case 'bottom': return 'translate(-50%, 0)';
    case 'left': return 'translate(-100%, -50%)';
    case 'right': return 'translate(0, -50%)';
  }
}

function enteringTransform(side: TooltipSide): string {
  switch (side) {
    case 'top': return 'translate(-50%, -100%) translateY(4px)';
    case 'bottom': return 'translate(-50%, 0) translateY(-4px)';
    case 'left': return 'translate(-100%, -50%) translateX(4px)';
    case 'right': return 'translate(0, -50%) translateX(-4px)';
  }
}

function flip(side: TooltipSide): TooltipSide {
  switch (side) {
    case 'top': return 'bottom';
    case 'bottom': return 'top';
    case 'left': return 'right';
    case 'right': return 'left';
  }
}

/**
 * Room a side must have. Measured: the bubble's own extent plus the gap it
 * keeps from the trigger and the margin it keeps from the window. Unmeasured:
 * the old flat guess, which is the only thing available on first paint.
 */
function roomNeeded(side: TooltipSide, gap: number, tip?: TipSize): number {
  if (!tip) return FLIP_ROOM_PX;
  const extent = side === 'top' || side === 'bottom' ? tip.height : tip.width;
  return extent + gap + VIEWPORT_EDGE_PX;
}

function sideFits(side: TooltipSide, anchor: Rect, viewport: Viewport, gap: number, tip?: TipSize): boolean {
  const need = roomNeeded(side, gap, tip);
  switch (side) {
    case 'top': return anchor.y >= need;
    case 'bottom': return viewport.height - (anchor.y + anchor.height) >= need;
    case 'left': return anchor.x >= need;
    case 'right': return viewport.width - (anchor.x + anchor.width) >= need;
  }
}

/**
 * Clamp a centre coordinate so the bubble's EDGES land inside the viewport.
 * `extent` is the bubble's size along that axis; half of it sits either side of
 * the returned value because of the `translate(-50%)` resting transform.
 */
function clampCentre(mid: number, extent: number | undefined, limit: number): number {
  const half = extent === undefined ? 0 : extent / 2;
  const lo = VIEWPORT_EDGE_PX + half;
  const hi = limit - VIEWPORT_EDGE_PX - half;
  // A bubble wider than the window cannot satisfy both edges; centre it rather
  // than pinning it to one side and hiding the rest of the label.
  if (lo > hi) return limit / 2;
  return clamp(mid, lo, hi);
}

function positionFor(
  side: TooltipSide,
  anchor: Rect,
  gap: number,
  viewport: Viewport,
  tip?: TipSize,
): { left: number; top: number } {
  const midX = anchor.x + anchor.width / 2;
  const midY = anchor.y + anchor.height / 2;
  switch (side) {
    case 'top':
      return {
        left: clampCentre(midX, tip?.width, viewport.width),
        // `translate(-100%)` hangs the bubble above this line, so the clamp is
        // a floor: never let its top edge climb past the window margin.
        top: tip ? Math.max(anchor.y - gap, VIEWPORT_EDGE_PX + tip.height) : anchor.y - gap,
      };
    case 'bottom':
      return {
        left: clampCentre(midX, tip?.width, viewport.width),
        top: tip
          ? Math.min(anchor.y + anchor.height + gap, viewport.height - VIEWPORT_EDGE_PX - tip.height)
          : anchor.y + anchor.height + gap,
      };
    case 'left':
      return {
        left: tip ? Math.max(anchor.x - gap, VIEWPORT_EDGE_PX + tip.width) : anchor.x - gap,
        top: clampCentre(midY, tip?.height, viewport.height),
      };
    case 'right':
      return {
        left: tip
          ? Math.min(anchor.x + anchor.width + gap, viewport.width - VIEWPORT_EDGE_PX - tip.width)
          : anchor.x + anchor.width + gap,
        top: clampCentre(midY, tip?.height, viewport.height),
      };
  }
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

/**
 * Pick a side and fixed-position coordinates for a tooltip bubble.
 *
 * Pass `tip` (the bubble's measured box) whenever it exists: the flip then
 * reflects the space the bubble really needs, and the clamps keep its edges
 * inside the window. Without it we fall back to `FLIP_ROOM_PX` and a centre-only
 * clamp, which is enough to get a first frame on screen but is what let a
 * two-line label paint over the chat header.
 *
 * Callers that must also dodge the native browser webview use
 * `placeSidebarTooltip` instead.
 */
export function placeViewportTooltip(
  anchor: Rect,
  preferred: TooltipSide = 'top',
  gap: number = TOOLTIP_GAP_PX,
  viewport: Viewport = defaultViewport(),
  tip?: TipSize,
): ViewportPlacement {
  // A zero box is "not laid out yet", not "a bubble with no size" — treat it as
  // unmeasured so we do not flip on a measurement that has not happened.
  const measured = tip && tip.width > 0 && tip.height > 0 ? tip : undefined;
  const preferredFits = sideFits(preferred, anchor, viewport, gap, measured);
  const opposite = flip(preferred);
  // Only flip if the other side is actually better. Flipping blindly moved the
  // label from one cramped side to a worse one.
  const side = preferredFits || !sideFits(opposite, anchor, viewport, gap, measured)
    ? preferred
    : opposite;
  // If both sides are tight, still honour the (possibly flipped) choice — a
  // clamped position is better than hiding a label the user asked for.
  const { left, top } = positionFor(side, anchor, gap, viewport, measured);
  return {
    left,
    top,
    side,
    transform: restingTransform(side),
    fromTransform: enteringTransform(side),
  };
}

function defaultViewport(): Viewport {
  if (typeof window === 'undefined') return { width: 1280, height: 800 };
  return { width: window.innerWidth, height: window.innerHeight };
}
