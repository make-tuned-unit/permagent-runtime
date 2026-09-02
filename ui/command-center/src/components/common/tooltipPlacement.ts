/**
 * Viewport-edge placement for the shared Tooltip primitive.
 *
 * Prefer the caller's side; if that side would leave the bubble flush against
 * (or past) the viewport edge, flip to the opposite side. Horizontal midpoints
 * for top/bottom are clamped so a long label cannot spill past the window.
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

function sideFits(side: TooltipSide, anchor: Rect, viewport: Viewport): boolean {
  switch (side) {
    case 'top': return anchor.y >= FLIP_ROOM_PX;
    case 'bottom': return viewport.height - (anchor.y + anchor.height) >= FLIP_ROOM_PX;
    case 'left': return anchor.x >= FLIP_ROOM_PX;
    case 'right': return viewport.width - (anchor.x + anchor.width) >= FLIP_ROOM_PX;
  }
}

function positionFor(side: TooltipSide, anchor: Rect, gap: number, viewport: Viewport): { left: number; top: number } {
  const midX = anchor.x + anchor.width / 2;
  const midY = anchor.y + anchor.height / 2;
  switch (side) {
    case 'top':
      return {
        left: clamp(midX, VIEWPORT_EDGE_PX, viewport.width - VIEWPORT_EDGE_PX),
        top: anchor.y - gap,
      };
    case 'bottom':
      return {
        left: clamp(midX, VIEWPORT_EDGE_PX, viewport.width - VIEWPORT_EDGE_PX),
        top: anchor.y + anchor.height + gap,
      };
    case 'left':
      return {
        left: anchor.x - gap,
        top: clamp(midY, VIEWPORT_EDGE_PX, viewport.height - VIEWPORT_EDGE_PX),
      };
    case 'right':
      return {
        left: anchor.x + anchor.width + gap,
        top: clamp(midY, VIEWPORT_EDGE_PX, viewport.height - VIEWPORT_EDGE_PX),
      };
  }
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

/**
 * Pick a side and fixed-position coordinates for a tooltip bubble.
 *
 * Tip size is not required: top/bottom use a centred transform, and we flip
 * when the preferred side lacks `FLIP_ROOM_PX` of clearance. Callers that need
 * measured width (sidebar vs native webview) use `placeSidebarTooltip` instead.
 */
export function placeViewportTooltip(
  anchor: Rect,
  preferred: TooltipSide = 'top',
  gap: number = TOOLTIP_GAP_PX,
  viewport: Viewport = defaultViewport(),
): ViewportPlacement {
  const side = sideFits(preferred, anchor, viewport) ? preferred : flip(preferred);
  // If both sides are tight, still honour the (possibly flipped) choice — a
  // clamped position is better than hiding a label the user asked for.
  const { left, top } = positionFor(side, anchor, gap, viewport);
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
