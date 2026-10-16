// Browser acceptance against the test-owned daemon on :3002. Never proxy :3001.
import {createServer} from 'vite';
import react from '@vitejs/plugin-react';
import {fileURLToPath} from 'node:url';
import {readFile} from 'node:fs/promises';
import {join} from 'node:path';
const directory = process.env.PERMAGENT_UX_BROWSER_DIR;
if (!directory) throw new Error('Pass the directory printed by run_rust_tests.py --browser');
const root = fileURLToPath(new URL('..', import.meta.url));
const server = await createServer({
  configFile: false, root, base: '/ui/',
  // Same-origin requests reach only the explicit test daemon through the proxy.
  define: {'import.meta.env.VITE_DAEMON_URL': '""', 'import.meta.env.VITE_API_BASE_URL': '""'},
  plugins: [react(), {name: 'isolated-ux-acceptance', configureServer(vite) {
    vite.middlewares.use(async (req, res, next) => {
      if (req.url?.startsWith('/ui/')) return next();
      let state;
      try { state = JSON.parse(await readFile(join(directory, 'state.json'), 'utf8')); }
      catch { res.statusCode = 503; return res.end('The isolated staging test is not ready.'); }
      if (req.url === '/__ux_review') {
        if (req.method !== 'GET') { res.statusCode = 405; return res.end(); }
        res.setHeader('Content-Type', 'application/json');
        res.setHeader('Cache-Control', 'no-store');
        return res.end(JSON.stringify(state));
      }
      const ownedProject = req.method === 'PATCH' && req.url === `/api/projects/${state.projectId}`;
      const ownedPerson = req.method === 'PATCH' && req.url === `/api/people/${state.personId}/fields`;
      const configRead = req.method === 'POST' && req.url === '/config/read';
      if (!['GET', 'HEAD', 'OPTIONS'].includes(req.method) && !ownedProject && !ownedPerson && !configRead) {
        res.statusCode = 405;
        return res.end('This review allows reads and edits to its two synthetic records only.');
      }
      next();
    });
  }}],
  resolve: {dedupe: ['react', 'react-dom']}, cacheDir: '../../reports/ux-dag/.vite-staging',
  server: {host: '127.0.0.1', port: 5201, strictPort: true,
    proxy: {
      // Production views subscribe to this read-only authenticated stream.
      '^/events(?:\\?|$)': {target: 'http://127.0.0.1:3002', ws: true},
      '^/(?!ui/|__ux_review|events(?:\\?|$))': {target: 'http://127.0.0.1:3002'},
    }},
});
await server.listen();
console.log('Isolated browser acceptance: http://127.0.0.1:5201/ui/scripts/ux-staging.html');
