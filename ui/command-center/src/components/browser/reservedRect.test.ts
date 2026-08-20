/**
 * The sidebar hover label vs the native browser surface (reported 2026-08-19).
 *
 * "When the browser is full view and the terminal is toggled off, the tooltips
 * on the sidebar go beneath the browser window instead of over it."
 *
 * There is no CSS answer to that sentence. The tooltip is DOM inside the
 * shell's own webview; the browser is a native child WKWebView that macOS
 * composites above that webview's content. So the contract worth pinning is
 * not "the tooltip has a high z-index" — it always did — but "the browser's
 * RECT does not reach the tooltip", which is pure arithmetic and therefore
 * testable without a Tauri bridge or a live webview.
 *
 * The geometry below is the real shell's: the collapsed rail is 64px
 * (`Sidebar.tsx` `const W = open ? 208 : 64`), the tooltip is placed at
 * `rect.right + 10` (`SidebarTooltip.tsx`), and `BuildView` wraps the
 * terminal/browser split in `padding: '12px 18px'`, so a full-width browser
 * pane starts at 64 + 18 = 82.
 */
import { describe, it, expect } from 'vitest';
import { reserveFromLeft, MIN_BROWSER_WIDTH, type Rect } from './reservedRect';

const SIDEBAR_W = 64;
const PANE_PAD_X = 18;
const WINDOW_W = 1440;
const PANE_TOP = 120;
const PANE_BOTTOM = 900;

/** A sidebar tooltip for the row at `rowTop`, as SidebarTooltip places it. */
function tooltip(rowTop: number, labelWidth = 132): Rect {
  return { x: SIDEBAR_W + 10, y: rowTop, width: labelWidth, height: 26 };
}

/** The browser pane with the terminal HIDDEN: it owns the whole pane. */
const browserFullWidth: Rect = {
  x: SIDEBAR_W + PANE_PAD_X,
  y: PANE_TOP,
  width: WINDOW_W - SIDEBAR_W - 2 * PANE_PAD_X,
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

/** True when the tooltip would be swallowed by the native surface. */
function occluded(browser: Rect, tip: Rect): boolean {
  return (
    browser.x < tip.x + tip.width &&
    tip.x < browser.x + browser.width &&
    browser.y < tip.y + tip.height &&
    tip.y < browser.y + browser.height
  );
}

describe('the reported case: browser full view, terminal toggled off', () => {
  const tip = tooltip(300);

  it('is genuinely broken before the reservation is applied', () => {
    // If this ever stops being true the bug has moved and the rest of this
    // file is testing nothing.
    expect(occluded(browserFullWidth, tip)).toBe(true);
  });

  it('moves the browser off the tooltip instead of trying to out-z-index it', () => {
    const out = reserveFromLeft(browserFullWidth, tip);
    expect(occluded(out, tip)).toBe(false);
    expect(out.x).toBe(tip.x + tip.width);
    // Only the left edge moves; the pane keeps its right edge, top and bottom.
    expect(out.x + out.width).toBe(browserFullWidth.x + browserFullWidth.width);
    expect(out.y).toBe(browserFullWidth.y);
    expect(out.height).toBe(browserFullWidth.height);
  });

  it('holds for a row at any height, since the pane spans the rail', () => {
    for (const rowTop of [PANE_TOP, 250, 400, 640, PANE_BOTTOM - 40]) {
      const t = tooltip(rowTop);
      expect(occluded(reserveFromLeft(browserFullWidth, t), t)).toBe(false);
    }
  });

  it('holds for a long label, which reaches further into the pane', () => {
    const wide = tooltip(300, 260);
    expect(occluded(reserveFromLeft(browserFullWidth, wide), wide)).toBe(false);
  });
});

describe('the other layouts are left exactly as they are', () => {
  it('does not touch the browser when the terminal is showing', () => {
    const tip = tooltip(300);
    // The tooltip lands over the terminal panel, which is ordinary DOM.
    expect(occluded(browserRightHalf, tip)).toBe(false);
    expect(reserveFromLeft(browserRightHalf, tip)).toBe(browserRightHalf);
  });

  it('does not touch the browser when no tooltip is showing', () => {
    expect(reserveFromLeft(browserFullWidth, null)).toBe(browserFullWidth);
  });

  it('does not touch the browser for a tooltip above or below the pane', () => {
    const above = tooltip(PANE_TOP - 60);
    const below = tooltip(PANE_BOTTOM + 10);
    expect(reserveFromLeft(browserFullWidth, above)).toBe(browserFullWidth);
    expect(reserveFromLeft(browserFullWidth, below)).toBe(browserFullWidth);
  });

  it('ignores a degenerate rect from an unmeasured node', () => {
    const empty = { x: 74, y: 300, width: 0, height: 0 };
    expect(reserveFromLeft(browserFullWidth, empty)).toBe(browserFullWidth);
  });

  it('returns the SAME object when nothing moves, so callers can skip the native call', () => {
    expect(reserveFromLeft(browserFullWidth, null)).toBe(browserFullWidth);
    expect(reserveFromLeft(browserRightHalf, tooltip(300))).toBe(browserRightHalf);
  });
});

describe('the page wins over the decoration', () => {
  it('refuses a reservation that would squeeze the browser into a gutter', () => {
    const narrow: Rect = { x: 82, y: PANE_TOP, width: 300, height: 400 };
    const huge = tooltip(300, 200); // would leave 82 + 300 - 274 = 108px
    const out = reserveFromLeft(narrow, huge);
    expect(out).toBe(narrow);
    // …and the reason is the floor, not a missing intersection.
    expect(occluded(narrow, huge)).toBe(true);
    expect(narrow.x + narrow.width - (huge.x + huge.width)).toBeLessThan(MIN_BROWSER_WIDTH);
  });

  it('accepts one that leaves exactly the floor', () => {
    const tip = tooltip(300, 100);
    const browser: Rect = {
      x: 82,
      y: PANE_TOP,
      width: tip.x + tip.width + MIN_BROWSER_WIDTH - 82,
      height: 400,
    };
    const out = reserveFromLeft(browser, tip);
    expect(out.width).toBe(MIN_BROWSER_WIDTH);
    expect(occluded(out, tip)).toBe(false);
  });

  it('gives up rather than hide the pane when the widget covers it outright', () => {
    const covering: Rect = { x: 0, y: 0, width: WINDOW_W, height: PANE_BOTTOM + 100 };
    expect(reserveFromLeft(browserFullWidth, covering)).toBe(browserFullWidth);
  });
});
