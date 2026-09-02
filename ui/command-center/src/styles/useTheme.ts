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
    reduceMotion: getReduceMotion(),
  };
}

export type { ThemeId, ThemeColors, ThemeGlass, GlassSurface, IdleAnim, UIDensity };
