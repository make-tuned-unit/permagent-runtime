/**
 * Decision Inbox L4 — evidence screenshot capture.
 *
 * Serverless: the app is built with the in-page decisions mock baked in
 * (VITE_DECISIONS_MOCK=1), and Playwright fulfills every request from the
 * dist directory — no dev server process needed.
 *
 * Repro:
 *   cd ui/command-center
 *   npm ci && npm install --no-save playwright && npx playwright install chromium
 *   VITE_DECISIONS_MOCK=1 npx vite build --outDir dist-mock --emptyOutDir
 *   node ../../docs/decision-inbox/capture-l4.cjs
 *   rm -rf dist-mock   # build artifact, not committed
 *
 * (Equivalently: VITE_DECISIONS_MOCK=1 npm run dev + point the script at it
 *  with CAPTURE_BASE_URL=http://localhost:5173 CAPTURE_DIST= .)
 *
 * The decisions data layer runs against the in-page mock. The rest of the
 * daemon API (config, workspaces, dashboard) is fulfilled here with fixtures
 * via Playwright route interception so the app boots without a daemon.
 */

const fs = require('fs');
const path = require('path');
const { chromium } = require(path.join(
  __dirname, '..', '..', 'ui', 'command-center', 'node_modules', 'playwright'
));

const DIST = process.env.CAPTURE_DIST ?? path.join(
  __dirname, '..', '..', 'ui', 'command-center', 'dist-mock'
);
const BASE = process.env.CAPTURE_BASE_URL || 'http://app.local';
const OUT = path.join(__dirname, 'screenshots-l4');
const THEMES = ['dark', 'aurora', 'silver'];

const MIME = {
  '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon',
  '.json': 'application/json', '.woff2': 'font/woff2', '.woff': 'font/woff',
  '.webmanifest': 'application/manifest+json', '.wasm': 'application/wasm',
};

const DASHBOARD_FIXTURE = {
  agent: { name: 'Henry', state: 'thinking', active_count: 2, summary: 'Working across 2 sessions' },
  stats: { sessions_today: 4, sessions_total: 182, memory_count: 5210, memory_delta_today: 14 },
  in_flight: [
    { id: 's1', title: 'Voice latency sweep', started_at: new Date(Date.now() - 42 * 60000).toISOString(), state: 'thinking', progress: 0.62 },
    { id: 's2', title: 'World map speedup', started_at: new Date(Date.now() - 9 * 60000).toISOString(), state: 'thinking', progress: 0.2 },
  ],
  recent: [
    { id: 'r1', title: 'Nightly backup snapshot', state: 'completed', ended_at: new Date(Date.now() - 3 * 3600000).toISOString() },
    { id: 'r2', title: 'Doctor diagnostic run', state: 'completed', ended_at: new Date(Date.now() - 5 * 3600000).toISOString() },
  ],
};

const LAYOUT_FIXTURE = {
  cards: [
    { id: 'hero', type: 'hero', position: { x: 0, y: 0 }, size: { w: 7, h: 4 }, visible: true },
    { id: 'decisions', type: 'decisions', position: { x: 7, y: 0 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'stats', type: 'stats', position: { x: 0, y: 4 }, size: { w: 5, h: 4 }, visible: true },
    { id: 'in_flight', type: 'in_flight', position: { x: 0, y: 8 }, size: { w: 12, h: 3 }, visible: true },
  ],
};

const WORKSPACES_FIXTURE = [{
  id: 'ws-dash', name: 'Home', icon: 'home', sortOrder: 0,
  layoutJson: { type: 'panel', tool: 'dashboard', config: {} },
  isDefault: true, createdAt: '', updatedAt: '',
}];

const json = (data) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(data) });

