/**
 * World HUD chrome geometry + interaction fills.
 *
 * HUD panels are the one place glass-over-content is correct: they float over
 * the 3D canvas (D1). One glass plane per panel (D2); interactive children use
 * fillHover/fillActive, never a second backdrop-filter. Geometry is named so
 * a retune is deliberate; nested radii come from `concentric`.
 */
import type { CSSProperties } from 'react';
import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/** Pixel paddings/gaps for the floating agent panels. Prior literals, named. */
export const HUD_GEOM = {
  panelPadX: 14,
  headerPadTop: space.lg,      // 10
  headerPadBottom: space.sm,   // 6
  bodyPadY: space.md,          // 8
  sectionGap: space.sm,        // 6
  tabPadY: space.sm,           // 6
  tabPadX: space.lg,           // 10
  closePad: 2,
  panelWidth: 300,
  panelInset: space.xxl,       // 16 — distance from canvas edge
  badgePadY: space.xs,         // 4
  badgePadX: space.lg,         // 10
  pillPadY: 2,
  pillPadX: space.md,          // 8
} as const;

/** Outermost floating panel radius (D4 — concentric with the window). */
export const HUD_PANEL_RADIUS = radius.glass;

/** Nested control radius inside the panel padding. */
export const HUD_INNER_RADIUS = concentric(HUD_PANEL_RADIUS, HUD_GEOM.panelPadX);

/** Status-pill radius nested under the header pad. */
export const HUD_PILL_RADIUS = concentric(HUD_PANEL_RADIUS, HUD_GEOM.headerPadTop);

/** Spring transition for hover/press on HUD controls (<500ms, D9). */
export function hudTransition(reduceMotion: boolean): string {
  return reduceMotion ? 'none' : `background ${duration.snappy}ms ${ease.snappy}, color ${duration.snappy}ms ${ease.snappy}, border-color ${duration.snappy}ms ${ease.snappy}, transform ${duration.snappy}ms ${ease.snappy}`;
}

/** Bare icon / tab button vars on glass: hover/press via fill tokens (D2/D10). */
export function hudBareVars(
  colors: ThemeColors,
  opts: {
    fg?: string;
    fgHover?: string;
    bg?: string;
    pad?: string;
    radiusPx?: number;
    weight?: number;
  } = {},
): CSSProperties {
  return {
    '--pa-btn-bg': opts.bg ?? 'transparent',
    '--pa-btn-fg': opts.fg ?? colors.textMuted,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': colors.fillHover,
    '--pa-btn-fg-hover': opts.fgHover ?? colors.text,
    '--pa-btn-bg-active': colors.fillActive,
    '--pa-btn-pad': opts.pad ?? `${HUD_GEOM.closePad}px ${space.xs}px`,
    '--pa-btn-radius': `${opts.radiusPx ?? radius.xs}px`,
    ...(opts.weight != null ? { '--pa-btn-weight': opts.weight } : {}),
    fontFamily: font.mono,
  } as CSSProperties;
}

/** Mono caption used across HUD bodies (was scattered `fontSize: 10` / micro). */
export const hudCaption = {
  fontFamily: font.mono,
  fontSize: textSize.micro,
  lineHeight: 1.5,
} as const;
