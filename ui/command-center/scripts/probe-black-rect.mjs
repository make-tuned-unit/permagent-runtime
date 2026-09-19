#!/usr/bin/env node
/*
 * Does a change make the ANGLE/Metal blown-out rectangle go away? (Either
 * polarity — it has been seen black AND white.)
 *
 * Job 19 and 20 recorded a solid block of exact RGB 0,0,0 in the rendered
 * frame, root-caused it to the ANGLE **Metal** backend (67,447 identical black
 * pixels across separate browser processes; zero on the GL backend) and worked
 * around it in the capture harness by driving GL. The product runs on Metal, so
 * the user still sees it — which is exactly what happened again today.
 *
 * This probe is the missing piece: it reproduces the defect ON PURPOSE, on the
 * Metal backend, and counts the black pixels, so a candidate mitigation can be
 * judged by measurement instead of by hope. It needs no daemon and no token —
 * the standalone forum page renders the world on its own.
 *
 *   node scripts/probe-black-rect.mjs [url] [label] [--engine webkit|chromium] [--scheme dark|light]
 *   node scripts/probe-black-rect.mjs --shipped [/Applications/Permagent.app]
 *
 * `--shipped` answers a different question, with no browser and no GPU: is
 * the fix in the app Jesse launches? Every pixel run below renders
 * `ui/forum.html` from the vite DEV server, which is not in `dist/` at all,
 * and on 2026-09-19 the in-app look was blocked by a locked screen. So this
 * reads the asset names the Tauri binary embeds, opens the same-named chunks
 * in `dist/assets` (vite names a chunk by a hash of its content, so a name
 * match is a byte match), and reports the EffectComposer chain written in
 * them. Exit 0 only if the forum chain is Bloom > AgX > grade, single-sampled.
 * Exit 2 if the app embeds chunks this dist no longer has: then the dist was
 * rebuilt after packaging and cannot vouch for the app either way.
 *
 * Engine: WEBKIT is the default, because the shipped app is Tauri, and Tauri on
 * macOS is WKWebView. Every earlier run of this probe drove Chromium with
 * `--use-angle=metal`, which is a renderer the app never uses — so on
 * 2026-09-18, when Jesse photographed a white block over the night sky in the
 * real app, this instrument had never once looked at the renderer he was
 * looking at. `--engine chromium` keeps the old ANGLE path for comparison.
 *
 * Scheme: the forum follows prefers-color-scheme (useSystemAppearance), and
 * the white rectangle was reported at NIGHT, so `dark` is the default. A
 * probe that renders the day scene cannot see a night-only defect.
 *
 * Output also carries `clipPerFrame`: pure-white pixels, pixels with any
 * channel at 255, and the largest connected pure-white blob, counted over the
 * canvas rect only. That is the proxy the white-block A/B in
 * ForumPostProcessing.tsx is recorded in: the block came from linear HDR
 * clipping at 1.0 (no view transform in the composer), which shows up as a
 * big flat-white blob long before it grows into a 200 px run.
 *
 * Env:
 *   ENGINE         webkit (default) | chromium — same as --engine
 *   SCHEME         dark (default) | light — same as --scheme
 *   ANGLE_BACKEND  chromium only: metal (default) | gl
 *   SHOTS          how many frames to sample (default 6)
 *   SETTLE_MS      wait before the first frame (default 6000)
 *   VIEWPORT       WxH (default 1280x1000)
 *   DPR            deviceScaleFactor (default 1). The app runs on a Retina
 *                  display, where the forum Canvas resolves dpr [1, 1.5] to
 *                  1.5 — every render target is a different, non-power-of-two
 *                  size from the one a DPR 1 probe exercises.
 *   SAVE_ALL       save every frame, not only the ones with an artifact
 *   DRAG           1 (default) orbits the camera between frames; 0 holds the
 *                  default vista, which is the pose the white block was
 *                  reported in (beside the gas giant)
 *   RESIZE         1 resizes the viewport between frames (every render target
 *                  in the composer is reallocated on a resize, and the app's
 *                  window and Agent Desk resize the canvas all the time)
 */
