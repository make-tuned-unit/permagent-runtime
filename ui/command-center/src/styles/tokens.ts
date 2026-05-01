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
