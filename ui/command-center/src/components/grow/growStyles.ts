/**
 * Grow's shared control looks, as `--pa-btn-*` custom properties.
 *
 * Split out of GrowView.tsx (R9) so every module in this directory reaches for
 * the same six, rather than each growing its own copy of the chip. Nothing here
 * changed in the split — these are the same objects the view has always spread.
 */

import type { CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/* Grow's two recurring button looks, as `--pa-btn-*` custom properties rather
   than inline `background`/`color`/`border`: an inline declaration outranks
   `.pa-btn:hover`, which is exactly the state this view was missing. `ghost`
   already carries the hairline's resting colors, so the plain chip only has to
   name its size. */
export const growChip = (pad = '4px 10px'): CSSProperties => ({
  '--pa-btn-pad': pad,
  '--pa-btn-radius': `${radius.md}px`,
  fontFamily: font.body,
} as CSSProperties);

/** The accent action — cyan type on a cyan wash, the "do the thing" control. */
export const growAccent = (colors: ThemeColors, pad = '5px 12px'): CSSProperties => ({
  '--pa-btn-fg': colors.cyan,
  '--pa-btn-bg': colors.cyanSoft,
  '--pa-btn-border': colors.borderHi,
  '--pa-btn-bg-hover': colors.cyanGlow,
  '--pa-btn-border-hover': colors.cyan,
  '--pa-btn-pad': pad,
  '--pa-btn-radius': `${radius.md}px`,
  fontFamily: font.body,
} as CSSProperties);

/** A segmented-control tab. Stays a raw `<button role="tab">` — `Button`
 *  would flatten the role — so it takes the shared rules through `.pa-btn`
 *  and its look through the same custom properties. */
export const segmentedTab = (colors: ThemeColors, selected: boolean): CSSProperties => ({
  '--pa-btn-bg': selected ? colors.cyanSoft : 'transparent',
  '--pa-btn-fg': selected ? colors.cyan : colors.textMuted,
  '--pa-btn-border': 'transparent',
  '--pa-btn-bg-hover': selected ? colors.cyanSoft : colors.surfaceHi,
  '--pa-btn-fg-hover': selected ? colors.cyan : colors.text,
  '--pa-btn-border-hover': 'transparent',
  '--pa-btn-bg-active': selected ? colors.cyanGlow : colors.surface,
  '--pa-btn-pad': '5px 12px',
  '--pa-btn-radius': `${radius.sm}px`,
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
  '--pa-btn-pad': '3px 10px',
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
