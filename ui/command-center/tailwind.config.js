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
          bg: '#0a0e17',       // deep midnight - primary background
          surface: '#111827',  // raised surfaces
          'surface-2': '#1a2233', // secondary surface (cards)
          border: '#1e293b',   // subtle borders
          text: '#e2e8f0',     // primary text
          muted: '#64748b',    // secondary text
        },
        accent: {
          DEFAULT: '#00ffb4',  // primary accent - terminal green
          dim: '#00cc90',      // dimmed accent
          glow: 'rgba(0, 255, 180, 0.15)', // glow effect
        },
        status: {
          ok: '#22c55e',
          warn: '#f59e0b',
          error: '#ef4444',
          info: '#3b82f6',
        },
      },
      fontFamily: {
        sans: ['-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', '"Helvetica Neue"', 'Arial', 'sans-serif'],
        mono: ['"JetBrains Mono"', '"SF Mono"', '"Fira Code"', 'monospace'],
      },
      animation: {
        'pulse-slow': 'pulse 3s cubic-bezier(0.4, 0, 0.2, 1) infinite',
        'glow': 'glow 2s cubic-bezier(0.4, 0, 0.2, 1) infinite alternate',
      },
      keyframes: {
        glow: {
          '0%': { boxShadow: '0 0 5px rgba(0, 255, 180, 0.2)' },
          '100%': { boxShadow: '0 0 20px rgba(0, 255, 180, 0.4)' },
        },
      },
    },
  },
  plugins: [],
}
