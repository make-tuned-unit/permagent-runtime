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
 *   node scripts/probe-black-rect.mjs --app-state [url] [label]
 *   node scripts/probe-black-rect.mjs --app-window [/Applications/Permagent.app]
 *
 * `--app-window` is the last hop every other mode stops short of: it captures
 * the RUNNING Permagent.app window itself (`screencapture -l`, read-only: no
 * click, no key, no focus change) and judges it with the same detector. It
 * refuses on a locked screen (exit 4) rather than judge the lock screen, and
 * says so if the capture fails (exit 5: Screen Recording permission). It
 * refuses unless macOS is in Dark appearance (exit 6, `not-night`): the World
 * takes night from that setting, and the user's white block was at night.
 * Exit 2 if a solid block is in the window, 0 if not. Which VIEW is open is
 * read off the capture by on-device text recognition (macOS Vision, no network)
 * and must show the World's own labels (worldFromOcr), else exit 7
 * (`world-not-open`): until 2026-09-19 this printed "clean" for any view and
 * left "check the capture is the World" to an operator, which nobody could do
 * on a locked screen. Exit 3 if the app window belongs to a process launched
 * before the binary was written (it shows the previous build).
 *
 *   node scripts/probe-black-rect.mjs --app-window --wait 3600
 *
 * arms it: it polls (POLL_MS, default 15 s) through a locked screen, light
 * mode, or another view, and judges the first capture that is unlocked, at
 * night, and of the World. Captures that are not the World are deleted.
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
 * rebuilt after packaging and cannot vouch for the app either way. Exit 3
 * (`fix-shipped-but-running-app-predates-it`) if the fix is on disk but a
 * running Permagent.app was launched before the binary was written: it is
 * still serving the previous build, and only a relaunch shows the fix.
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
 *   PLANT          black | white: a positive control. Lays a fixed 340x251
 *                  CSS px block of that exact colour at [437,0], the job-19
 *                  rectangle, over the live scene. A run with PLANT must
 *                  report framesWithArtifact == frames. Why: on 2026-09-19 the
 *                  detector judged fill over one box around every run, and in
 *                  the user's layout the crushed tower facade diluted a real
 *                  block to solid:false, so a clean verdict could have meant a
 *                  blind instrument. PLANT proves it sees the block in the
 *                  same frame as that texture before a clean verdict counts.
 */
