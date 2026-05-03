/** Permagent design tokens — ported from Claude Design handoff (tokens.js) */

export const color = {
  bg: '#0B1220',
  bgDeeper: '#070B14',
  surface: '#1E2433',
  surfaceHi: '#262D3F',
  border: 'rgba(255,255,255,0.07)',
  borderHi: 'rgba(0,213,255,0.18)',
  cyan: '#00D5FF',
  cyanSoft: 'rgba(0,213,255,0.14)',
  cyanGlow: 'rgba(0,213,255,0.45)',
  purple: '#8D44AE',
  purpleBright: '#A855CC',
  purpleSoft: 'rgba(141,68,174,0.18)',
  purpleGlow: 'rgba(141,68,174,0.45)',
  text: '#FFFFFF',
  textMuted: '#8A94A6',
  textDim: '#5A6478',
  danger: '#FFB4A2',
} as const;

export const font = {
  display: '"Manrope", "Satoshi", -apple-system, BlinkMacSystemFont, sans-serif',
  body: '"Inter", -apple-system, BlinkMacSystemFont, sans-serif',
  mono: '"JetBrains Mono", ui-monospace, SFMono-Regular, monospace',
} as const;

export const ease = {
  out: 'cubic-bezier(0.22, 1, 0.36, 1)',
  inOut: 'cubic-bezier(0.65, 0, 0.35, 1)',
  spring: 'cubic-bezier(0.34, 1.56, 0.64, 1)',
} as const;

export const radius = { sm: 6, md: 10, lg: 14, xl: 20, pill: 999 } as const;

export const shadow = {
  glow: '0 0 40px rgba(0,213,255,0.25)',
  glowStrong: '0 0 80px rgba(0,213,255,0.4)',
  card: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
} as const;

export const tokens = { color, font, ease, radius, shadow } as const;
export type DesignTokens = typeof tokens;

// ── Theme gradients ─────────────────────────────────────────────────
export type ThemeId = 'dark' | 'aurora' | 'slate';

export const THEME_GRADIENTS: Record<ThemeId, { workspace: string; card: string; label: string }> = {
  dark: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 50%, #050810 100%)',
    card: 'linear-gradient(180deg, rgba(20,28,48,0.7), rgba(11,18,32,0.7))',
    label: 'Permagent dark',
  },
  aurora: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #1a1040 0%, #0B1220 40%, #2d1050 100%)',
    card: 'linear-gradient(180deg, rgba(45,16,80,0.5), rgba(11,18,32,0.7))',
    label: 'Aurora',
  },
  slate: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #1e2430 0%, #161B26 50%, #0f1318 100%)',
    card: 'linear-gradient(180deg, rgba(30,36,48,0.7), rgba(22,27,38,0.7))',
    label: 'Slate',
  },
};

// Reactive theme — read by components, set by Appearance panel
let _activeTheme: ThemeId = (typeof localStorage !== 'undefined' ? localStorage.getItem('permagent-theme') as ThemeId : null) || 'dark';
const _listeners: Set<() => void> = new Set();

export function getTheme(): ThemeId { return _activeTheme; }
export function getThemeGradient() { return THEME_GRADIENTS[_activeTheme]; }
export function setTheme(id: ThemeId) {
  _activeTheme = id;
  try { localStorage.setItem('permagent-theme', id); } catch { /* */ }
  _listeners.forEach(fn => fn());
}
export function onThemeChange(fn: () => void) { _listeners.add(fn); return () => { _listeners.delete(fn); }; }
