import { useState, useEffect } from 'react';
import { getTheme, getThemeGradient, onThemeChange, type ThemeId } from './tokens';

export function useTheme() {
  const [, setTick] = useState(0);
  useEffect(() => onThemeChange(() => setTick(t => t + 1)), []);
  return { theme: getTheme(), gradient: getThemeGradient() };
}

export type { ThemeId };
