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
import { font, radius, space } from '../../styles/tokens';
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

/* The segmented control moved to `components/common/segmented.ts` when History
   needed the same look; re-exported so Grow's own imports keep working. */
export { SEGMENT_STRIP_RADIUS, SEGMENT_STRIP_PAD, segmentedTab } from '../common/segmented';

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
