/**
 * Glass — the one surface in the app allowed to be translucent.
 *
 * This module is the sole owner of `backdropFilter` in `src/` (enforced by
 * `styles/backdropFilter.test.ts`). Everything else either uses `<Glass>`,
 * spreads `glassSurface()`, or is opaque. That is not tidiness for its own
 * sake — it is the fix for the bug this primitive shipped with.
 *
 * WHAT WAS WRONG. The old `Glass` (wizard/atoms.tsx) paired
 * `backdropFilter: blur(24px) saturate(140%)` with `background: colors.surface`
 * — an OPAQUE hex. A blur filter samples what is behind the element and then
 * an opaque fill paints straight over the result, so the blur was inert: no
 * pixel of it ever reached the screen, and it still cost a full compositing
 * pass every frame, on every one of the ten surfaces that had copied the
 * pairing. Real glass needs a translucent fill; the tokens supply both halves
 * together (`GlassSurface`) so they cannot be separated again.
 *
 * WHERE GLASS IS ALLOWED. Apple's rule, verbatim: "Don't use Liquid Glass in
 * the content layer." Glass belongs to the floating control layer — toolbars,
 * sidebars, floating command bars, popover and menu chrome, toasts, HUD
 * overlays — and stops there. Cards are content. List rows are content. Chat
 * bubbles are content. Modal bodies are content. If a content surface needs to
 * feel elevated, change its fill or its spacing, not its transparency.
 *
 * And never glass on glass: "Stacking Liquid Glass elements on top of each
 * other can quickly make the interface feel cluttered and confusing." A
 * control sitting on a glass toolbar gets `colors.fillHover`, not a second
 * `backdrop-filter`. This is simultaneously the aesthetic rule and the
 * performance one — each nested filter forces its own render-to-texture pass,
 * so two or three animated glass surfaces are enough to cost frames.
 */

import { useEffect } from 'react';
import type { CSSProperties, ReactNode } from 'react';

import { useTheme } from '../../styles/useTheme';
import type { GlassSurface } from '../../styles/useTheme';
import { initReduceTransparencyBridge } from '../../styles/reduceTransparency';

export type GlassVariant = 'glass' | 'glassHi';

/**
 * The glass material as a style object, ready to spread onto any floating
 * surface that is not shaped like a `<Glass>` box — a fixed-position toast, a
 * launcher pill, an inspector rail.
 *
 * `reduceTransparency` collapses it to the theme's flat surface and drops the
 * filter entirely (not merely raising the alpha): under that setting there is
 * no compositing pass left to pay for, which is half the reason people turn it
 * on. The elevation shadows stay — a scrim is transparency, a drop shadow is
 * depth, and the accessibility setting is about the former.
 *
 * NOTE the `WebkitBackdropFilter` twin. Unprefixed `backdrop-filter` only
 * landed in Safari 18, and our `minimumSystemVersion` is macOS 11 (Safari 14),
 * so the prefix is load-bearing for the users on the old floor — which is
 * exactly the kind of detail that goes missing when thirty files hand-roll the
 * same effect.
 */
export function glassSurface(surface: GlassSurface, reduceTransparency: boolean): CSSProperties {
  if (reduceTransparency) {
    return { background: surface.opaque, boxShadow: surface.boxShadow };
  }
  return {
    background: surface.background,
    backdropFilter: surface.backdropFilter,
    WebkitBackdropFilter: surface.backdropFilter,
    boxShadow: surface.boxShadow,
  } as CSSProperties;
}

/**
 * Hook form, for surfaces that build their own style object.
 * Also starts the Reduce-Transparency bridge, since a surface asking for glass
 * is precisely when the answer starts mattering.
 */
export function useGlass(variant: GlassVariant = 'glass'): CSSProperties {
  const { glass, reduceTransparency } = useTheme();
  useEffect(() => { initReduceTransparencyBridge(); }, []);
  return glassSurface(glass[variant], reduceTransparency);
}

/**
 * A glass box.
 *
 * `r` and `padding` keep the wizard's inherited geometry as defaults (14 / 18)
 * so promoting this primitive out of `wizard/atoms.tsx` changes the wizard's
 * material and nothing else. New callers should pass `radius.glass` for a
 * top-level floating surface and derive any nested radius with
 * `concentric(r, padding)` rather than picking a second number by eye.
 *
 * The material goes on THIS element and not on its children — "make sure to
 * apply the material directly to the control, not its inner views"
 * (WWDC25/356). A group of controls shares one glass shape; it is not five
 * glass buttons.
 */
export function Glass({
  children,
  r = 14,
  padding = 18,
  variant = 'glass',
  style = {},
}: {
  children: ReactNode;
  r?: number;
  padding?: number;
  /** `glassHi` for LARGE surfaces — sidebars, inspectors. More opaque (D6). */
  variant?: GlassVariant;
  style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const material = useGlass(variant);
  return (
    <div style={{
      position: 'relative',
      ...material,
      border: `1px solid ${colors.borderHi}`,
      borderRadius: r,
      padding,
      ...style,
    }}>
      {children}
    </div>
  );
}
