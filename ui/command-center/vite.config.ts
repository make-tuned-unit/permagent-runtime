// `vitest/config` rather than `vite` so the `test` block below is typed. The
// only reason it exists: the shared analytics client lives in a sibling
// package (ui/analytics-client) and the repo's frontend gates — `npx tsc
// --noEmit` and `npx vitest run` — are run from here. A shared package that
// no gate executes is a shared package nobody can trust.
import { defineConfig } from 'vitest/config';
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
      // Missing from the proxy list, so under `base: '/ui/'` these fell
      // through to Vite's own handler and 404'd — the Automate tab showed
      // "Couldn't load automations. Unknown error" in `npm run dev`, which
      // reads as a broken daemon rather than a missing proxy entry.
      '/schedule': { target: 'http://localhost:3001', changeOrigin: true },
      '/activity': { target: 'http://localhost:3001', changeOrigin: true },
      '/voice': { target: 'http://localhost:3001', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  test: {
    include: [
      'src/**/*.{test,spec}.{ts,tsx}',
      '../analytics-client/src/**/*.{test,spec}.ts',
    ],
    coverage: {
      provider: 'v8',
      reporter: ['text-summary', 'json-summary', 'lcov'],
      reportsDirectory: 'coverage',
      // An explicit include is what makes the percentage comparable run to
      // run: without it the denominator is only whatever the tests imported.
      // It cannot reach ../analytics-client — coverage globs are resolved
      // under the project root — so that package's sources are executed by
      // the suite above but are not part of this number.
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['**/*.{test,spec}.{ts,tsx}', '**/*.d.ts'],
    },
  },
});