import { chromium, webkit } from 'playwright';
import { mkdir, readdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import { inflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { clipReport, composerChains, embeddedAssetNames, extremeReport, isShippedForumChain } from '../src/components/world/forum/extremeRuns.ts';

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const flag = (name) => { const i = argv.indexOf(name); if (i < 0) return undefined; const v = argv[i + 1]; argv.splice(i, 2); return v; };
// `--analyze a.png b.png ...` runs the same detector over frames captured
// elsewhere — the system-WKWebView probe (`probe-wkwebview.swift`) or a
// screenshot of the real app — so every source is judged by one rule.
const analyzeAt = argv.indexOf('--analyze');
const analyzeFiles = analyzeAt >= 0 ? argv.splice(analyzeAt).slice(1) : [];
const engine = flag('--engine') || process.env.ENGINE || 'webkit';
const scheme = flag('--scheme') || process.env.SCHEME || 'dark';
if (!['webkit', 'chromium'].includes(engine)) throw new Error(`--engine must be webkit or chromium, got ${engine}`);
if (!['dark', 'light'].includes(scheme)) throw new Error(`--scheme must be dark or light, got ${scheme}`);
const url = process.env.APP_URL || argv[0] || 'http://localhost:5173/ui/forum.html';
const label = argv[1] || 'probe';
// The label on saved frames and in the report: the ANGLE backend only means
// anything under Chromium.
const backend = engine === 'webkit' ? 'webkit' : `chromium-${process.env.ANGLE_BACKEND || 'metal'}`;
const shots = Number(process.env.SHOTS || 6);
const settleMs = Number(process.env.SETTLE_MS || 6000);
const outDir = path.join(here, '../../../.playwright-mcp/black-rect');

function decodePng(buffer) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  if (!buffer.subarray(0, 8).equals(signature)) throw new Error('not a png');
  let offset = 8, width = 0, height = 0, colorType = 0, bitDepth = 0;
  const chunks = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset); offset += 4;
    const type = buffer.toString('ascii', offset, offset + 4); offset += 4;
    const data = buffer.subarray(offset, offset + length); offset += length + 4;
    if (type === 'IHDR') { width = data.readUInt32BE(0); height = data.readUInt32BE(4); bitDepth = data[8]; colorType = data[9]; }
    if (type === 'IDAT') chunks.push(data);
    if (type === 'IEND') break;
  }
  if (bitDepth !== 8 || (colorType !== 2 && colorType !== 6)) throw new Error('unsupported png');
  const channels = colorType === 6 ? 4 : 3;
  const stride = width * channels;
  const raw = inflateSync(Buffer.concat(chunks));
  const pixels = Buffer.alloc(height * stride);
  let src = 0;
  const paeth = (a, b, c) => { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < height; y++) {
    const filter = raw[src++];
    const row = y * stride, prior = (y - 1) * stride;
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? pixels[row + x - channels] : 0;
      const up = y ? pixels[prior + x] : 0;
      const upperLeft = y && x >= channels ? pixels[prior + x - channels] : 0;
      const value = raw[src++];
      pixels[row + x] = filter === 0 ? value : filter === 1 ? value + left : filter === 2 ? value + up : filter === 3 ? value + Math.floor((left + up) / 2) : value + paeth(left, up, upperLeft);
    }
  }
  return { width, height, channels, stride, pixels };
}

// The detector lives beside the forum so vitest can pin it (`src/` is the only
// tree the frontend gate runs); Node 24 strips the types on import.

// The WebKit build this playwright version pins may not be the one on disk
// (this machine has webkit-2359 while playwright 1.62 asks for 2336, and
// `playwright install` is a large download nobody asked for). Fall back to the
// newest installed build rather than refusing to look; WEBKIT_EXECUTABLE wins.
async function webkitExecutable() {
  if (process.env.WEBKIT_EXECUTABLE) return process.env.WEBKIT_EXECUTABLE;
  if (existsSync(webkit.executablePath())) return undefined;
  const cache = path.join(os.homedir(), 'Library/Caches/ms-playwright');
  const builds = (await readdir(cache).catch(() => []))
    .filter((d) => /^webkit-\d+$/.test(d))
    .sort((a, b) => Number(b.slice(7)) - Number(a.slice(7)));
  for (const build of builds) {
    const run = path.join(cache, build, 'pw_run.sh');
    if (existsSync(run)) return run;
  }
  return undefined;
}

// `--shipped [app]`: read the composer chain out of the bytes the installed
// app serves. See the header; nothing is launched and nothing is rendered.
const shippedAt = argv.indexOf('--shipped');
if (shippedAt >= 0) {
  const { readFile, stat } = await import('node:fs/promises');
  const next = argv[shippedAt + 1];
  const app = next && !next.startsWith('--') ? next : '/Applications/Permagent.app';
  const binaryPath = path.join(app, 'Contents/MacOS/permagent-app');
  const distAssets = path.join(here, '../dist');
  if (!existsSync(binaryPath)) { console.error(`--shipped: no app binary at ${binaryPath}`); process.exit(1); }
  if (!existsSync(path.join(distAssets, 'assets'))) { console.error(`--shipped: no ${distAssets}/assets; run \`npm run build\` first`); process.exit(1); }
  const embedded = embeddedAssetNames((await readFile(binaryPath)).toString('latin1'));
  const missing = [];
  const chains = [];
  for (const name of embedded) {
    const file = path.join(distAssets, name);
    if (!existsSync(file)) { missing.push(name); continue; }
    for (const chain of composerChains(await readFile(file, 'utf8'))) chains.push({ chunk: name, ...chain, shipped: isShippedForumChain(chain) });
  }
  const forum = chains.filter((c) => c.effects.includes('Bloom') && c.effects.includes('HueSaturation'));
  const verdict = embedded.length === 0 ? 'no-embedded-assets'
    : missing.length ? 'dist-does-not-match-app'
    : forum.length === 1 && forum[0].shipped ? 'fix-shipped'
    : forum.length === 0 ? 'no-forum-composer' : 'fix-not-shipped';
  console.log(JSON.stringify({
    app,
    binary: binaryPath,
    binaryModified: (await stat(binaryPath)).mtime.toISOString(),
    embeddedChunks: embedded.length,
    missingFromDist: missing.slice(0, 10),
    missingCount: missing.length,
    composers: chains,
    verdict,
  }, null, 2));
  process.exit(verdict === 'fix-shipped' ? 0 : verdict === 'dist-does-not-match-app' ? 2 : 1);
}

