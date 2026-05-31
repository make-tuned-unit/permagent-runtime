import { useState, useEffect } from 'react';
import {
  getTheme, getThemeGradient, getThemedColors, onThemeChange,
  getMobiusGlow, getIdleAnim, getShowHeroMobius,
  getDensity, getReduceMotion,
} from './tokens';
import type { ThemeId, ThemeColors, IdleAnim, UIDensity } from './tokens';

export function useTheme() {
  const [, setTick] = useState(0);
  useEffect(() => onThemeChange(() => setTick(t => t + 1)), []);
  return {
    theme: getTheme(),
    gradient: getThemeGradient(),
    colors: getThemedColors(),
    mobiusGlow: getMobiusGlow(),
    idleAnim: getIdleAnim(),
    showHeroMobius: getShowHeroMobius(),
    density: getDensity(),
    reduceMotion: getReduceMotion(),
  };
}

export type { ThemeId, ThemeColors, IdleAnim, UIDensity };
