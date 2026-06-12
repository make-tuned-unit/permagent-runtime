/**
 * Theme-derived xterm palette.
 *
 * Chrome slots (background / foreground / cursor / cursorAccent / selection)
 * are composed from the active theme's `ThemeColors` (useTheme().colors) so
 * the terminal follows the design tokens. ANSI slots have no token
 * counterparts (tokens.ts `danger` #FFB4A2 is unusable as an ANSI red, etc.)
 * and are curated per theme below.
 *
 * tokens.ts is import-only here — it stays xterm-free by design.
 */
import type { ITheme } from '@xterm/xterm';
import { THEME_GRADIENTS } from '../../styles/tokens';
import type { ThemeColors, ThemeId } from '../../styles/tokens';

type AnsiPalette = Pick<
  ITheme,
  | 'black' | 'red' | 'green' | 'yellow'
  | 'blue' | 'magenta' | 'cyan' | 'white'
  | 'brightBlack' | 'brightRed' | 'brightGreen' | 'brightYellow'
  | 'brightBlue' | 'brightMagenta' | 'brightCyan' | 'brightWhite'
>;

/** Curated ANSI set for the dark surfaces (dark + aurora). */
const DARK_ANSI: AnsiPalette = {
  black: '#1e293b',
  red: '#ef4444',
  green: '#5BD17F',
  yellow: '#FF9500',
  blue: '#3b82f6',
  magenta: '#A855CC', // matches token purpleBright
  cyan: '#00D5FF', // matches token cyan
  white: '#e2e8f0',
  brightBlack: '#64748b',
  brightRed: '#f87171',
  brightGreen: '#5BD17F',
  brightYellow: '#FFB340',
  brightBlue: '#60a5fa',
  brightMagenta: '#C893E0',
  brightCyan: '#00D5FF',
  brightWhite: '#f8fafc',
};

/**
 * Curated ANSI set for silver — a LIGHT terminal (dark-on-light), per the
 * codeBg/codeText precedent. Token cyan #00BFEF lacks contrast on a light
 * background, so the ANSI cyan is the darker derived #00838F.
 */
const SILVER_ANSI: AnsiPalette = {
  black: '#5A6577',
  red: '#D32F2F',
  green: '#2E7D32',
  yellow: '#F57C00',
  blue: '#1565C0',
  magenta: '#7B1FA2',
  cyan: '#00838F',
  white: '#1E2530',
  brightBlack: '#8492A6',
  brightRed: '#E53935',
  brightGreen: '#43A047',
  brightYellow: '#FB8C00',
  brightBlue: '#1E88E5',
  brightMagenta: '#8E24AA',
  brightCyan: '#00ACC1',
  brightWhite: '#0F1419',
};

const ANSI: Record<ThemeId, AnsiPalette> = {
  dark: DARK_ANSI,
  aurora: DARK_ANSI,
  silver: SILVER_ANSI,
};

/** '#RRGGBB' → 'rgba(r, g, b, a)'. Solid 6-digit hex only. */
function withAlpha(hex: string, alpha: number): string {
  const s = hex.replace('#', '');
  const r = parseInt(s.substring(0, 2), 16);
  const g = parseInt(s.substring(2, 4), 16);
  const b = parseInt(s.substring(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Build the xterm ITheme for a given app theme.
 *
 * Background: aurora uses THEME_GRADIENTS.aurora.shell (its ThemeColors are
 * shared with dark, so the shell color is what distinguishes it); dark and
 * silver use colors.bgDeeper (silver → #EEF2F7 Chrome Mist).
 */
export function getXtermTheme(themeId: ThemeId, colors: ThemeColors): ITheme {
  const background = themeId === 'aurora' ? THEME_GRADIENTS.aurora.shell : colors.bgDeeper;
  const isLight = themeId === 'silver';
  return {
    background,
    foreground: colors.text,
    cursor: isLight ? colors.text : colors.cyan,
    cursorAccent: background,
    selectionBackground: withAlpha(colors.cyan, 0.2),
    selectionForeground: colors.text,
    ...ANSI[themeId],
  };
}