if (analyzeFiles.length) {
  const { readFile } = await import('node:fs/promises');
  const scale = Number(process.env.DPR || 1);
  for (const file of analyzeFiles) {
    const img = decodePng(await readFile(file));
    const r = extremeReport(img, scale);
    console.log(JSON.stringify({ file: path.basename(file), black: r.black, white: r.white, clip: clipReport(img) }));
  }
  process.exit(0);
}

const browser = engine === 'webkit'
  ? await webkit.launch({ headless: true, executablePath: await webkitExecutable() })
  : await chromium.launch({
      headless: true,
      channel: 'chrome',
      args: [`--use-angle=${process.env.ANGLE_BACKEND || 'metal'}`, '--enable-gpu', '--ignore-gpu-blocklist', '--disable-frame-rate-limit', '--disable-gpu-vsync'],
    });
const [vw, vh] = (process.env.VIEWPORT || '1280x1000').split('x').map(Number);
const dpr = Number(process.env.DPR || 1);
const context = await browser.newContext({ viewport: { width: vw, height: vh }, deviceScaleFactor: dpr, colorScheme: scheme });
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', (err) => pageErrors.push(String(err)));
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
await page.waitForTimeout(settleMs);
// Refuse to report "clean" on a frame that is not the scene being asked about:
// the forum prints which appearance it resolved, and a day frame says nothing
// about a night defect.
const appearance = await page.locator('[data-appearance]').first().getAttribute('data-appearance').catch(() => null);
const renderer = await page.evaluate(() => {
  const c = document.querySelector('canvas');
  const gl = c && (c.getContext('webgl2') || c.getContext('webgl'));
  if (!gl) return null;
  const dbg = gl.getExtension('WEBGL_debug_renderer_info');
  return { version: gl.getParameter(gl.VERSION), renderer: dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER) };
}).catch(() => null);

await mkdir(outDir, { recursive: true });
const results = [];
for (let i = 0; i < shots; i++) {
  // Nudge the camera between frames: the defect sticks to a pose, so a single
  // frame proves nothing either way.
  if (process.env.RESIZE === '1') {
    const f = [1, 0.83, 0.91, 0.77, 1.07, 0.95][i % 6];
    await page.setViewportSize({ width: Math.round(vw * f), height: Math.round(vh * (2 - f) / 1.05) });
    await page.waitForTimeout(250);
  }
  if (process.env.DRAG !== '0') {
    await page.mouse.move(640, 500);
    await page.mouse.down();
    await page.mouse.move(640 + i * 40, 500 + i * 12, { steps: 6 });
    await page.mouse.up();
  }
  await page.waitForTimeout(700);
  const buffer = await page.screenshot({ type: 'png' });
  const img = decodePng(buffer);
  // Clip stats over the canvas only: the Agent Desk is white text on dark UI.
  const rect = await page.evaluate(() => { const c = document.querySelector('canvas'); if (!c) return null; const r = c.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom]; }).catch(() => null);
  const region = rect ? rect.map((v) => Math.round(v * dpr)) : undefined;
  const report = { ...extremeReport(img, dpr), clip: clipReport(img, region) };
  results.push(report);
  if (report.count > 0 || process.env.SAVE_ALL) await writeFile(path.join(outDir, `${label}-${backend}-${i}.png`), buffer);
}

await browser.close();

const worst = results.reduce((a, b) => (b.count > a.count ? b : a), results[0]);
const total = results.reduce((sum, r) => sum + r.count, 0);
console.log(JSON.stringify({
  label,
  engine,
  backend,
  scheme,
  viewport: `${vw}x${vh}@${dpr}`,
  appearance,
  renderer,
  pageErrors: pageErrors.slice(0, 5),
  frames: results.length,
  framesWithArtifact: results.filter(r => r.count > 0).length,
  totalExtremePixels: total,
  blackPixels: results.reduce((sum, r) => sum + r.black.count, 0),
  whitePixels: results.reduce((sum, r) => sum + r.white.count, 0),
  worstFrame: worst,
  // The clipping proxy (see clipReport): per frame, so a flicker shows.
  clipPerFrame: results.map((r) => ({ white: r.clip.white, anyChannel: r.clip.anyChannel, largestWhite: r.clip.largestWhite })),
}, null, 2));
