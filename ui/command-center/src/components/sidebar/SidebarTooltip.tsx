import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useCommandCenter } from '../../lib/store';
import {
  TooltipBubble,
  TOOLTIP_COLD_DELAY_MS,
  TOOLTIP_WARM_WINDOW_MS,
  isTooltipWarm,
  noteTooltipHidden,
  useTooltipDismiss,
} from '../common/Tooltip';
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
 * The chrome (glass, concentric radius, spring-in, reduce-motion /
 * reduce-transparency) lives in `common/Tooltip` (`TooltipBubble`). This file
 * keeps the multi-row warm-hover controller and the browser-pane inversion.
 *
 * The delay is short on first hover and drops to zero while the pointer is
 * moving between rows (the "warm" window), matching how OS menu bars behave:
 * deliberate on entry, instant once you are clearly browsing.
 */

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
    const commit = () => setTarget({ rect: el.getBoundingClientRect(), label, shortcut });
    if (isTooltipWarm()) commit();
    else timer.current = setTimeout(commit, TOOLTIP_COLD_DELAY_MS);
  };

  const hide = () => {
    clearTimeout(timer.current);
    setTarget(prev => {
      if (prev) noteTooltipHidden();
      return null;
    });
  };

  useEffect(() => () => clearTimeout(timer.current), []);

  return { target, show, hide };
}

// Re-export so existing warm-window timing references stay discoverable.
export { TOOLTIP_COLD_DELAY_MS as COLD_DELAY_MS, TOOLTIP_WARM_WINDOW_MS as WARM_WINDOW_MS };

export function SidebarTooltip({
  target,
  onDismiss,
}: {
  target: TooltipTarget | null;
  /** Escape / scroll — parent clears its `useSidebarTooltip` target. */
  onDismiss?: () => void;
}) {
  // Read-only: the Browser publishes this from its own bounds sync and never
  // hears about the sidebar at all. See tooltipPlacement.ts for the geometry
  // proof that placing the label around this rect can never overlap it.
  const browserPaneRect = useCommandCenter(s => s.browserPaneRect);

  useTooltipDismiss(Boolean(target), () => {
    onDismiss?.();
  });

  if (!target) return null;

  const placement = placeSidebarTooltip(target.rect, browserPaneRect);
  if (!placement.visible) return null;

  // Right-of-rail resting pose (translateY -50%); enter with a short nudge.
  const transform = 'translateY(-50%)';
  const fromTransform = 'translateY(-50%) translateX(-4px)';

  return createPortal(
    <TooltipBubble
      id="pa-sidebar-tooltip"
      left={placement.left}
      top={placement.top}
      transform={transform}
      fromTransform={fromTransform}
      maxWidth={placement.maxWidth}
      wrap={placement.maxWidth !== undefined}
      shortcut={target.shortcut}
    >
      {target.label}
    </TooltipBubble>,
    document.body,
  );
}
