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

// ── Theme gradients + colors ────────────────────────────────────────
export type ThemeId = 'dark' | 'aurora' | 'silver';

/** Per-theme color overrides. Components use useTheme().colors for theme-aware colors. */
export interface ThemeColors {
  bg: string; bgDeeper: string; surface: string; surfaceHi: string;
  border: string; borderHi: string;
  cyan: string; cyanSoft: string; cyanGlow: string;
  purple: string; purpleBright: string; purpleSoft: string; purpleGlow: string;
  text: string; textMuted: string; textDim: string;
  danger: string;
  /** Card elevation shadow (cool-tinted on silver) */
  cardShadow: string;
  /** Top-edge highlight for metallic cards (empty string on dark themes) */
  cardHighlight: string;
  /** Brand ribbon gradient for primary buttons / AI moments */
  ribbonGradient: string;
  /** User chat bubble surface */
  userBubble: string;
  /** User chat bubble text */
  userBubbleText: string;
  /** Inset surface for inputs */
  inputBg: string;
  /** On-accent text (white on gradient/cyan buttons) */
  textOnAccent: string;
  /** Success semantic */
  success: string;
  /** Warning semantic */
  warning: string;
  /** Inline code background */
  codeBg: string;
  /** Inline code text */
  codeText: string;
}

const DARK_COLORS: ThemeColors = {
  bg: color.bg, bgDeeper: color.bgDeeper, surface: color.surface, surfaceHi: color.surfaceHi,
  border: color.border, borderHi: color.borderHi,
  cyan: color.cyan, cyanSoft: color.cyanSoft, cyanGlow: color.cyanGlow,
  purple: color.purple, purpleBright: color.purpleBright, purpleSoft: color.purpleSoft, purpleGlow: color.purpleGlow,
  text: color.text, textMuted: color.textMuted, textDim: color.textDim,
  danger: color.danger,
  cardShadow: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
  cardHighlight: '',
  ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #8D44AE 100%)',
  userBubble: 'rgba(141,68,174,0.18)',
  userBubbleText: '#FFFFFF',
  inputBg: '#1E2433',
  textOnAccent: '#FFFFFF',
  success: '#34D399',
  warning: '#FBBF24',
  codeBg: 'rgba(0,0,0,0.30)',
  codeText: '#00D5FF',
};
const AURORA_COLORS: ThemeColors = {
  ...DARK_COLORS,
  ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #A855CC 100%)',
};
const SILVER_COLORS: ThemeColors = {
  // Surfaces — Pearl White base, white glass cards
  bg: '#F8FAFC',             // Pearl White (page background)
  bgDeeper: '#EEF2F7',      // Chrome Mist (secondary/inset bg)
  surface: '#FFFFFF',        // Pure white glass cards
  surfaceHi: '#F8FAFC',     // Elevated (modals, popovers)
  // Borders — Titanium Gray hairline
  border: 'rgba(167,176,190,0.35)',
  borderHi: 'rgba(0,191,239,0.40)',  // Cyan focus
  // Accents — brand cyan/violet
  cyan: '#00BFEF',           // Cyan Intelligence
  cyanSoft: 'rgba(0,191,239,0.10)',
  cyanGlow: 'rgba(0,191,239,0.25)',
  purple: '#8B5CFF',         // Violet Memory
  purpleBright: '#9B6FFF',
  purpleSoft: 'rgba(139,92,255,0.10)',
  purpleGlow: 'rgba(139,92,255,0.25)',
  // Text — Graphite
  text: '#1E2530',           // Graphite Text (primary) — 14.5:1 on white
  textMuted: '#4B5563',      // Cool grey — 7.0:1 on white (AA)
  textDim: '#6B7585',        // Titanium Gray — 4.9:1 on white (AA body text)
  // Semantic
  danger: '#DC2626',
  // Elevation — soft shadow + glass edge (cards MUST float via shadow, not color)
  cardShadow: '0 2px 12px rgba(30,37,48,0.10), 0 1px 4px rgba(30,37,48,0.06)',
  cardHighlight: 'inset 0 1px 0 rgba(255,255,255,0.9)',
  // Brand ribbon — weighted so text sits over blue→violet (AA large text)
  ribbonGradient: 'linear-gradient(135deg, #00BFEF 0%, #3A7BFF 30%, #8B5CFF 100%)',
  userBubble: '#DCCBFF',     // Soft Lavender
  userBubbleText: '#1E2530', // Graphite (BLACK text)
  inputBg: '#EEF2F7',       // Chrome Mist (recessed)
  textOnAccent: '#FFFFFF',
  success: '#059669',
  warning: '#D97706',
  codeBg: '#EEF2F7',          // Chrome Mist — 1.1:1 vs white (subtle tint)
  codeText: '#0369A1',        // Sky-700 — 5.5:1 on Chrome Mist (AA)
};

export interface ThemeGradients {
  workspace: string; card: string; label: string;
  shell: string; sidebar: string; navRail: string;
  dropdown: string; dropdownSolid: string;
}

