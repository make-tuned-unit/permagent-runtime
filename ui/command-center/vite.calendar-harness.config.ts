import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));
export default defineConfig({
  root: here('./harness/calendar'),
  plugins: [react()],
  resolve: {
    alias: {
      '@tauri-apps/api/core': here('./harness/calendar/tauri-core-mock.ts'),
      '@tauri-apps/plugin-shell': here('./harness/calendar/shell-mock.ts'),
    },
  },
  server: { port: 5391, strictPort: true, fs: { allow: [here('.'), here('../../..')] } },
  css: { postcss: here('./postcss.config.js') },
});
