/**
 * Terminal chrome geometry + interaction fills.
 *
 * The PTY/xterm canvas fills the flex-1 leftover under the tab strip. Any
 * change to these paddings/gaps changes that rect. Tests freeze the numbers;
 * do not retune them for aesthetics. Attach/detach / reattach / fit logic
 * lives elsewhere and must stay byte-identical.
 */
import type { CSSProperties } from 'react';
import { concentric, radius, space } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/** Pixel paddings that set the PTY leftover under the tab strip. Was Tailwind
 *  `py`-via-`6px 12px` / `gap: 6` / `6px 8px` / `6px 10px` / `m-2` — same
 *  values, named so a retune is a deliberate, tested change. */
export const CHROME_GEOM = {
  tabPadY: space.sm,           // 6
  tabPadX: space.xl,           // 12
  tabGap: space.sm,            // 6 — icon/label gap inside a tab
  railPadY: space.sm,          // 6 — pop-out / new-tab vertical
  popOutPadX: space.md,        // 8
  newTabPadX: space.lg,        // 10
  /** Drop overlay inset from pane edge — was Tailwind `m-2`. */
  dropMargin: space.md,        // 8
  /** Pending-prompt chip inset — was `bottom-2 right-2`. */
  chipInset: space.md,         // 8
  chipPadY: space.md,          // 8 — was `py-2`
  chipPadX: space.xl,          // 12 — was `px-3`
  chipBtnPadY: space.xs,       // 4
  chipBtnPadX: space.md,       // 8
  chipBtnGap: space.xs,        // 4
} as const;

/** Outer chrome radius nested under the window glass step (D4). Flush in the
 *  pane when pad ≥ outer — same arithmetic browser chrome uses. */
export const CHROME_RADIUS = concentric(radius.glass, radius.glass);

/** Drop overlay corner — was Tailwind `rounded-xl` ≡ `radius.lg`. */
export const DROP_RADIUS = radius.lg;

/** Pending-prompt chip — was Tailwind `rounded-lg` ≡ `radius.md`. Nested
 *  under its own pad so the inner buttons can sit concentrically. */
export const CHIP_RADIUS = radius.md;
export const CHIP_BTN_RADIUS = concentric(CHIP_RADIUS, CHROME_GEOM.chipBtnPadY);

/** Soft danger wash for arm-then-confirm close (was Tailwind red/20). */
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
    '--pa-btn-pad': opts.pad ?? `${CHROME_GEOM.railPadY}px ${CHROME_GEOM.popOutPadX}px`,
    '--pa-btn-radius': `${opts.radiusPx ?? radius.xs}px`,
  } as CSSProperties;
}
