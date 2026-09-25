import { useState, useEffect } from 'react';
import {
  getTheme, getThemePref, getThemeGradient, getThemedColors, getThemedGlass, onThemeChange,
  getMobiusGlow, getIdleAnim,
  getDensity, getReduceMotion, getReduceTransparency,
} from './tokens';
import type { ThemeId, ThemeColors, ThemeGlass, GlassSurface, IdleAnim, UIDensity } from './tokens';

export function useTheme() {
  const [, setTick] = useState(0);
  useEffect(() => onThemeChange(() => setTick(t => t + 1)), []);
  const reduceMotion = getReduceMotion();
  // R8: the CSS-only motion primitives (`.pa-pressable`, `.pa-enter`,
  // `.pa-skeleton`, toasts, modal/popover entrances, the agent pulse) have no
  // JS of their own to check `reduceMotion` in — they are plain animations in
  // index.css. The OS's own `prefers-reduced-motion` reaches them already via
  // the app-wide `@media` guard, but the in-app override (Settings > Reduce
  // Motion) previously only reached components that read this hook and
  // disabled their OWN inline animation. Mirroring it onto `<html>` lets the
  // stylesheet key off it too, so a person who turned the setting on inside
  // the app — without their OS agreeing — still gets it everywhere, not just
  // in the surfaces that happened to check.
  useEffect(() => {
    document.documentElement.dataset.reduceMotion = reduceMotion ? 'true' : 'false';
  }, [reduceMotion]);
  return {
    theme: getTheme(),
    themePref: getThemePref(),
    gradient: getThemeGradient(),
    colors: getThemedColors(),
    // The two glass surfaces for the active theme. Prefer the `<Glass>`
    // primitive or `glassSurface()` over spreading these by hand — they are
    // exposed for the cases that genuinely need the raw values.
    glass: getThemedGlass(),
    reduceTransparency: getReduceTransparency(),
    mobiusGlow: getMobiusGlow(),
    idleAnim: getIdleAnim(),
    density: getDensity(),
    reduceMotion,
  };
}

export type { ThemeId, ThemeColors, ThemeGlass, GlassSurface, IdleAnim, UIDensity };
