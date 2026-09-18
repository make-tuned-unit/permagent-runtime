/**
 * Grow's shared control looks, as `--pa-btn-*` custom properties.
 *
 * Split out of GrowView.tsx (R9) so every module in this directory reaches for
 * the same six, rather than each growing its own copy of the chip.
 *
 * Every padding now names a step on the spacing scale rather than repeating the
 * numbers the app converged on by hand, and the tab's radius is DERIVED from
 * the strip that holds it instead of chosen — see `segmentedTab`.
 */

import type { CSSProperties } from 'react';
import { concentric, font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/* Grow's two recurring button looks, as `--pa-btn-*` custom properties rather
   than inline `background`/`color`/`border`: an inline declaration outranks
   `.pa-btn:hover`, which is exactly the state this view was missing. `ghost`
   already carries the hairline's resting colors, so the plain chip only has to
   name its size. */
export const growChip = (pad = `${space.xs}px ${space.lg}px`): CSSProperties => ({
  '--pa-btn-pad': pad,
  '--pa-btn-radius': `${radius.md}px`,
  fontFamily: font.body,
} as CSSProperties);

/** The accent action — cyan type on a cyan wash, the "do the thing" control. */
export const growAccent = (colors: ThemeColors, pad = `${space.sm}px ${space.xl}px`): CSSProperties => ({
  '--pa-btn-fg': colors.cyan,
  '--pa-btn-bg': colors.cyanSoft,
  '--pa-btn-border': colors.borderHi,
  '--pa-btn-bg-hover': colors.cyanGlow,
  '--pa-btn-border-hover': colors.cyan,
  '--pa-btn-pad': pad,
  '--pa-btn-radius': `${radius.md}px`,
  fontFamily: font.body,
} as CSSProperties);

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

/** The small filled control the action cards and the verify rail both use. */
export const growSmall = (colors: ThemeColors): CSSProperties => ({
  '--pa-btn-bg': colors.surface,
  '--pa-btn-fg': colors.text,
  '--pa-btn-border': colors.border,
  '--pa-btn-bg-hover': colors.surfaceHi,
  '--pa-btn-border-hover': colors.borderHi,
  '--pa-btn-pad': `${space.xs}px ${space.lg}px`,
  '--pa-btn-radius': `${radius.sm}px`,
  fontFamily: font.body,
} as CSSProperties);

/** An underlined text link. Cyan on hover: this view already spells "a link"
 *  in cyan (site ↗ / repo ↗ / open project ↗), and these had no hover at all. */
export const growLink = (colors: ThemeColors): CSSProperties => ({
  '--pa-btn-fg': colors.text,
  '--pa-btn-fg-hover': colors.cyan,
  '--pa-btn-bg-hover': 'transparent',
  '--pa-btn-pad': '0',
  fontFamily: font.body,
  textDecoration: 'underline',
} as CSSProperties);

/** A text affordance with no chrome — `bare`, muted, coming up to full on
 *  hover. Padding stays at zero because preflight gave these none. */
export const growBare = (colors: ThemeColors): CSSProperties => ({
  '--pa-btn-fg': colors.textMuted,
  '--pa-btn-fg-hover': colors.text,
  '--pa-btn-bg-hover': 'transparent',
  '--pa-btn-pad': '0',
  fontFamily: font.body,
} as CSSProperties);
