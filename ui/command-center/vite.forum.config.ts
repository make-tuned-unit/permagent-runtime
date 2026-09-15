import { defineConfig, mergeConfig } from 'vite';
import base from './vite.config';

// Separate entry/output: the exploration never replaces the shipping app.
export default mergeConfig(base, defineConfig({
  build: { outDir: 'dist/forum', rollupOptions: { input: 'forum.html' } },
}));
