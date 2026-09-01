/**
 * Where a sidebar hover label may render without ever touching the native
 * browser surface — the replacement for the bounds-subtraction that #1068
 * shipped (`browser/reservedRect.ts`, deleted 2026-09-01).
 *
 * THE CONSTRAINT DOES NOT MOVE. The in-app browser is a native child
 * WKWebView (`browser.rs` `create_browser_webview` -> `window.add_child`);
 * macOS composites a native subview ABOVE the web content of the window it
 * lives in, so nothing the React shell draws — DOM, any `z-index` — can ever
 * appear over it. `WEBVIEW_LIFECYCLE.md` ruling D2 still holds.
 *
 * WHAT MOVES IS WHICH SIDE ADAPTS. The 2026-08-19 fix had the tooltip publish
 * its rect and had the Browser subtract it from the native bounds on every
 * hover — correct, but it meant showing a tooltip could push a real
 * `update_browser_bounds` call, and #1068's own file carried a warning that
 * dormant machinery of exactly this shape invites rewiring the bug back in.
 * This module inverts it: the Browser publishes ITS OWN rect (read-only,
 * `browserPaneRect` in the store) and reacts to nothing about the sidebar;
 * the tooltip reads that rect and places ITSELF to guarantee no overlap.
 *
 * THE GEOMETRY IS TIGHTER THAN IT LOOKS. `BuildView` wraps the terminal /
 * browser split in `padding: '12px 18px'`, so with the terminal hidden and
 * the rail collapsed (`Sidebar.tsx` `W = open ? 208 : 64`) the browser's
 * native rect starts at only 64 + 18 = 82px — a handful of pixels past a
 * hover label's natural position at `anchor.right + 10`. There is not enough
 * ROOM to the label's right for a full label there in that layout, so rather
 * than truncate (illegible) or overlap (the whole bug), `placeSidebarTooltip`
 * makes a different trade: pull the label's LEFT edge back — even as far as
 * the rail's own icon column, which is ordinary DOM the tooltip's portal can
 * paint over without any native-surface conflict — and let it wrap onto a
 * second line rather than spill past the boundary. The guarantee is proven
 * by construction: the returned box's right edge (`left + maxWidth`) is
 * always `<= browserRect.x`, so its X-range and the browser's X-range are
 * disjoint. Two rects with disjoint X-ranges cannot intersect regardless of
 * Y or height, which is what makes this safe without ever measuring the
 * label's rendered height (unknowable before the wrap happens).
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TooltipPlacement {
  left: number;
  top: number;
  /** `undefined` when nothing needs to be avoided — render at natural width. */
  maxWidth: number | undefined;
  /** False only in a layout so degenerate (browser rect flush against the
   *  rail with no gap at all) that no legible box would fit anywhere safe.
   *  Never observed with the shell's real geometry; a defensive floor. */
  visible: boolean;
}

/** Generous cap on a natural, unconstrained tooltip — long enough for any
 *  real label + shortcut chip on one line, short enough not to sprawl. */
export const NATURAL_MAX_WIDTH = 260;

/** Below this, wrapping stops helping — a box this narrow cannot hold even
 *  one word legibly, so `placeSidebarTooltip` gives up and hides it instead
 *  of rendering an unreadable sliver. */
export const MIN_RENDERABLE_WIDTH = 40;

/** The width `placeSidebarTooltip` tries to guarantee before it resorts to
 *  pulling the label's left edge back over the rail's own icon column. */
export const PREFERRED_MIN_WIDTH = 120;

/** Breathing room between the label's right edge and the native surface's
 *  left edge. Matches the old `TOOLTIP_CLEARANCE_PX`: a boundary landing
 *  exactly on the label's edge can round the wrong way under device-pixel
 *  scaling and clip it by a hair. */
export const CLEARANCE_PX = 6;

/** Gap between the hovered row and the label, at its natural (unconstrained)
 *  position — matches `SidebarTooltip.tsx`'s existing `left = rect.right + 10`. */
export const NATURAL_OFFSET_PX = 10;

/**
 * Compute where a sidebar hover label may render.
 *
 * `anchorRect` is the hovered row's own bounding rect (`el.getBoundingClientRect()`
 * in `SidebarTooltip.tsx`). `browserRect` is the browser pane's CURRENT native
 * rect from the store, or `null` when no browser webview is being positioned
 * at all (most workspaces, or the browser not yet loaded) — in which case
 * there is nothing to avoid and the label gets its natural size.
 */
export function placeSidebarTooltip(anchorRect: Rect, browserRect: Rect | null): TooltipPlacement {
  const top = anchorRect.y + anchorRect.height / 2;
  const desiredLeft = anchorRect.x + anchorRect.width + NATURAL_OFFSET_PX;

  if (!browserRect) {
    return { left: desiredLeft, top, maxWidth: NATURAL_MAX_WIDTH, visible: true };
  }

  const rightBound = browserRect.x - CLEARANCE_PX;
  const naturalAvailable = rightBound - desiredLeft;

  // Plenty of room at the natural position (the overwhelmingly common case:
  // rail open, terminal showing, or no browser filling the pane) — cap width
  // only as much as the gap actually requires, which for a generous gap is
  // not at all.
  if (naturalAvailable >= PREFERRED_MIN_WIDTH) {
    return { left: desiredLeft, top, maxWidth: Math.min(NATURAL_MAX_WIDTH, naturalAvailable), visible: true };
  }

  // Not enough room to the label's right. Pull its LEFT edge back — over the
  // rail's own icon column if need be, which is DOM the portal can paint
  // over freely — so it has PREFERRED_MIN_WIDTH to wrap into instead of
  // spilling past `rightBound`.
  const left = Math.max(0, rightBound - PREFERRED_MIN_WIDTH);
  const maxWidth = Math.max(0, Math.min(NATURAL_MAX_WIDTH, rightBound - left));

  if (maxWidth < MIN_RENDERABLE_WIDTH) {
    return { left, top, maxWidth: 0, visible: false };
  }
  return { left, top, maxWidth, visible: true };
}

/** True if a placed box (as `placeSidebarTooltip` would render it, at ANY
 *  height) could ever overlap `browserRect`. Used by the placement's own
 *  tests to hold the "disjoint X-ranges" proof to account without having to
 *  measure a real rendered node. */
export function boxIntersectsBrowser(
  placement: Pick<TooltipPlacement, 'left' | 'maxWidth'>,
  browserRect: Rect | null,
): boolean {
  if (!browserRect || placement.maxWidth === undefined) return false;
  return placement.left + placement.maxWidth > browserRect.x;
}
