// W3 browser evidence: real pointer/tour interaction, reduced-motion fallback,
// and the hidden-layout render gate. Uses the same standalone World surface and
// Chrome launch contract as world-blender-evidence.mjs. No user credentials.
import { chromium } from 'playwright';

const BASE = 'http://127.0.0.1:5173/ui/worldcensus.html?perf=1&dpr=1.5';
const browser = await chromium.launch({ channel: 'chrome', headless: true });

function distance(a, b) {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

async function ready(page) {
  await page.goto(BASE);
  await page.waitForFunction(() => window.__worldDebug?.cameraSnapshot && window.__worldPerf, undefined, { timeout: 30000 });
  await page.waitForFunction(() => {
    const assets = window.__worldDebug.assetStats?.();
    return assets?.vaults === 1 && assets.authoredAgents === 12;
  }, undefined, { timeout: 30000 });
}

try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  await page.addInitScript(() => localStorage.setItem('permagent-theme', 'system'));
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  await ready(page);

  // Actual pointer drag through the canvas, not a synthetic state setter.
  const canvas = page.locator('canvas').first();
  const box = await canvas.boundingBox();
  if (!box) throw new Error('World canvas has no layout box');
  const beforeDrag = await page.evaluate(() => window.__worldDebug.cameraSnapshot());
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 180, cy + 42, { steps: 8 });
  await page.mouse.up();
  await page.waitForTimeout(250);
  const afterDrag = await page.evaluate(() => window.__worldDebug.cameraSnapshot());
  if (distance(beforeDrag.position, afterDrag.position) < 0.2) {
    throw new Error('Orbit pointer drag did not move the World camera');
  }

  // Tour is started by its public keyboard affordance and owns the camera for
  // its first beat. This proves event-driven motion through the mounted scene.
  await page.keyboard.press('t');
  await page.waitForFunction(() => window.__worldDebug.tourActive?.() === true);
  await page.waitForTimeout(1200);
  const afterTour = await page.evaluate(() => window.__worldDebug.cameraSnapshot());
  if (distance(afterDrag.position, afterTour.position) < 0.05) {
    throw new Error('Tour key did not move the World camera');
  }
  await page.keyboard.press('Escape');

  // ResizeObserver visibility gate: removing layout pauses the manual frame
  // loop, so the perf probe cannot accumulate a fresh measurement while hidden.
  await page.waitForTimeout(300);
  const hiddenResult = await page.evaluate(() => {
    const root = document.getElementById('root');
    if (!root) throw new Error('World root missing');
    root.style.display = 'none';
    return { style: root.style.display };
  });
  // ResizeObserver/React commit asynchronously. Start the measurement only
  // after the actual visibility gate acknowledges the hidden layout, not
  // during its final pre-hide frame/perf-window flush.
  await page.waitForFunction(() => document.querySelector('[data-world-active="false"]'));
  await page.evaluate(() => { window.__worldPerfLog = []; });
  await page.waitForTimeout(2200);
  const hiddenSamples = await page.evaluate(() => window.__worldPerfLog?.length ?? 0);
  await page.evaluate(() => {
    const root = document.getElementById('root');
    if (root) root.style.display = '';
  });
  await page.waitForTimeout(2300);
  const resumedSamples = await page.evaluate(() => window.__worldPerfLog?.length ?? 0);
  if (hiddenSamples !== 0) throw new Error(`Hidden World rendered ${hiddenSamples} perf windows`);
  if (resumedSamples < 1) throw new Error('World did not resume perf sampling after becoming visible');

  // A second page gets a fresh hook snapshot under the media preference.
  const reduced = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  await reduced.addInitScript(() => localStorage.setItem('permagent-theme', 'system'));
  await reduced.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  const reducedFailures = [];
  reduced.on('pageerror', error => reducedFailures.push(error.message));
  await ready(reduced);
  const mediaReduced = await reduced.evaluate(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  if (!mediaReduced) throw new Error('Browser did not expose reduced-motion preference');
  await reduced.keyboard.press('t');
  await reduced.waitForFunction(() => window.__worldDebug.tourActive?.() === true);
  await reduced.waitForTimeout(300);
  const reducedStart = await reduced.evaluate(() => window.__worldDebug.cameraSnapshot());
  await reduced.waitForTimeout(1200);
  const reducedEnd = await reduced.evaluate(() => window.__worldDebug.cameraSnapshot());
  await reduced.screenshot({ path: '/private/tmp/permagent-world-w3-reduced-motion.png' });
  if (distance(reducedStart.position, reducedEnd.position) > 0.05) {
    throw new Error('Reduced-motion tour drifted instead of holding its establishing shot');
  }
  if (reducedFailures.length) throw new Error(JSON.stringify({ reducedFailures }));
  await reduced.close();

  if (failures.length) throw new Error(JSON.stringify({ failures }));
  console.log(JSON.stringify({
    pointer: { before: beforeDrag.position, after: afterDrag.position, moved: distance(beforeDrag.position, afterDrag.position) },
    tour: { after: afterTour.position, moved: distance(afterDrag.position, afterTour.position) },
    hidden: { ...hiddenResult, perfWindows: hiddenSamples, resumedPerfWindows: resumedSamples },
    reducedMotion: { mediaReduced, start: reducedStart.position, end: reducedEnd.position },
    screenshot: '/private/tmp/permagent-world-w3-reduced-motion.png',
    failures: [...failures, ...reducedFailures],
  }));
} finally {
  await browser.close();
}
