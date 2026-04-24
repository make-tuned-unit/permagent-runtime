import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const isTauri = !!process.env.TAURI_ENV_PLATFORM;

export default defineConfig({
  plugins: [react()],
  base: isTauri ? '/' : '/ui/',
  server: {
    port: 5173,
    host: '0.0.0.0',
    proxy: {
      '/sessions': 'http://localhost:3000',
      '/reply': 'http://localhost:3000',
      '/agent': 'http://localhost:3000',
      '/permagent': 'http://localhost:3000',
      '/events': { target: 'http://localhost:3000', ws: true },
      '/config': 'http://localhost:3000',
      '/status': 'http://localhost:3000',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
