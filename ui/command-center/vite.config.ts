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
      '/api': 'http://localhost:3001',
      '/sessions': 'http://localhost:3001',
      '/reply': 'http://localhost:3001',
      '/agent': 'http://localhost:3001',
      '/permagent': 'http://localhost:3001',
      '/events': { target: 'http://localhost:3001', ws: true },
      '/config': 'http://localhost:3001',
      '/status': 'http://localhost:3001',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
