import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const isTauri = !!process.env.TAURI_ENV_PLATFORM;

export default defineConfig({
  plugins: [react()],
  // Expose only this explicitly non-secret PERMAGENT flag in addition to
  // Vite's normal VITE_* variables.
  envPrefix: ['VITE_', 'PERMAGENT_SHORTLIVED_STREAM_TOKEN'],
  base: isTauri ? '/' : '/ui/',
  server: {
    port: 5273,
    host: '0.0.0.0',
    proxy: {
      '/api': { target: 'http://localhost:3001', changeOrigin: true },
      '/sessions': { target: 'http://localhost:3001', changeOrigin: true },
      '/reply': { target: 'http://localhost:3001', changeOrigin: true },
      '/agent': { target: 'http://localhost:3001', changeOrigin: true },
      '/permagent': { target: 'http://localhost:3001', changeOrigin: true },
      '/events': { target: 'http://localhost:3001', ws: true, changeOrigin: true },
      '/config': { target: 'http://localhost:3001', changeOrigin: true },
      '/status': { target: 'http://localhost:3001', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
