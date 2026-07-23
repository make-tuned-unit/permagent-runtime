/** Permagent design tokens — ported from Claude Design handoff (tokens.js) */

/**
 * THE canonical Permagent neon accent (issue #193, ruled 2026-06-23:
 * global accent = #00D5FF). Every cyan/neon accent in the app derives from
 * this constant — never inline a second near-duplicate (a D9 variant was
 * the historical drift). Non-importable surfaces (index.css fallbacks,
 * iOS Theme.swift) mirror this value literally.
 */
export const NEON_ACCENT = '#00D5FF';

export const color = {
  bg: '#0B1220',
  bgDeeper: '#070B14',
  surface: '#1E2433',
  surfaceHi: '#262D3F',
  border: 'rgba(255,255,255,0.07)',
  borderHi: 'rgba(0,213,255,0.18)',
  cyan: NEON_ACCENT,
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
  /** Strong red for ANSI output and high-emphasis destructive states. */
  dangerStrong: '#EF4444',
} as const;

export const font = {
  display: '"Manrope", "Satoshi", -apple-system, BlinkMacSystemFont, sans-serif',
  body: '"Inter", -apple-system, BlinkMacSystemFont, sans-serif',
  mono: '"JetBrains Mono", ui-monospace, SFMono-Regular, monospace',
} as const;

/**
 * Fixed type ramp (design audit: "no arbitrary sizes"). Role-based — each style
 * bundles size + line-height + weight + tracking, so callers spread one token
 * (`style={{ ...type.body }}`) instead of setting the four ad hoc. Tracking
 * tightens as size grows, per the audit. Sizes in px.
 */
