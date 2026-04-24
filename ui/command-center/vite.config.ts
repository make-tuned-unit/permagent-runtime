import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  base: '/ui/',
  server: {
    port: 5173,
    host: '0.0.0.0',
    proxy: {
      '/sessions': 'http://localhost:3001',
      '/skills': 'http://localhost:3001',
      '/memories': 'http://localhost:3001',
      '/events': 'http://localhost:3001',
      '/config': 'http://localhost:3001',
      '/status': 'http://localhost:3001',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
