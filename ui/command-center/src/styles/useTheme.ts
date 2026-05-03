import { useState, useEffect } from 'react';
import {
  getTheme, getThemeGradient, onThemeChange,
  getMobiusGlow, getIdleAnim, getShowHeroMobius,
  getDensity, getReduceMotion,
  type ThemeId, type IdleAnim, type UIDensity,
} from './tokens';

export function useTheme() {
  const [, setTick] = useState(0);
  useEffect(() => onThemeChange(() => setTick(t => t + 1)), []);
  return {
    theme: getTheme(),
    gradient: getThemeGradient(),
    mobiusGlow: getMobiusGlow(),
    idleAnim: getIdleAnim(),
    showHeroMobius: getShowHeroMobius(),
    density: getDensity(),
    reduceMotion: getReduceMotion(),
  };
}

export type { ThemeId, IdleAnim, UIDensity };
