/** @type {import('tailwindcss').Config} */
/*
 * Design-system bridge: every color here is backed by a CSS custom property
 * synced from tokens.ts _syncCssVars(). Solid colors are stored as RGB channel
 * triplets (e.g. "11 18 32") so Tailwind's /NN alpha modifiers compose correctly.
 * Colors with intrinsic alpha (border, glow) are stored as full rgba values and
 * do NOT support further alpha modification — that's intentional.
 *
 * Source of truth: src/styles/tokens.ts  (ThemeColors)
 * Bridge:          src/styles/tokens.ts  (_syncCssVars)
 * Layout/spacing:  Tailwind utilities    (keep using freely)
 * Colors:          useTheme().colors      (preferred in new code)
 */
export default {
  darkMode: 'selector',
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        dark: {
          bg: 'rgb(var(--tw-dark-bg, 11 18 32) / <alpha-value>)',
          surface: 'rgb(var(--tw-dark-surface, 30 36 51) / <alpha-value>)',
          'surface-2': 'rgb(var(--tw-dark-surface-2, 38 45 63) / <alpha-value>)',
          border: 'var(--tw-dark-border, rgba(255,255,255,0.07))',
          text: 'rgb(var(--tw-dark-text, 255 255 255) / <alpha-value>)',
          muted: 'rgb(var(--tw-dark-muted, 138 148 166) / <alpha-value>)',
        },
        accent: {
          DEFAULT: 'rgb(var(--tw-accent, 0 213 255) / <alpha-value>)',
          dim: 'rgb(var(--tw-accent-dim, 0 213 255) / <alpha-value>)',
          glow: 'var(--tw-accent-glow, rgba(0, 213, 255, 0.15))',
        },
        status: {
          ok: 'rgb(var(--tw-status-ok, 52 211 153) / <alpha-value>)',
          warn: 'rgb(var(--tw-status-warn, 251 191 36) / <alpha-value>)',
          error: 'rgb(var(--tw-status-error, 255 180 162) / <alpha-value>)',
          info: 'rgb(var(--tw-status-info, 0 213 255) / <alpha-value>)',
        },
      },
      fontFamily: {
        sans: ['"Inter"', '-apple-system', 'BlinkMacSystemFont', 'sans-serif'],
        display: ['"Manrope"', '"Satoshi"', '-apple-system', 'BlinkMacSystemFont', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'ui-monospace', 'SFMono-Regular', 'monospace'],
      },
      animation: {
        'pulse-slow': 'pulse 3s cubic-bezier(0.4, 0, 0.2, 1) infinite',
        'glow': 'glow 2s cubic-bezier(0.4, 0, 0.2, 1) infinite alternate',
      },
      keyframes: {
        glow: {
          '0%': { boxShadow: '0 0 5px rgba(0, 213, 255, 0.2)' },
          '100%': { boxShadow: '0 0 20px rgba(0, 213, 255, 0.4)' },
        },
      },
    },
  },
  plugins: [],
}