async function setupRoutes(page) {
  // Playwright matches routes in reverse registration order:
  // register the static-file catch-all FIRST, API fixtures after.
  if (DIST) {
    await page.route('**/*', route => {
      const url = new URL(route.request().url());
      // Only serve the app origin; let fonts/CDNs hit the network.
      if (!url.origin.includes('app.local')) return route.continue();
      let p = url.pathname;
      if (p.startsWith('/ui/')) p = p.slice('/ui'.length); // non-Tauri build base
      if (p === '/' || p === '') p = '/index.html';
      const file = path.join(DIST, p);
      if (fs.existsSync(file) && fs.statSync(file).isFile()) {
        return route.fulfill({
          status: 200,
          contentType: MIME[path.extname(file)] || 'application/octet-stream',
          body: fs.readFileSync(file),
        });
      }
      return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
    });
  }
  await page.route('**/permagent/**', r => r.fulfill({ status: 404, contentType: 'application/json', body: '{}' }));
  await page.route('**/api/**', r => r.fulfill({ status: 404, contentType: 'application/json', body: '{}' }));
  await page.route('**/config', r => r.fulfill(json({ config: { wizard_complete: true } })));
  await page.route('**/api/workspaces', r => r.fulfill(json(WORKSPACES_FIXTURE)));
  await page.route('**/api/workspaces/active', r => r.fulfill(json({ workspaceId: 'ws-dash' })));
  await page.route('**/api/dashboard', r => r.fulfill(json(DASHBOARD_FIXTURE)));
  await page.route('**/api/dashboard/layout', r => r.fulfill(json(LAYOUT_FIXTURE)));
}

async function bootToDashboard(page, url) {
  await page.goto(url);
  // Splash runs ~5.4s before the app phase.
  await page.waitForSelector('[data-testid="decisions-card"]', { timeout: 30000 });
  await page.waitForTimeout(800); // let the mock's 300ms latency + paint settle
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  for (const theme of THEMES) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    await context.addInitScript(t => {
      localStorage.setItem('permagent-theme', t);
      localStorage.setItem('permagent-reduce-motion', 'true'); // determinism
    }, theme);
    const page = await context.newPage();
    await setupRoutes(page);

    // ── (a) home card on the dashboard ──
    await bootToDashboard(page, BASE + '/');
    await page.screenshot({ path: `${OUT}/${theme}-a-home-card.png` });

    // ── (b) inbox overlay: confirm step + unblock + Tier-1 group ──
    await page.click('[data-testid="decisions-card"]');
    await page.waitForSelector('text=Decision inbox');
    await page.waitForSelector('[data-testid="decision-dec-001"]');
    // Tier-2 inline confirm on the spend-cap item (not submitted)
    await page.locator('[data-testid="decision-dec-002-conflict"] button', { hasText: 'Approve' }).first().click();
    await page.waitForSelector('text=Confirm approve');
    await page.screenshot({ path: `${OUT}/${theme}-b1-inbox-confirm-unblock.png` });
    // Expand the Tier-1 group and scroll it into view
    await page.locator('button', { hasText: 'Henry handled 7 routine items overnight' }).click();
    await page.waitForSelector('text=audit →');
    await page.locator('[data-testid="inbox-body"]').evaluate(el => { el.scrollTop = el.scrollHeight; });
    await page.waitForTimeout(200);
    await page.screenshot({ path: `${OUT}/${theme}-b2-tier1-group.png` });

    // ── (c) expanded evidence with Show details open ──
    await page.locator('[data-testid="inbox-body"]').evaluate(el => { el.scrollTop = 0; });
    await page.locator('[data-testid="decision-dec-001"] button', { hasText: 'Evidence' }).click();
    await page.waitForSelector('text=Cost so far');
    await page.locator('[data-testid="decision-dec-001"] button', { hasText: 'Show details' }).click();
    await page.waitForSelector('text=VERIFIER');
    await page.waitForTimeout(200);
    await page.screenshot({ path: `${OUT}/${theme}-c-evidence-details.png` });

    // ── (d) empty state (card + overlay copy share the same source) ──
    await bootToDashboard(page, BASE + '/?decisions=empty');
    await page.waitForSelector('text=No decisions needed.');
    await page.screenshot({ path: `${OUT}/${theme}-d-empty.png` });

    await context.close();
    console.log(`captured: ${theme}`);
  }
  await browser.close();
  console.log('done →', OUT);
})().catch(e => { console.error(e); process.exit(1); });
