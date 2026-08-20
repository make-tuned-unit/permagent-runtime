/**
 * Keeping a piece of DOM chrome visible when the native browser surface would
 * cover it.
 *
 * THE MECHANISM, because no amount of CSS can work around it. The in-app
 * browser is a native child WKWebView added to the shell window
 * (`browser.rs` `create_browser_webview` -> `window.add_child`). macOS
 * composites a native subview ABOVE the web content of the window it lives in,
 * so anything drawn by the React shell — which IS that web content — is behind
 * it no matter what `z-index` it carries. `WEBVIEW_LIFECYCLE.md` records this
 * as the root of #553 and rules the remedy (D2): **bounds-subtract**. The
 * browser's rectangle is ours to choose (`update_browser_bounds`), so the way
 * to keep a widget visible is to stop the rectangle from reaching it, not to
 * try to lift the widget.
 *
 * `Browser.tsx` already does exactly this for the collapsed chat launcher's
 * corner. This module generalises the same idea to a rectangle any part of the
 * shell can reserve, and — more to the point — makes the geometry a pure
 * function that a test can hold to account. The launcher's version lives
 * inline in a callback that needs a Tauri bridge, a ResizeObserver and a live
 * webview to reach, which is why it has never had one.
 *
 * Reported 2026-08-19: "when the browser is full view and the terminal is
 * toggled off, the tooltips on the sidebar go beneath the browser window
 * instead of over it." That combination is not incidental — see
 * `sidebarTooltipCase` in the tests. `BuildView` lays the terminal and the
 * browser out as a HORIZONTAL pair, so with the terminal showing, the browser
 * occupies only the right half of the pane and a sidebar tooltip lands over
 * the terminal, which is ordinary DOM and layers normally. Hide the terminal
 * and the browser panel takes the whole pane: its rectangle now starts a few
 * pixels right of the sidebar rail and the tooltip is swallowed.
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Never shrink the browser below this. A reservation is decoration; the page
 * is the point. If honouring one would leave a strip too narrow to browse in,
 * the reservation is refused and the widget stays covered — a visibly wrong
 * tooltip is a smaller harm than a page squeezed into a gutter.
 */
export const MIN_BROWSER_WIDTH = 240;

function intersects(a: Rect, b: Rect): boolean {
  return (
    a.x < b.x + b.width &&
    b.x < a.x + a.width &&
    a.y < b.y + b.height &&
    b.y < a.y + a.height
  );
}

/**
 * Move the browser rectangle's LEFT edge right, far enough that it no longer
 * intersects `reserved`.
 *
 * Left, specifically, because every widget this exists for lives against the
 * left of the shell (the sidebar rail and its hover labels) while the browser
 * pane always sits to its right. Returns `browser` unchanged — the SAME object
 * — whenever nothing needs to move, so a caller can use identity to skip a
 * native bounds call.
 */
export function reserveFromLeft(browser: Rect, reserved: Rect | null): Rect {
  if (!reserved) return browser;
  if (reserved.width <= 0 || reserved.height <= 0) return browser;
  if (!intersects(browser, reserved)) return browser;

  const reservedRight = reserved.x + reserved.width;
  const browserRight = browser.x + browser.width;

  // The widget already clears the browser's left edge, or it covers the pane
  // outright. Neither is fixable by moving one edge.
  if (reservedRight <= browser.x) return browser;
  if (reservedRight >= browserRight) return browser;

  const width = browserRight - reservedRight;
  if (width < MIN_BROWSER_WIDTH) return browser;

  return { x: reservedRight, y: browser.y, width, height: browser.height };
}
