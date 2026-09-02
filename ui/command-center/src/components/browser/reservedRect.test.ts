/**
 * The sidebar hover label vs the native browser surface (reported 2026-08-19,
 * INVERTED 2026-09-01).
 *
 * "When the browser is full view and the terminal is toggled off, the tooltips
 * on the sidebar go beneath the browser window instead of over it."
 *
 * There is no CSS answer to that sentence: the tooltip is DOM inside the
 * shell's own webview; the browser is a native child WKWebView that macOS
 * composites above that webview's content, no `z-index` reaches it.
 *
 * The 2026-08-19 fix (`reserveFromLeft`, deleted with this file's rewrite)
 * had the tooltip publish its rect and had the Browser subtract it from the
 * native bounds on every hover. It worked, but it meant `update_browser_bounds`
 * — a REAL native call — could fire because someone hovered a sidebar icon,
 * and the file's own header warned that machinery of this shape invites
 * rewiring the bug back in. This suite now pins the OPPOSITE contract:
 *
 *   1. The browser's bounds computation does not know the sidebar tooltip
 *      exists — showing one calls `update_browser_bounds` zero additional
 *      times, and the rect it would send is bit-for-bit identical to the
 *      no-tooltip case (source guard on Browser.tsx).
 *   2. `placeSidebarTooltip` (sidebar/tooltipPlacement.ts) guarantees, by
 *      construction, that wherever it puts the label, the label's rect
 *      cannot intersect the browser's rect — in EITHER of the two layouts
 *      that made this bug reportable in the first place.
 *
 * The geometry fixtures are the real shell's: the collapsed rail is 64px
 * (`Sidebar.tsx` `const W = open ? 208 : 64`), and `BuildView` wraps the
 * terminal/browser split in `padding: '12px 18px'`, so a full-width browser
 * pane starts at 64 + 18 = 82.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  placeSidebarTooltip,
  boxIntersectsBrowser,
  NATURAL_MAX_WIDTH,
  MIN_RENDERABLE_WIDTH,
  type Rect,
} from '../sidebar/tooltipPlacement';

const SIDEBAR_W = 64;
const SIDEBAR_W_OPEN = 208;
const PANE_PAD_X = 18;
const WINDOW_W = 1440;
const PANE_TOP = 120;
const PANE_BOTTOM = 900;

/** A collapsed-rail row's own bounding rect (40px button, centred in the 64px
 *  column — `Sidebar.tsx`'s `width: 40, margin: '0 auto'` for `open: false`). */
function collapsedAnchor(rowTop: number): Rect {
  return { x: 12, y: rowTop, width: 40, height: 40 };
}

/** An open-rail row's own bounding rect (`calc(100% - 16px)` in a 208px
 *  column, `margin: '0 8px'`). */
function openAnchor(rowTop: number): Rect {
  return { x: 8, y: rowTop, width: 192, height: 40 };
}

/** The browser pane with the terminal HIDDEN: it owns the whole pane. */
const browserFullWidth: Rect = {
  x: SIDEBAR_W + PANE_PAD_X,
  y: PANE_TOP,
  width: WINDOW_W - SIDEBAR_W - 2 * PANE_PAD_X,
  height: PANE_BOTTOM - PANE_TOP,
};

/** Same layout, but with the rail OPEN — the tightest realistic case is
 *  collapsed, but the open rail is checked too since a hover fires in both. */
const browserFullWidthOpenRail: Rect = {
  x: SIDEBAR_W_OPEN + PANE_PAD_X,
  y: PANE_TOP,
  width: WINDOW_W - SIDEBAR_W_OPEN - 2 * PANE_PAD_X,
  height: PANE_BOTTOM - PANE_TOP,
};

/**
 * The browser pane with the terminal SHOWING. BuildView's Group is
 * `orientation="horizontal"`, so the terminal takes the left half and the
 * browser starts at the midpoint — well clear of the rail.
 */
const browserRightHalf: Rect = {
  x: SIDEBAR_W + PANE_PAD_X + (WINDOW_W - SIDEBAR_W - 2 * PANE_PAD_X) / 2,
  y: PANE_TOP,
  width: (WINDOW_W - SIDEBAR_W - 2 * PANE_PAD_X) / 2,
  height: PANE_BOTTOM - PANE_TOP,
};

describe('the reported case: browser full view, terminal toggled off, rail collapsed', () => {
  it('places a visible, non-degenerate label without ever reaching the browser rect', () => {
    for (const rowTop of [PANE_TOP - 40, PANE_TOP, 250, 400, 640, PANE_BOTTOM - 40, PANE_BOTTOM + 20]) {
      const placement = placeSidebarTooltip(collapsedAnchor(rowTop), browserFullWidth);
      expect(placement.visible).toBe(true);
      expect(placement.maxWidth).toBeGreaterThanOrEqual(MIN_RENDERABLE_WIDTH);
      expect(boxIntersectsBrowser(placement, browserFullWidth)).toBe(false);
    }
  });

  it('pulls the label back over the rail\'s own icon column rather than spill past the boundary', () => {
    // The natural position (anchor.right + 10 = 12+40+10 = 62) leaves almost
    // nothing before the browser's edge at 82 — not enough room at
    // PREFERRED_MIN_WIDTH, so the box must be pulled left of its natural spot.
    const placement = placeSidebarTooltip(collapsedAnchor(300), browserFullWidth);
    expect(placement.left).toBeLessThan(62);
    expect(placement.left).toBeGreaterThanOrEqual(0);
  });

  it('never lets the box reach the boundary, regardless of label length', () => {
    // The guarantee holds for ANY maxWidth this function could return — proven
    // structurally (left + maxWidth <= browserRect.x), not just for the one
    // label this suite happens to render.
    const placement = placeSidebarTooltip(collapsedAnchor(300), browserFullWidth);
    expect(placement.left + (placement.maxWidth ?? 0)).toBeLessThanOrEqual(browserFullWidth.x);
  });
});

