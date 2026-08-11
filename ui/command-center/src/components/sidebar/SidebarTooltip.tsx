import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { font, radius, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

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
  if (!target) return null;

  const top = target.rect.top + target.rect.height / 2;
  const left = target.rect.right + 10;

  return createPortal(
    <div
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
          borderRadius: 4, padding: '1px 4px',
        }}>{target.shortcut}</span>
      )}
    </div>,
    document.body,
  );
}
