import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { font, radius, ease, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { placeSidebarTooltip } from './tooltipPlacement';

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
 *     it, `zIndex: 9999` and all (reported 2026-08-19).
 *
 *     FIXED 2026-08-19 by having the tooltip publish its rect and having
 *     `Browser.tsx` subtract it from the native bounds (WEBVIEW_LIFECYCLE.md
 *     ruling D2). CHANGED 2026-09-01: that made showing a tooltip able to
 *     move the page, and #1068's own file warned that dormant machinery of
 *     exactly this shape invites rewiring the bug back in. `placeSidebarTooltip`
 *     (./tooltipPlacement.ts) inverts it — the Browser publishes its OWN rect
 *     (`browserPaneRect` in the store, read-only here) and this component
 *     places ITSELF to guarantee no overlap, including pulling its own left
 *     edge back over the rail's icon column (ordinary DOM; a portal can paint
 *     over it with no native-surface conflict) rather than ever reaching into
 *     the browser's rect. The page never moves for this again.
 *
 * The delay is short on first hover and drops to zero while the pointer is
 * moving between rows (the "warm" window), matching how OS menu bars behave:
 * deliberate on entry, instant once you are clearly browsing.
 */

const COLD_DELAY_MS = 260;
const WARM_WINDOW_MS = 700;

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
  // Read-only: the Browser publishes this from its own bounds sync and never
  // hears about the sidebar at all. See tooltipPlacement.ts for the geometry
  // proof that placing the label around this rect can never overlap it.
  const browserPaneRect = useCommandCenter(s => s.browserPaneRect);

  if (!target) return null;

  const placement = placeSidebarTooltip(target.rect, browserPaneRect);
  if (!placement.visible) return null;

  return createPortal(
    <div
      role="tooltip"
      style={{
        position: 'fixed', top: placement.top, left: placement.left,
        transform: 'translateY(-50%)',
        zIndex: 9999, pointerEvents: 'none',
        display: 'flex', alignItems: 'center', gap: 8,
        flexWrap: 'wrap',
        maxWidth: placement.maxWidth,
        padding: '5px 9px',
        borderRadius: radius.sm,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: colors.elevationRaised ?? colors.cardShadow,
        fontFamily: font.body, fontSize: textSize.caption, fontWeight: 500,
        color: colors.text,
        // Only the tight collapsed-rail-vs-full-browser layout ever sets a
        // maxWidth narrow enough to matter; everywhere else the box is wide
        // enough that this never wraps. Wrapping (not truncating) is the
        // point — a label that cannot fit its natural width still shows every
        // character, just on more than one line.
        whiteSpace: placement.maxWidth !== undefined ? 'normal' : 'nowrap',
        wordBreak: 'break-word',
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
          whiteSpace: 'nowrap',
        }}>{target.shortcut}</span>
      )}
    </div>,
    document.body,
  );
}