export const type = {
  display: { fontSize: 32, lineHeight: '38px', fontWeight: 600, letterSpacing: '-0.02em' },
  title:   { fontSize: 20, lineHeight: '26px', fontWeight: 600, letterSpacing: '-0.01em' },
  heading: { fontSize: 16, lineHeight: '22px', fontWeight: 600, letterSpacing: '-0.005em' },
  body:    { fontSize: 14, lineHeight: '21px', fontWeight: 400 },
  small:   { fontSize: 13, lineHeight: '19px', fontWeight: 400 },
  caption: { fontSize: 12, lineHeight: '16px', fontWeight: 400 },
  micro:   { fontSize: 11, lineHeight: '14px', fontWeight: 500 },
  label:   { fontSize: 11, lineHeight: '14px', fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase' },
} as const;

/** Tabular figures — aligned numerals for metrics/counts/timers (never prose),
 *  so digits don't reflow as values change. Spread onto any numeric display. */
export const tabularNums = { fontVariantNumeric: 'tabular-nums' } as const;

export const ease = {
  out: 'cubic-bezier(0.22, 1, 0.36, 1)',
  inOut: 'cubic-bezier(0.65, 0, 0.35, 1)',
  spring: 'cubic-bezier(0.34, 1.56, 0.64, 1)',
} as const;

/**
 * Motion duration scale (ms). One vocabulary for transitions so surfaces don't
 * hand-roll a grab-bag of 160/200/320 timings (wizard audit #603). `fast` =
 * hover/focus feedback, `base` = element state changes, `slow` = view/crossfade.
 * Pair with an `ease` token: `transition: \`all ${duration.fast}ms ${ease.out}\``.
 */
export const duration = { fast: 160, base: 200, slow: 320 } as const;

export const radius = { sm: 6, md: 10, lg: 14, xl: 20, pill: 999 } as const;

export const shadow = {
  glow: '0 0 40px rgba(0,213,255,0.25)',
  glowStrong: '0 0 80px rgba(0,213,255,0.4)',
  card: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
} as const;

export const tokens = { color, font, type, tabularNums, ease, duration, radius, shadow } as const;
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
  danger: string; dangerStrong: string;
  /** Card elevation shadow (cool-tinted on silver) */
  cardShadow: string;
  /** Discrete elevation ladder for floating layers (theme-aware — deep on the
   *  void, soft-cool on silver where a black shadow is invisible). Use the level
   *  that matches prominence: raised=dropdowns/menus, overlay=popovers/panels,
   *  floating=toasts/notifications. Static content stays border-first, flat. */
  elevationRaised: string;
  elevationOverlay: string;
  elevationFloating: string;
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
  /** On-accent text (white on the blue→violet gradient/ribbon buttons) */
  textOnAccent: string;
  /** Text/iconography sitting on a FLAT cyan accent fill (solid `colors.cyan`
   *  buttons, active pills). Cyan is bright in every theme, so on-cyan text must
   *  be a fixed dark ink — white/`colors.bg` fails WCAG contrast (and inverts to
   *  near-white on the silver theme). Never use textOnAccent on a flat-cyan fill. */
  textOnCyan: string;
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
  dangerStrong: color.dangerStrong,
  cardShadow: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.04)',
  elevationRaised: '0 4px 16px rgba(0,0,0,0.35)',
  elevationOverlay: '0 8px 24px rgba(0,0,0,0.42)',
  elevationFloating: '0 20px 52px rgba(0,0,0,0.5)',
  cardHighlight: '',
  ribbonGradient: 'linear-gradient(135deg, #00D5FF 0%, #6366F1 50%, #8D44AE 100%)',
  userBubble: 'rgba(141,68,174,0.18)',
  userBubbleText: '#FFFFFF',
  inputBg: '#1E2433',
  textOnAccent: '#FFFFFF',
  textOnCyan: '#04141B',
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
  dangerStrong: '#DC2626',
  // Elevation — soft shadow + glass edge (cards MUST float via shadow, not color)
  cardShadow: '0 2px 12px rgba(30,37,48,0.10), 0 1px 4px rgba(30,37,48,0.06)',
  elevationRaised: '0 4px 16px rgba(30,37,48,0.10)',
  elevationOverlay: '0 8px 24px rgba(30,37,48,0.13)',
  elevationFloating: '0 20px 52px rgba(30,37,48,0.17)',
  cardHighlight: 'inset 0 1px 0 rgba(255,255,255,0.9)',
  // Brand ribbon — weighted so text sits over blue→violet (AA large text)
  ribbonGradient: 'linear-gradient(135deg, #00BFEF 0%, #3A7BFF 30%, #8B5CFF 100%)',
  userBubble: '#DCCBFF',     // Soft Lavender
  userBubbleText: '#1E2530', // Graphite (BLACK text)
  inputBg: '#EEF2F7',       // Chrome Mist (recessed)
  textOnAccent: '#FFFFFF',
  textOnCyan: '#04141B',    // Deep ink — ~12:1 on #00BFEF (white would be ~1.9:1)
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
    shell: '#FFFFFF',         // Pure white header (matches content, separated by border)
    sidebar: 'rgba(248,250,252,0.95)',
    navRail: 'rgba(248,250,252,0.85)',
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

// Theme. The stored PREFERENCE may be 'system' (light-awareness, à la
// ChatGPT/Claude): follow the OS — silver during light hours, dark when the
// device is dark, live-switching when the OS does. The resolved _activeTheme
// is always a concrete ThemeId so every consumer keeps working.
export type ThemePref = ThemeId | 'system';

const THEME_PREFS: readonly ThemePref[] = ['dark', 'aurora', 'silver', 'system'];

/** Keep old or manually-edited preferences from producing an undefined token set. */
export function normalizeThemePref(value: string | null): ThemePref {
  if (value === 'slate') return 'silver';
  return THEME_PREFS.includes(value as ThemePref) ? value as ThemePref : 'dark';
}

