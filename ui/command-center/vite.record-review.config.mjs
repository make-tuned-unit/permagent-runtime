// Isolated browser QA only. Never merge the production/default dev proxy table.
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';

const root = fileURLToPath(new URL('.', import.meta.url));
export const daemonTarget = 'http://127.0.0.1:3002';
export const browserOrigin = 'http://127.0.0.1:5302';
export const daemonPrefixes = [
  'action-required', 'activity', 'agent', 'api', 'automation', 'collect', 'config',
  'debug', 'diagnostics', 'events', 'features', 'gateway', 'handle_nanogpt',
  'handle_openrouter', 'handle_tetrate', 'integrations', 'local-inference', 'pair',
  'permagent', 'recipes', 'reply', 'schedule', 'sessions', 'sse-token', 'status',
  'system_info', 'telemetry', 'terminal', 'tunnel', 'voice',
];
const proxyKey = `^/(?:${daemonPrefixes.join('|')})(?:/|\\?|$)`;
// Vite proxy errors may include a stream-token query. Keep credential-bearing
// request URLs/error objects out of terminal logs; the browser still sees errors.
const redactedLogger = {
  info(message) { console.info(String(message).replace(/([?#&](?:token|claim|code)=)[^\s&#]+/gi, '$1[redacted]')); },
  warn() { console.warn('[record-review] Vite warning; inspect the browser for details.'); },
  warnOnce() { this.warn(); },
  error() { this.hasWarned = true; console.error('[record-review] Vite request/build error; details suppressed.'); },
  clearScreen() {}, hasErrorLogged() { return true; }, hasWarned: false,
};
export default {
  root, base: '/ui/', plugins: [react()], clearScreen: false,
  customLogger: redactedLogger,
  envPrefix: [], envDir: resolve(root, '../../reports/record-evidence/empty-vite-env'),
  define: {
    'import.meta.env.VITE_DAEMON_URL': JSON.stringify(browserOrigin),
    'import.meta.env.VITE_API_BASE_URL': JSON.stringify(browserOrigin),
  },
  resolve: { dedupe: ['react', 'react-dom', 'three', '@react-three/fiber', '@react-three/drei', '@react-three/postprocessing'] },
  server: {
    host: '127.0.0.1', port: 5302, strictPort: true, open: false,
    cors: { origin: browserOrigin },
    hmr: { host: '127.0.0.1', clientPort: 5302 },
    fs: { strict: true, allow: [root, resolve(root, '../analytics-client')],
          deny: ['.env', '.env.*', '*.{crt,pem,key}', '**/.git/**'] },
    proxy: { [proxyKey]: { target: daemonTarget, changeOrigin: true, ws: true } },
  },
};