describe('the same case with the rail OPEN', () => {
  it('also never intersects the browser rect', () => {
    for (const rowTop of [PANE_TOP, 300, 600, PANE_BOTTOM - 40]) {
      const placement = placeSidebarTooltip(openAnchor(rowTop), browserFullWidthOpenRail);
      expect(placement.visible).toBe(true);
      expect(boxIntersectsBrowser(placement, browserFullWidthOpenRail)).toBe(false);
    }
  });
});

describe('the spacious layouts are left alone', () => {
  it('does not constrain the label when the terminal is showing', () => {
    const placement = placeSidebarTooltip(collapsedAnchor(300), browserRightHalf);
    // Plenty of room (browserRightHalf starts around x=739) — the label gets
    // its natural, unconstrained position and width, same as before #1068.
    expect(placement.left).toBe(collapsedAnchor(300).x + collapsedAnchor(300).width + 10);
    expect(placement.maxWidth).toBe(NATURAL_MAX_WIDTH);
    expect(boxIntersectsBrowser(placement, browserRightHalf)).toBe(false);
  });

  it('does not constrain the label when no browser pane is being positioned at all', () => {
    const placement = placeSidebarTooltip(collapsedAnchor(300), null);
    expect(placement.maxWidth).toBe(NATURAL_MAX_WIDTH);
    expect(placement.visible).toBe(true);
  });

  it('is a placement problem, not a browser-geometry problem: the browser rect passed in is never read back different', () => {
    // Calling the placement function cannot mutate or otherwise imply a
    // change to the rect it was handed — there is no return value describing
    // a "moved" browser at all, unlike the deleted reserveFromLeft.
    const before = { ...browserFullWidth };
    placeSidebarTooltip(collapsedAnchor(300), browserFullWidth);
    expect(browserFullWidth).toEqual(before);
  });
});

// ── Source guards: the geometry above is only real if the shell is actually
// wired to it, in BOTH directions — the Browser must not read anything
// sidebar-shaped, and the tooltip must not write anything browser-shaped.
const BROWSER_TSX = readFileSync(new URL('./Browser.tsx', import.meta.url), 'utf8');
const TOOLTIP_TSX = readFileSync(new URL('../sidebar/SidebarTooltip.tsx', import.meta.url), 'utf8');

describe('update_browser_bounds is called zero times for a sidebar tooltip', () => {
  it('the reservation machinery is gone, not just unused', () => {
    expect(BROWSER_TSX).not.toContain('reserveFromLeft');
    expect(BROWSER_TSX).not.toContain('sidebarTooltipRect');
    expect(TOOLTIP_TSX).not.toContain('setSidebarTooltipRect');
    expect(TOOLTIP_TSX).not.toContain('setReservedRect');
  });

  it('syncBounds computes ONE rect and sends exactly that one, with nothing subtracted for a tooltip', () => {
    const syncBounds = BROWSER_TSX.slice(
      BROWSER_TSX.indexOf('const syncBounds = useCallback'),
      BROWSER_TSX.indexOf('syncBoundsRef.current = syncBounds'),
    );
    expect(syncBounds).toContain('update_browser_bounds');
    // The value handed to update_browser_bounds is the SAME object that gets
    // published for the tooltip to read — one rect, not a reserved copy.
    expect(syncBounds).toContain('setBrowserPaneRect(finalRect)');
    expect(syncBounds).toMatch(/x:\s*finalRect\.x/);
  });

  it('no effect re-runs syncBounds because of anything tooltip-shaped', () => {
    // The chat-launcher re-sync effect used to also depend on
    // sidebarTooltipRect. If a tooltip-shaped identifier were ever added back
    // to ANY dependency array in this file, showing a tooltip would call
    // syncBounds (and therefore update_browser_bounds) again — exactly the
    // coupling this file exists to keep dead. Pin the exact array, not just
    // the absence of one identifier, so a differently-named replacement still
    // fails this test.
    const marker = "// ── Re-sync when the chat launcher appears/disappears/resizes (#553) ──";
    const effect = BROWSER_TSX.slice(
      BROWSER_TSX.indexOf(marker),
      BROWSER_TSX.indexOf(marker) + 700,
    );
    expect(effect).toContain('}, [chatLauncherSize, chatDockOpen, syncBounds]);');
  });

  it('the Browser publishes its rect instead — the inverted half of the contract', () => {
    expect(BROWSER_TSX).toContain('setBrowserPaneRect(');
    expect(TOOLTIP_TSX).toContain('s.browserPaneRect');
  });
});

describe('the tooltip places itself with the pure, tested function', () => {
  it('SidebarTooltip decides its own position with placeSidebarTooltip, not inline arithmetic', () => {
    expect(TOOLTIP_TSX).toContain('placeSidebarTooltip(');
  });

  it('a tooltip the placement function marked unsafe does not render', () => {
    expect(TOOLTIP_TSX).toMatch(/placement\.visible/);
  });
});
