/**
 * Browser chrome geometry + interaction fills.
 *
 * The native webview fills `containerRef` (flex-1 between the top chrome stack
 * and the status bar). Any change to these paddings/gaps changes that rect.
 * Tests freeze the numbers; do not retune them for aesthetics.
 */
import type { CSSProperties } from 'react';
import { concentric, radius, space } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/** Pixel paddings that set the webview's vertical leftover. Was Tailwind
 *  `px-3 py-2` / `px-3 py-1` / `gap-2` / `gap-1` / `gap-3` — same values,
 *  named so a retune is a deliberate, tested change. */
export const CHROME_GEOM = {
  toolbarPadY: space.md,       // 8  — URL row `py-2`
  toolbarPadX: space.xl,       // 12 — URL row `px-3`
  toolbarGap: space.md,        // 8  — URL row `gap-2`
  bookmarksPadY: space.xs,     // 4  — bookmarks `py-1`
  bookmarksPadX: space.xl,     // 12 — bookmarks `px-3`
  bookmarksGap: space.xs,      // 4  — bookmarks `gap-1`
  statusPadY: space.xs,        // 4  — status `py-1`
  statusPadX: space.xl,        // 12 — status `px-3`
  statusGap: space.xl,         // 12 — status `gap-3`
  /** Tab / nav icon button pad — was `6px` / `6px 12px`. */
  tabPadY: space.sm,           // 6
  tabPadX: space.xl,           // 12
  navIconPad: space.sm,        // 6
  /** Bookmark chip vertical pad. Half-step below `space.xs`; growing it
   *  would thicken the bookmarks row and shrink the webview. */
  chipPadY: 2,
  chipPadX: space.md,          // 8
} as const;

/**
 * Address-field radius. The field is a medium dense control (D5 — not a
 * capsule). Previous look was Tailwind `rounded-md` ≡ `radius.sm` (6). Nested
 * through the chrome stack: `concentric(radius.glass, toolbarPadX)` = 0
 * (square — correct when pad ≥ outer), so the field keeps its own scale step
 * rather than pinching to 0/1. Chips nest under THIS radius via CHIP_RADIUS.
 */
export const ADDRESS_RADIUS = radius.sm;

/** Chip radius nested inside the address-field radius (D4). */
export const CHIP_RADIUS = concentric(ADDRESS_RADIUS, CHROME_GEOM.chipPadY);

/** Soft danger wash for arm-then-confirm close/delete (was Tailwind red/20). */
export function dangerWash(colors: ThemeColors): string {
  return `${colors.danger}33`;
}

/** Bare icon / tab button vars on glass: hover/press via fill tokens (D2/D10),
 *  never a second backdrop-filter. */
export function chromeBareVars(
  colors: ThemeColors,
  opts: {
    fg?: string;
    fgHover?: string;
    bg?: string;
    pad?: string;
    radiusPx?: number;
  } = {},
): CSSProperties {
  return {
    '--pa-btn-bg': opts.bg ?? 'transparent',
    '--pa-btn-fg': opts.fg ?? colors.textMuted,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': colors.fillHover,
    '--pa-btn-fg-hover': opts.fgHover ?? colors.text,
    '--pa-btn-bg-active': colors.fillActive,
    '--pa-btn-pad': opts.pad ?? `${CHROME_GEOM.navIconPad}px`,
    '--pa-btn-radius': `${opts.radiusPx ?? radius.xs}px`,
  } as CSSProperties;
}
