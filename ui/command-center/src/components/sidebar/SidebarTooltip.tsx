import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { font, radius, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';

/**
 * Hover label for a sidebar row.
 *
 * Two constraints drive the implementation:
 *
 *  1. **The sidebar clips.** Its container sets `overflow: hidden` to animate
 *     width, so a tooltip rendered inside it is cut off at the rail edge. This
 *     portals to `document.body` and positions from a measured rect instead.
 *
 *  2. **The native `title` attribute is unusable for wayfinding.** It waits
 *     roughly a second before appearing, can't be styled, and renders in the
 *     OS chrome. When the rail is collapsed the label is the ONLY way to tell
 *     tabs apart, so a one-second delay means expanding the sidebar is faster
 *     than hovering — which is exactly the problem this is meant to remove.
 *
 *  3. **The in-app browser is not DOM, so it cannot be layered under.** The
 *     browser pane is a native child WKWebView, and macOS composites a native
 *     subview above the web content of the window that hosts it. The tooltip
 *     is drawn to the RIGHT of the rail, i.e. into the pane the browser
 *     occupies — so with the browser filling that pane it disappeared behind
 *     it, `zIndex: 9999` and all (reported 2026-08-19). The fix cannot live in
 *     CSS: the tooltip publishes its measured rect and `Browser.tsx` keeps the
 *     native bounds clear of it (WEBVIEW_LIFECYCLE.md ruling D2).
 *
 * The delay is short on first hover and drops to zero while the pointer is
 * moving between rows (the "warm" window), matching how OS menu bars behave:
 * deliberate on entry, instant once you are clearly browsing.
 */

const COLD_DELAY_MS = 260;
const WARM_WINDOW_MS = 700;

/**
 * Breathing room between the label and the native browser surface. The rect
 * the shell measures in CSS pixels becomes a native frame in device pixels, and
 * a boundary that lands exactly on the label's border can round the wrong way
 * and clip it. Cheap insurance; it costs the page nothing it would miss.
 */
const TOOLTIP_CLEARANCE_PX = 6;

/** Shared across rows: when a tooltip last closed, for the warm-hover window. */
let lastHiddenAt = 0;

export interface TooltipTarget {
  rect: DOMRect;
  label: string;
  shortcut?: string;
}

export function useSidebarTooltip() {
  const [target, setTarget] = useState<TooltipTarget | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>();

  const show = (el: HTMLElement | null, label: string, shortcut?: string) => {
    if (!el) return;
    clearTimeout(timer.current);
    const warm = Date.now() - lastHiddenAt < WARM_WINDOW_MS;
    const commit = () => setTarget({ rect: el.getBoundingClientRect(), label, shortcut });
    if (warm) commit();
    else timer.current = setTimeout(commit, COLD_DELAY_MS);
  };

  const hide = () => {
    clearTimeout(timer.current);
    setTarget(prev => {
      if (prev) lastHiddenAt = Date.now();
      return null;
    });
  };

  useEffect(() => () => clearTimeout(timer.current), []);

  return { target, show, hide };
}

export function SidebarTooltip({ target }: { target: TooltipTarget | null }) {
  const { colors, reduceMotion } = useTheme();
  const setReservedRect = useCommandCenter(s => s.setSidebarTooltipRect);
  const boxRef = useRef<HTMLDivElement | null>(null);

  const top = target ? target.rect.top + target.rect.height / 2 : 0;
  const left = target ? target.rect.right + 10 : 0;

  // Measure AFTER layout, before paint: the label's width depends on the text,
  // so the rect the browser has to keep clear is not knowable until the node
  // exists. Nothing is reserved while no tooltip is up, which is why the
  // browser is untouched in the overwhelmingly common case.
  //
  // Width and height come from the node; the LEFT edge does not. `sidebarTooltipIn`
  // starts the label 4px to the left and slides it into place, so a rect
  // measured now would be that 4px short and leave a sliver of the finished
  // label under the webview. `left` is where it is going, which is the edge
  // the browser actually has to clear.
  useLayoutEffect(() => {
    if (!target) {
      setReservedRect(null);
      return;
    }
    const el = boxRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setReservedRect({
      x: left,
      y: r.y,
      width: r.width + TOOLTIP_CLEARANCE_PX,
      height: r.height,
    });
  }, [target, left, setReservedRect]);

  // A tooltip that is unmounted mid-hover (view switch, sidebar collapse) must
  // not leave the browser permanently narrowed.
  useEffect(() => () => setReservedRect(null), [setReservedRect]);

  if (!target) return null;

  return createPortal(
    <div
      ref={boxRef}
      role="tooltip"
      style={{
        position: 'fixed', top, left, transform: 'translateY(-50%)',
        zIndex: 9999, pointerEvents: 'none',
        display: 'flex', alignItems: 'center', gap: 8,
        padding: '5px 9px',
        borderRadius: radius.sm,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: colors.elevationRaised ?? colors.cardShadow,
        fontFamily: font.body, fontSize: 12, fontWeight: 500,
        color: colors.text, whiteSpace: 'nowrap',
        animation: reduceMotion ? undefined : `sidebarTooltipIn 120ms ${ease.out}`,
      }}
    >
      {target.label}
      {target.shortcut && (
        <span style={{
          fontSize: 10, fontWeight: 600, letterSpacing: '0.04em',
          color: colors.textDim,
          border: `1px solid ${colors.border}`,
          borderRadius: radius.xs, padding: '1px 4px',
        }}>{target.shortcut}</span>
      )}
    </div>,
    document.body,
  );
}
