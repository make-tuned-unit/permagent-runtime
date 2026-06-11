import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const isTauri = !!process.env.TAURI_ENV_PLATFORM;

export default defineConfig({
  plugins: [react()],
  base: isTauri ? '/' : '/ui/',
  server: {
    port: 5273,
    host: '0.0.0.0',
    proxy: {
      '/api': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/sessions': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/reply': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/agent': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/permagent': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/events': { target: 'http://127.0.0.1:3010', ws: true, changeOrigin: true },
      '/config': { target: 'http://127.0.0.1:3010', changeOrigin: true },
      '/status': { target: 'http://127.0.0.1:3010', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