export const THEME_GRADIENTS: Record<ThemeId, ThemeGradients> = {
  dark: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 50%, #050810 100%)',
    card: 'linear-gradient(180deg, rgba(20,28,48,0.7), rgba(11,18,32,0.7))',
    shell: '#0B1220',
    sidebar: 'rgba(7,11,20,0.6)',
    navRail: 'rgba(7,11,20,0.4)',
    dropdown: 'rgba(11,18,32,0.98)',
    dropdownSolid: '#0B1220',
    label: 'Permagent dark',
  },
  aurora: {
    workspace: 'radial-gradient(120% 80% at 50% 0%, #1a1040 0%, #0B1220 40%, #2d1050 100%)',
    card: 'linear-gradient(180deg, rgba(45,16,80,0.5), rgba(11,18,32,0.7))',
    shell: '#0e0a1e',
    sidebar: 'rgba(14,10,30,0.7)',
    navRail: 'rgba(14,10,30,0.5)',
    dropdown: 'rgba(14,10,30,0.98)',
    dropdownSolid: '#0e0a1e',
    label: 'Aurora',
  },
  silver: {
    workspace: 'linear-gradient(180deg, #F8FAFC 0%, #F2F5F9 100%)',
    card: 'linear-gradient(180deg, #FFFFFF 0%, #FAFBFD 100%)',
    shell: '#D8DEE8',        // Liquid Silver (window chrome / headers)
    sidebar: 'rgba(238,242,247,0.92)',
    navRail: 'rgba(238,242,247,0.75)',
    dropdown: 'rgba(255,255,255,0.98)',
    dropdownSolid: '#FFFFFF',
    label: 'Silver',
  },
};

const THEME_COLORS: Record<ThemeId, ThemeColors> = {
  dark: DARK_COLORS,
  aurora: AURORA_COLORS,
  silver: SILVER_COLORS,
};

export function getThemedColors(): ThemeColors { return THEME_COLORS[_activeTheme]; }

// ── Reactive appearance prefs ────────────────────────────────────────
// Persisted to localStorage, reactive via listener set.

const _listeners: Set<() => void> = new Set();
function _notify() { _listeners.forEach(fn => fn()); }
function _get(key: string, fallback: string): string {
  try { return localStorage.getItem(key) ?? fallback; } catch { return fallback; }
}
function _set(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch { /* */ }
  _notify();
}

// Theme
let _activeTheme: ThemeId = _get('permagent-theme', 'dark') as ThemeId;
// Migrate 'slate' -> 'silver' (one-time, idempotent)
if ((_activeTheme as string) === 'slate') {
  _activeTheme = 'silver'; _set('permagent-theme', 'silver');
}
export function getTheme(): ThemeId { return _activeTheme; }
export function getThemeGradient() { return THEME_GRADIENTS[_activeTheme]; }
export function setTheme(id: ThemeId) { _activeTheme = id; _set('permagent-theme', id); }

// Sync CSS custom properties for Tailwind theme-aware colors
function _syncCssVars() {
  if (typeof document === 'undefined') return;
  const c = THEME_COLORS[_activeTheme];
  const root = document.documentElement.style;
  root.setProperty('--tw-dark-bg', c.bg);
  root.setProperty('--tw-dark-surface', c.surface);
  root.setProperty('--tw-dark-surface-2', c.surfaceHi);
  root.setProperty('--tw-dark-border', c.border);
  root.setProperty('--tw-dark-text', c.text);
  root.setProperty('--tw-dark-muted', c.textMuted);
  root.setProperty('--tw-accent', c.cyan);
  root.setProperty('--tw-accent-dim', c.cyan);
  root.setProperty('--tw-accent-glow', c.cyanSoft);
  root.setProperty('--tw-input-bg', c.inputBg);
  root.setProperty('--tw-danger', c.danger);
  root.setProperty('--tw-success', c.success);
  root.setProperty('--tw-warning', c.warning);
}
_syncCssVars(); // initial sync
_listeners.add(_syncCssVars); // re-sync on theme change

// Cross-window theme sync: listen for localStorage changes from other windows
// (e.g., chat window picks up theme change made in main window's Settings)
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === 'permagent-theme' && e.newValue) {
      _activeTheme = e.newValue as ThemeId;
      _notify();
    }
  });
}

// Möbius glow (0-100)
export function getMobiusGlow(): number { return parseInt(_get('permagent-mobius-glow', '70'), 10); }
export function setMobiusGlow(v: number) { _set('permagent-mobius-glow', String(v)); }

// Möbius idle animation: 'still' | 'breathing' | 'drifting'
export type IdleAnim = 'still' | 'breathing' | 'drifting';
export function getIdleAnim(): IdleAnim { return _get('permagent-idle-anim', 'breathing') as IdleAnim; }
export function setIdleAnim(v: IdleAnim) { _set('permagent-idle-anim', v); }

// Show Möbius in dashboard hero
export function getShowHeroMobius(): boolean { return _get('permagent-show-hero-mobius', 'true') === 'true'; }
export function setShowHeroMobius(v: boolean) { _set('permagent-show-hero-mobius', String(v)); }

// UI density: 'comfortable' | 'default' | 'compact'
export type UIDensity = 'comfortable' | 'default' | 'compact';
export function getDensity(): UIDensity { return _get('permagent-density', 'default') as UIDensity; }
export function setDensity(v: UIDensity) { _set('permagent-density', v); }

// Reduce motion
export function getReduceMotion(): boolean { return _get('permagent-reduce-motion', 'false') === 'true'; }
export function setReduceMotion(v: boolean) { _set('permagent-reduce-motion', String(v)); }

export function onThemeChange(fn: () => void) { _listeners.add(fn); return () => { _listeners.delete(fn); }; }