import { chromium, webkit } from 'playwright';
import { mkdir, readdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import { inflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { clipReport, composerChains, embeddedAssetNames, extremeReport, forumStorageFromRows, isShippedForumChain, pickAppWindow, runningAppProcesses, screenIsLocked, worldAppearanceFromDefaults, worldFromOcr } from '../src/components/world/forum/extremeRuns.ts';

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
// `--app-state`: render the World in the layout the INSTALLED app has
// (Agent Desk, place rail, dismissed cards), read out of its WebKit
// localStorage. Every earlier run rendered empty storage — desk and rail open
// — while the user's app has both closed, i.e. a full-width canvas, a
// different camera aspect and a different gas-giant framing. STORAGE='{json}'
// seeds explicit keys instead.
const appWindowAt = argv.indexOf('--app-window');
const appWindowApp = appWindowAt >= 0 ? (argv[appWindowAt + 1] && !argv[appWindowAt + 1].startsWith('--') ? argv.splice(appWindowAt, 2)[1] : (argv.splice(appWindowAt, 1), '/Applications/Permagent.app')) : null;
const appStateAt = argv.indexOf('--app-state');
if (appStateAt >= 0) argv.splice(appStateAt, 1);
function appStorage(bundleId = 'ai.permagent.desktop') {
  // WebKit keeps one store per origin under WebsiteData/Default/<salt>/<salt>;
  // the app's is the newest one holding World keys.
  const root = path.join(os.homedir(), 'Library/WebKit', bundleId, 'WebsiteData');
  let dbs = [];
  try { dbs = execFileSync('find', [root, '-name', 'localstorage.sqlite3'], { encoding: 'utf8' }).split('\n').filter(Boolean); } catch { return {}; }
  let best = {}, bestTime = -1;
  for (const db of dbs) {
    let rows = '';
    // -readonly and mode=ro: this is the user's live app profile.
    try { rows = execFileSync('sqlite3', ['-readonly', `file:${db}?mode=ro`, 'select key, hex(value) from ItemTable'], { encoding: 'utf8' }); } catch { continue; }
    const found = forumStorageFromRows(rows);
    const mtime = Number(execFileSync('stat', ['-f', '%m', db], { encoding: 'utf8' }));
    if (Object.keys(found).length && mtime > bestTime) { best = found; bestTime = mtime; }
  }
  return best;
}
const storage = appStateAt >= 0 ? appStorage() : process.env.STORAGE ? JSON.parse(process.env.STORAGE) : {};
if (appStateAt >= 0 && !Object.keys(storage).length) { console.error('--app-state: no World keys in the app\'s WebKit storage'); process.exit(1); }
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
  // The binary on disk is not what is on screen: a running app serves the
  // assets of the binary it was LAUNCHED from. See runningAppProcesses.
  const binaryMtimeMs = (await stat(binaryPath)).mtimeMs;
  let running = [];
  try {
    running = runningAppProcesses(execFileSync('ps', ['-axo', 'pid=,etime=,comm='], { encoding: 'utf8' }), binaryPath, binaryMtimeMs, Date.now());
  } catch { /* no ps: report the disk verdict alone */ }
  const verdict = embedded.length === 0 ? 'no-embedded-assets'
    : missing.length ? 'dist-does-not-match-app'
    : !(forum.length === 1 && forum[0].shipped) ? (forum.length === 0 ? 'no-forum-composer' : 'fix-not-shipped')
    : running.some((r) => r.stale) ? 'fix-shipped-but-running-app-predates-it'
    : 'fix-shipped';
  console.log(JSON.stringify({
    app,
    binary: binaryPath,
    binaryModified: (await stat(binaryPath)).mtime.toISOString(),
    embeddedChunks: embedded.length,
    missingFromDist: missing.slice(0, 10),
    missingCount: missing.length,
    composers: chains,
    running,
    verdict,
  }, null, 2));
  process.exit(verdict === 'fix-shipped' ? 0 : verdict === 'dist-does-not-match-app' ? 2 : verdict === 'fix-shipped-but-running-app-predates-it' ? 3 : 1);
}

// On-device text recognition (macOS Vision; no network, no model call), one
// line per observation. It is how a capture is checked to BE the World (see
// worldFromOcr) rather than trusted to be. [] if Vision cannot run.
const visionReader = `import Foundation
import Vision
let req = VNRecognizeTextRequest()
req.recognitionLevel = .accurate
req.usesLanguageCorrection = false
try VNImageRequestHandler(url: URL(fileURLWithPath: CommandLine.arguments[1]), options: [:]).perform([req])
for o in (req.results ?? []) { if let t = o.topCandidates(1).first { print(t.string) } }
`;
function readText(file) {
  try { return execFileSync('swift', ['-', file], { input: visionReader, encoding: 'utf8', stdio: ['pipe', 'pipe', 'ignore'] }).split('\n').filter(Boolean); } catch { return []; }
}

// `--app-window [app]`: capture and judge the running app's own window (see
// the header). Nothing is launched, clicked or focused.
if (appWindowApp) {
  const { readFile, rm, stat } = await import('node:fs/promises');
  const binaryPath = path.join(appWindowApp, 'Contents/MacOS/permagent-app');
  const done = (verdict, extra, code) => { console.log(JSON.stringify({ mode: 'app-window', app: appWindowApp, ...extra, verdict }, null, 2)); process.exit(code); };
  // `--wait <seconds>` (or WAIT=): keep polling for that long instead of
  // refusing at once on a locked screen or a view that is not the World. Why:
  // on 2026-09-19 the node was un-confirmable twice over, both times on a
  // locked screen, and a one-shot check can only ever be run at the wrong
  // moment. Armed with --wait, the check fires on the first capture that is
  // unlocked, at night, and showing the World, with no one watching. Every
  // wait is a sleep plus the same read-only reads: nothing is clicked.
  const waitAt = argv.indexOf('--wait');
  const waitSec = waitAt >= 0 ? Number(argv.splice(waitAt, 2)[1]) : Number(process.env.WAIT || 0);
  if (!Number.isFinite(waitSec) || waitSec < 0) done('bad-wait', { wait: waitSec }, 1);
  const deadline = Date.now() + waitSec * 1000;
  const pollMs = Number(process.env.POLL_MS || 15000);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // Poll only while there is time left; otherwise report the refusal.
  const retryOr = async (verdict, extra, code) => { if (Date.now() + pollMs > deadline) done(verdict, { ...extra, waited: waitSec }, code); await sleep(pollMs); };
  const locked = () => { try { return screenIsLocked(execFileSync('ioreg', ['-n', 'Root', '-d1', '-a'], { encoding: 'utf8' })); } catch { return false; /* no ioreg: let the capture speak */ } };
  // Night is the condition the user reported the white block in, and the
  // World takes night from the system appearance (useSystemAppearance.ts). A
  // light-mode capture would be a 'clean' verdict about the day scene, so
  // refuse (exit 6) instead of judging it. Read again after the capture so a
  // switch mid-run cannot slip through.
  const appearance = () => {
    try { return worldAppearanceFromDefaults(execFileSync('defaults', ['read', '-g', 'AppleInterfaceStyle'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }), true); } catch { return worldAppearanceFromDefaults('', false); }
  };
  if (!existsSync(binaryPath)) done('no-app-binary', { binary: binaryPath }, 1);
  // CGWindowList via a tiny Swift script: owner pid, layer and bounds need no
  // Screen Recording permission (only window titles and pixels do).
  const lister = `import CoreGraphics
import Foundation
let list = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
for w in list {
  let b = (w[kCGWindowBounds as String] as? [String: Any]) ?? [:]
  let n = { (k: String) -> Double in (b[k] as? NSNumber)?.doubleValue ?? 0 }
  print("{\\"id\\":\\((w[kCGWindowNumber as String] as? Int) ?? 0),\\"pid\\":\\((w[kCGWindowOwnerPID as String] as? Int) ?? 0),\\"layer\\":\\((w[kCGWindowLayer as String] as? Int) ?? -1),\\"x\\":\\(n("X")),\\"y\\":\\(n("Y")),\\"w\\":\\(n("Width")),\\"h\\":\\(n("Height"))}")
}
`;
  await mkdir(outDir, { recursive: true });
  for (;;) {
    if (locked()) { await retryOr('screen-locked', {}, 4); continue; }
    if (appearance() !== 'night') { await retryOr('not-night', { appearance: 'day', fix: 'System Settings > Appearance > Dark, then run again' }, 6); continue; }
    const running = runningAppProcesses(execFileSync('ps', ['-axo', 'pid=,etime=,comm='], { encoding: 'utf8' }), binaryPath, (await stat(binaryPath)).mtimeMs, Date.now());
    if (!running.length) { await retryOr('app-not-running', { binary: binaryPath }, 1); continue; }
    // A window of a process older than the binary shows the PREVIOUS build:
    // judging it would be a verdict on code that is no longer shipped.
    if (running.some((r) => r.stale)) done('running-app-predates-binary', { running, fix: 'relaunch Permagent.app' }, 3);
    let rows = [];
    try { rows = execFileSync('swift', ['-'], { input: lister, encoding: 'utf8' }).split('\n').filter((l) => l.startsWith('{')).map((l) => JSON.parse(l)); } catch (e) { done('window-list-failed', { error: String(e).slice(0, 300) }, 5); }
    const win = pickAppWindow(rows, running.map((r) => r.pid));
    if (!win) { await retryOr('no-app-window-on-screen', { running }, 1); continue; }
    const shot = path.join(outDir, `app-window-${new Date().toISOString().replace(/[:.]/g, '-')}.png`);
    // -x no sound, -o no window shadow (the shadow's soft edge is not the app).
    try { execFileSync('screencapture', ['-x', '-o', '-l', String(win.id), shot]); } catch (e) { done('capture-failed', { window: win, error: String(e).slice(0, 300) }, 5); }
    if (!existsSync(shot)) done('capture-failed', { window: win, error: 'no file written (Screen Recording permission?)' }, 5);
    // The screen can lock between the check and the capture.
    if (locked()) { await rm(shot, { force: true }); await retryOr('screen-locked', {}, 4); continue; }
    if (appearance() !== 'night') done('not-night', { appearance: 'day', capture: shot, note: 'appearance changed during the capture' }, 6);
    const img = decodePng(await readFile(shot));
    // The capture is in device pixels; the run and block thresholds are CSS px.
    const scale = img.width / win.w;
    const r = extremeReport(img, scale);
    // No Vision means no text, and then the World is not confirmed: no clean verdict.
    const ocr = readText(shot);
    const world = worldFromOcr(ocr);
    const report = { running, window: win, capture: shot, scale, appearance: 'night', world, solid: r.solid, black: r.black, white: r.white, clip: clipReport(img) };
    // A solid block in the app window is a finding whatever view is open.
    if (r.solid) done('artifact-in-app-window', report, 2);
    // Not the World: a clean frame of another view says nothing about the gas
    // giant. Drop the capture (it is the user's screen, and it is not evidence).
    if (!world.world) { await rm(shot, { force: true }); await retryOr('world-not-open', { running, window: win, ocrLines: ocr.length, fix: 'open the World in Permagent.app' }, 7); continue; }
    // The World says what it resolved; trust that over the system setting.
    if (world.appearance === 'day') done('not-night', { ...report, appearance: 'day', note: 'the World resolved day' }, 6);
    done('clean-app-window', report, 0);
  }
}

if (analyzeFiles.length) {
  const { readFile } = await import('node:fs/promises');
  const scale = Number(process.env.DPR || 1);
  for (const file of analyzeFiles) {
    const img = decodePng(await readFile(file));
    const r = extremeReport(img, scale);
    // WORLD=1 also reads the frame's text, so a screenshot of the real app says
    // whether it shows the World (the same check --app-window refuses on).
    const world = process.env.WORLD === '1' ? worldFromOcr(readText(file)) : undefined;
    console.log(JSON.stringify({ file: path.basename(file), solid: r.solid, black: r.black, white: r.white, clip: clipReport(img), world }));
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
// Seed the World's layout before any page script reads it (see appStorage).
if (Object.keys(storage).length) {
  await context.addInitScript((entries) => { for (const [k, v] of Object.entries(entries)) { try { localStorage.setItem(k, v); } catch { /* storage blocked */ } } }, storage);
}
const plant = process.env.PLANT || '';
if (plant && !['black', 'white'].includes(plant)) throw new Error(`PLANT must be black or white, got ${plant}`);
if (plant) {
  await context.addInitScript((colour) => {
    const lay = () => {
      const d = document.createElement('div');
      d.setAttribute('data-probe-plant', colour);
      d.style.cssText = `position:fixed;left:437px;top:0;width:340px;height:251px;background:${colour === 'black' ? '#000' : '#fff'};z-index:2147483647;pointer-events:none`;
      document.body.appendChild(d);
    };
    if (document.body) lay(); else document.addEventListener('DOMContentLoaded', lay);
  }, plant);
}
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', (err) => pageErrors.push(String(err)));
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
await page.waitForTimeout(settleMs);
// Refuse to report "clean" on a frame that is not the scene being asked about:
// the forum prints which appearance it resolved, and a day frame says nothing
// about a night defect.
const appearance = await page.locator('[data-appearance]').first().getAttribute('data-appearance').catch(() => null);
// And which layout it actually rendered, so a seeded run can be checked.
const deskState = await page.locator('[data-desk]').first().getAttribute('data-desk').catch(() => null);
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
  if (report.solid || process.env.SAVE_ALL) await writeFile(path.join(outDir, `${label}-${backend}-${i}.png`), buffer);
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
  storage,
  plant: plant || null,
  desk: deskState,
  renderer,
  pageErrors: pageErrors.slice(0, 5),
  frames: results.length,
  // A frame has the artifact only if the runs form a SOLID block (see
  // SOLID_FILL): long extreme runs that are scattered texture are counted
  // below, in the pixel totals, but they are not the rectangle.
  framesWithArtifact: results.filter(r => r.solid).length,
  framesWithExtremeRuns: results.filter(r => r.count > 0).length,
  totalExtremePixels: total,
  blackPixels: results.reduce((sum, r) => sum + r.black.count, 0),
  whitePixels: results.reduce((sum, r) => sum + r.white.count, 0),
  worstFrame: worst,
  // The clipping proxy (see clipReport): per frame, so a flicker shows.
  clipPerFrame: results.map((r) => ({ white: r.clip.white, anyChannel: r.clip.anyChannel, largestWhite: r.clip.largestWhite })),
}, null, 2));
