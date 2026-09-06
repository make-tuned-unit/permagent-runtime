// Runs only the existing standalone World dev surface. No user credentials.
import { chromium } from 'playwright';
const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  await page.addInitScript(() => localStorage.setItem('permagent-theme', 'system'));
  await page.emulateMedia({ colorScheme: 'light' });
  const failures = [];
  const warnings = [];
  page.on('pageerror', error => failures.push(error.message));
  page.on('console', message => {
    if (message.type() === 'warning' && message.text().includes('[world]')) warnings.push(message.text());
  });
  await page.goto('http://127.0.0.1:5173/ui/worldcensus.html?perf=1&dpr=1.5');
  await page.waitForFunction(() => window.__worldDebug && window.__worldPerf, undefined, { timeout: 30000 });
  await page.waitForFunction(() => {
    const assets = window.__worldDebug.assetStats?.();
    return assets?.vaults === 1 && assets.authoredAgents === 12;
  }, undefined, { timeout: 30000 });
  await page.evaluate(() => window.__worldDebug.setCam([43, 28, 46], [0, 16, 0]));
  await page.waitForTimeout(4000);
  await page.screenshot({ path: '/private/tmp/permagent-blender-world.png' });
  const day = await page.evaluate(() => window.__worldTime.snapshot());
  const samples = [];
  for (let i = 0; i < 5; i++) {
    await page.waitForTimeout(1000);
    samples.push(await page.evaluate(() => window.__worldPerf));
  }
  const assets = await page.evaluate(() => window.__worldDebug.assetStats());
  const motion = await page.evaluate(async () => {
    const start = performance.now();
    await new Promise(resolve => {
      const tick = now => {
        const t = (now-start)/1000;
        window.__worldDebug.setCam([Math.cos(.8+t*.10)*63,28,Math.sin(.8+t*.10)*63],[0,16,0]);
        if (t < 12) requestAnimationFrame(tick); else resolve();
      };
      requestAnimationFrame(tick);
    });
    return window.__worldPerfLog?.slice(-10);
  });
  await page.evaluate(() => window.__worldDebug.setCam([3, 2.8, 5], [0, 1.3, 0]));
  await page.waitForTimeout(1000);
  await page.screenshot({ path: '/private/tmp/permagent-blender-character.png' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.waitForTimeout(4000);
  const night = await page.evaluate(() => window.__worldTime.snapshot());
  await page.evaluate(() => window.__worldDebug.setCam([43,28,46],[0,16,0]));
  await page.screenshot({ path: '/private/tmp/permagent-blender-night.png' });
  if (!['day','dawn'].includes(day.phase) || !['night','dusk'].includes(night.phase))
    throw new Error('System appearance did not reach World atmosphere');
  if (failures.length || warnings.length) throw new Error(JSON.stringify({failures,warnings}));
  console.log(JSON.stringify({ samples, motion, assets, day, night, failures, warnings }));
} finally { await browser.close(); }
