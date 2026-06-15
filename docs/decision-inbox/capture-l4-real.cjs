/**
 * Decision Inbox L4 — REAL-DAEMON evidence capture.
 *
 * Driven by real-daemon-demo.sh. The app dist (built with
 * VITE_DAEMON_URL=http://127.0.0.1:$DEMO_PORT, real client default) is served
 * via route interception; /api requests pass through to the LIVE daemon with
 * the Bearer token injected at the network layer (outside Tauri there is no
 * IPC to supply it — in the app it comes from loadDaemonToken()).
 *
 * Captures: (a) home card with the real count, (b) inbox list from
 * GET /api/decisions, (c) approve end-to-end (row leaves the list; daemon row
 * flips to answered), (d) the 409 conflict state on a pre-answered item.
 */

const fs = require('fs');
const path = require('path');
const { chromium } = require(path.join(
  __dirname, '..', '..', 'ui', 'command-center', 'node_modules', 'playwright'
));

const PORT = process.env.DEMO_PORT || '3899';
const TOKEN = process.env.DEMO_TOKEN || '';
const DIST = path.join(__dirname, '..', '..', 'ui', 'command-center', 'dist-real');
const OUT = process.env.CAPTURE_OUT || path.join(__dirname, 'screenshots-l4-real');

const MIME = {
  '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon',
  '.json': 'application/json', '.woff2': 'font/woff2', '.woff': 'font/woff',
  '.webmanifest': 'application/manifest+json', '.wasm': 'application/wasm',
};

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  await context.addInitScript(() => {
    localStorage.setItem('permagent-theme', 'dark');
    localStorage.setItem('permagent-reduce-motion', 'true');
  });
  const page = await context.newPage();

  // Static dist for the app origin; real daemon for everything API-shaped.
  await page.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.port === PORT) {
      // Live daemon: inject the Bearer token (loadDaemonToken() is Tauri-only).
      return route.continue({
        headers: { ...route.request().headers(), authorization: `Bearer ${TOKEN}` },
      });
    }
    if (!url.origin.includes('app.local')) return route.continue();
    let p = url.pathname;
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

  await page.goto('http://app.local/');
  await page.waitForSelector('[data-testid="decisions-card"]', { timeout: 30000 });
  await page.waitForTimeout(1200);

  // (a) home card with the real pending count
  await page.screenshot({ path: `${OUT}/real-a-home-card.png` });

  // (b) inbox list straight from GET /api/decisions
  await page.click('[data-testid="decisions-card"]');
  await page.waitForSelector('[data-testid="decision-dec-demo-1"]');
  await page.screenshot({ path: `${OUT}/real-b-inbox-list.png` });

  // (c) approve end-to-end: confirm step → POST answer → row leaves the list
  await page.locator('[data-testid="decision-dec-demo-1"] button', { hasText: 'Approve' }).first().click();
  await page.waitForSelector('text=Confirm approve');
  await page.screenshot({ path: `${OUT}/real-c1-confirm.png` });
  await page.locator('button', { hasText: 'Confirm approve' }).click();
  await page.waitForSelector('[data-testid="decision-dec-demo-1"]', { state: 'detached', timeout: 15000 });
  await page.screenshot({ path: `${OUT}/real-c2-answered.png` });

  // (d) 409: dec-demo-409 was pre-answered out-of-band by the demo script
  const conflictItem = page.locator('[data-testid="decision-dec-demo-409"]');
  if (await conflictItem.count()) {
    await conflictItem.locator('button', { hasText: 'Approve' }).first().click();
    await page.locator('button', { hasText: 'Confirm approve' }).click();
    await page.waitForSelector('text=Someone already answered this');
    await page.screenshot({ path: `${OUT}/real-d-409-conflict.png` });
  } else {
    console.warn('409 fixture already refreshed out of the list — rerun with a fresh seed');
  }

  await browser.close();
  console.log('done →', OUT);
})().catch(e => { console.error(e); process.exit(1); });
