/** @type {import('tailwindcss').Config} */
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
          bg: 'var(--tw-dark-bg, #0a0e17)',
          surface: 'var(--tw-dark-surface, #111827)',
          'surface-2': 'var(--tw-dark-surface-2, #1a2233)',
          border: 'var(--tw-dark-border, #1e293b)',
          text: 'var(--tw-dark-text, #e2e8f0)',
          muted: 'var(--tw-dark-muted, #64748b)',
        },
        accent: {
          DEFAULT: 'var(--tw-accent, #00D5FF)',
          dim: 'var(--tw-accent-dim, #00B0D4)',
          glow: 'var(--tw-accent-glow, rgba(0, 213, 255, 0.15))',
        },
        status: {
          ok: '#22c55e',
          warn: '#f59e0b',
          error: '#ef4444',
          info: '#3b82f6',
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
