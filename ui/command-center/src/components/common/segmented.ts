/**
 * The segmented control, app-wide.
 *
 * It started in Grow. History then grew its own copy, and that copy painted
 * the SELECTED tab with `colors.surface` — which in the dark theme is darker
 * than the strip it sits in, so the chosen tab read as recessed while a merely
 * hovered one read as raised. QA 2026-09-20 caught two tabs looking selected
 * at once. One control, one look: cyan type on a cyan wash means chosen,
 * everywhere.
 */
import type { CSSProperties } from 'react';
import { concentric, font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import type { ViewTransitionDirection } from '../../lib/viewTransition';

/** For a segmented control's tab switch (R8 item 5, coordinator ruling
 *  2026-09-25): 'forward' when the newly-selected tab sits to the right of
 *  the current one in the strip's own order, 'backward' when it sits to the
 *  left — so the view-transition slide reads as moving toward the tab you
 *  clicked, the way it would if the tabs were physical pages. */
export function segmentDirection<K>(keys: readonly K[], from: K, to: K): ViewTransitionDirection {
  const fromIdx = keys.indexOf(from);
  const toIdx = keys.indexOf(to);
  return toIdx < fromIdx ? 'backward' : 'forward';
}

/**
 * The strip a segmented tab sits in: its radius, and the padding that insets
 * the tabs. Exported because the tab's own radius is derived from both.
 */
export const SEGMENT_STRIP_RADIUS = radius.md;
export const SEGMENT_STRIP_PAD = space.xs / 2;

/** A segmented-control tab. Stays a raw `<button role="tab">` — `Button`
 *  would flatten the role — so it takes the shared rules through `.pa-btn`
 *  and its look through the same custom properties.
 *
 *  D4: the tab is a child of the strip, so `r_inner = r_outer - padding`. At an
 *  8px strip with a 2px inset that is 6 — which is `radius.sm`, the number that
 *  was here by hand. Derived now rather than coincidental, so retuning the
 *  strip carries the tab with it instead of pinching the corners apart. */
export const segmentedTab = (colors: ThemeColors, selected: boolean): CSSProperties => ({
  '--pa-btn-bg': selected ? colors.cyanSoft : 'transparent',
  '--pa-btn-fg': selected ? colors.cyan : colors.textMuted,
  '--pa-btn-border': 'transparent',
  '--pa-btn-bg-hover': selected ? colors.cyanSoft : colors.surfaceHi,
  '--pa-btn-fg-hover': selected ? colors.cyan : colors.text,
  '--pa-btn-border-hover': 'transparent',
  '--pa-btn-bg-active': selected ? colors.cyanGlow : colors.surface,
  '--pa-btn-pad': `${space.sm}px ${space.xl}px`,
  '--pa-btn-radius': `${concentric(SEGMENT_STRIP_RADIUS, SEGMENT_STRIP_PAD)}px`,
  '--pa-btn-weight': selected ? 600 : 500,
  fontSize: textSize.caption,
  fontFamily: font.body,
  outline: 'none',
} as CSSProperties);