function _prefersDark(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function _resolve(pref: ThemePref): ThemeId {
  if (pref === 'system') return _prefersDark() ? 'dark' : 'silver';
  return pref;
}

const _storedThemePref = _get('permagent-theme', 'dark');
let _themePref: ThemePref = normalizeThemePref(_storedThemePref);
// Migrate 'slate' -> 'silver' (one-time, idempotent)
if (_storedThemePref === 'slate') {
  _set('permagent-theme', 'silver');
}
let _activeTheme: ThemeId = _resolve(_themePref);
export function getTheme(): ThemeId { return _activeTheme; }
export function getThemePref(): ThemePref { return _themePref; }
export function getThemeGradient() { return THEME_GRADIENTS[_activeTheme]; }
export function setTheme(pref: ThemePref) {
  _themePref = pref;
  _activeTheme = _resolve(pref);
  _set('permagent-theme', pref);
}

// Live OS-theme switching: when the preference is 'system', re-resolve and
// notify on every prefers-color-scheme flip (the day/night shift).
if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (_themePref === 'system') {
      _activeTheme = _resolve('system');
      _notify();
    }
  });
}

// Extract RGB channel triplet from a hex color string for Tailwind alpha-modifier support.
// e.g. '#0B1220' → '11 18 32'. Only call on solid hex values, not rgba().
function _hex(h: string): string {
  const s = h.replace('#', '');
  return `${parseInt(s.substring(0, 2), 16)} ${parseInt(s.substring(2, 4), 16)} ${parseInt(s.substring(4, 6), 16)}`;
}

// Sync CSS custom properties for Tailwind theme-aware colors.
// Solid colors are stored as RGB channel triplets so Tailwind /NN alpha modifiers
// compose correctly: rgb(var(--tw-dark-muted) / 0.5). Colors with intrinsic alpha
// (border, glow) are stored as full rgba values and don't support further alpha.
function _syncCssVars() {
  if (typeof document === 'undefined') return;
  const c = THEME_COLORS[_activeTheme];
  const root = document.documentElement.style;
  // Channel triplets (support Tailwind /NN alpha modifiers)
  root.setProperty('--tw-dark-bg', _hex(c.bg));
  root.setProperty('--tw-dark-surface', _hex(c.surface));
  root.setProperty('--tw-dark-surface-2', _hex(c.surfaceHi));
  root.setProperty('--tw-dark-text', _hex(c.text));
  root.setProperty('--tw-dark-muted', _hex(c.textMuted));
  root.setProperty('--tw-accent', _hex(c.cyan));
  root.setProperty('--tw-accent-dim', _hex(c.cyan));
  root.setProperty('--tw-input-bg', _hex(c.inputBg));
  root.setProperty('--tw-danger', _hex(c.danger));
  root.setProperty('--tw-success', _hex(c.success));
  root.setProperty('--tw-warning', _hex(c.warning));
  // Status palette → bridged to theme tokens (ok→success, warn→warning, error→danger, info→cyan)
  root.setProperty('--tw-status-ok', _hex(c.success));
  root.setProperty('--tw-status-warn', _hex(c.warning));
  root.setProperty('--tw-status-error', _hex(c.danger));
  root.setProperty('--tw-status-info', _hex(c.cyan));
  // Full rgba values (intrinsic alpha, no Tailwind alpha modifier support)
  root.setProperty('--tw-dark-border', c.border);
  root.setProperty('--tw-accent-glow', c.cyanSoft);
  // Scrollbar colors per theme (aligned with tokens.ts dark bg/surface)
  const scrollThumb = _activeTheme === 'silver' ? '#C8CDD5' : '#1E2433';
  const scrollThumbHover = _activeTheme === 'silver' ? '#A0A8B4' : '#262D3F';
  root.setProperty('--scrollbar-thumb', scrollThumb);
  root.setProperty('--scrollbar-thumb-hover', scrollThumbHover);
  // Sync color-scheme + body background so macOS native title bar matches theme
  const scheme = _activeTheme === 'silver' ? 'light' : 'dark';
  root.setProperty('color-scheme', scheme);
}
_syncCssVars(); // initial sync
_listeners.add(_syncCssVars); // re-sync on theme change

// Cross-window theme sync: listen for localStorage changes from other windows
// (e.g., chat window picks up theme change made in main window's Settings)
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === 'permagent-theme') {
      _themePref = normalizeThemePref(e.newValue);
      _activeTheme = _resolve(_themePref);
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
