import {createServer} from 'vite';
import react from '@vitejs/plugin-react';
import {fileURLToPath} from 'node:url';
const root = fileURLToPath(new URL('..', import.meta.url));
const server = await createServer({configFile: false, root, base: '/ui/', plugins: [react()], resolve: {dedupe: ['react', 'react-dom']}, cacheDir: '../../reports/home-ui/.vite-review', server: {host: '127.0.0.1', port: Number(process.env.PERMAGENT_REVIEW_UI_PORT) || 5199, strictPort: true, proxy: {}}});
await server.listen();
console.log(`Synthetic UI review: http://127.0.0.1:${server.config.server.port}/ui/scripts/ux-review.html`);
