#!/usr/bin/env node
/*
 * Job 20 evidence: the World's new default vantage, in the real app.
 *
 * Three things this has to show, and none of them can be asserted from a unit
 * test:
 *
 *   frame      which of the north-star landmarks
 *              (`src/components/world/forum/vistaLandmarks.json`) actually
 *              project inside the live canvas, read from the LIVE camera
 *              rather than from the constant the app was supposed to apply
 *   day/night  the same vantage in both appearances, at 1280 x 1000
 *   sky        the gas giant and the moon measured against the sky right
 *              behind them, in DAY as well as night — job 19 bug 1
 *
 * Auth, the read-only React-DevTools probe, the PNG decoder and the black-frame
 * guard are `verify-world-in-app-v2.mjs` and `verify-world-night-v3.mjs`'s
 * code, reused unchanged in behaviour: the daemon bearer token is read from the
 * daemon's own secrets file into `localStorage['permagent-daemon-token']`
 * before any page script runs, and is never printed and never written to the
 * report. No chat message is sent and nothing in the page is mutated.
 *
 * Usage (from ui/command-center, with the dev server on 5284):
 *   node scripts/capture-vista-v4.mjs
 */
import { chromium } from 'playwright';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const appUrl = process.env.APP_URL || process.argv[2] || 'http://127.0.0.1:5284/ui/';
const origin = new URL(appUrl).origin;
const shotDir = path.join(repo, 'assets/world/forum/browser');
const reportPath = path.join(repo, 'docs/design/solar-forum/vista-frame-v4.json');
const tokenFile = process.env.PERMAGENT_TOKEN_FILE || path.join(os.homedir(), '.permagent/secrets/daemon_token.json');
const viewport = { width: Number(process.env.VIEW_W || 1280), height: Number(process.env.VIEW_H || 1000) };

await mkdir(shotDir, { recursive: true });

const token = JSON.parse(await readFile(tokenFile, 'utf8')).token;
if (typeof token !== 'string' || !token) throw new Error(`no daemon token in ${tokenFile}`);
const landmarkTable = JSON.parse(await readFile(path.join(here, '../src/components/world/forum/vistaLandmarks.json'), 'utf8'));

async function daemon(endpoint) {
  const res = await fetch(`${origin}${endpoint}`, { headers: { Authorization: `Bearer ${token}` } });
  const text = await res.text();
  try { return { status: res.status, body: JSON.parse(text) }; } catch { return { status: res.status, body: text.slice(0, 200) }; }
}

// --- PNG decode and luma (verify-world-in-app-v2.mjs) ------------------------
function decodePng(buffer) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  if (!buffer.subarray(0, 8).equals(signature)) return { supported: false, reason: 'not-png' };
  let offset = 8, width = 0, height = 0, colorType = 0, bitDepth = 0;
  const chunks = [];
  while (offset < buffer.length) {
    if (offset + 8 > buffer.length) return { supported: false, reason: 'truncated-png-chunk' };
    const length = buffer.readUInt32BE(offset); offset += 4;
    if (length > buffer.length - offset - 4) return { supported: false, reason: 'invalid-png-chunk-length' };
    const type = buffer.toString('ascii', offset, offset + 4); offset += 4;
    const data = buffer.subarray(offset, offset + length); offset += length + 4;
    if (type === 'IHDR') { width = data.readUInt32BE(0); height = data.readUInt32BE(4); bitDepth = data[8]; colorType = data[9]; }
    if (type === 'IDAT') chunks.push(data);
    if (type === 'IEND') break;
  }
  if (bitDepth !== 8 || (colorType !== 2 && colorType !== 6) || !width || !height) return { supported: false, reason: 'unsupported-png-format' };
  const channels = colorType === 6 ? 4 : 3;
  const stride = width * channels;
  let raw;
  try { raw = inflateSync(Buffer.concat(chunks)); } catch { return { supported: false, reason: 'invalid-png-deflate' }; }
  if (raw.length < height * (stride + 1)) return { supported: false, reason: 'truncated-png-pixels' };
  const pixels = Buffer.alloc(height * stride);
  let src = 0;
  const paeth = (a, b, c) => { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < height; y++) {
    const filter = raw[src++];
    if (filter > 4) return { supported: false, reason: 'invalid-png-filter-data' };
    const row = y * stride, prior = (y - 1) * stride;
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? pixels[row + x - channels] : 0;
      const up = y ? pixels[prior + x] : 0;
      const upperLeft = y && x >= channels ? pixels[prior + x - channels] : 0;
      const value = raw[src++];
      pixels[row + x] = filter === 0 ? value : filter === 1 ? value + left : filter === 2 ? value + up : filter === 3 ? value + Math.floor((left + up) / 2) : value + paeth(left, up, upperLeft);
    }
  }
  return { supported: true, width, height, channels, stride, pixels };
}

const luma = (p, at) => .2126 * p[at] + .7152 * p[at + 1] + .0722 * p[at + 2];

/** Job 19 bug 2's guard: some frames come back from ANGLE/Metal with a solid
 *  block of exact RGB 0,0,0 across the canvas. Any measurement inside one is
 *  worthless, so every capture is counted before it is trusted. */
function pureBlackPixels(img) {
  let n = 0;
  for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) {
    const at = y * img.stride + x * img.channels;
    if (img.pixels[at] === 0 && img.pixels[at + 1] === 0 && img.pixels[at + 2] === 0) n++;
  }
  return n;
}

/** Mean luminance of a disc and of the annulus around it. */
function discLuma(img, cx, cy, r) {
  let inSum = 0, inN = 0, outSum = 0, outN = 0, inMax = 0;
  const inside = [], outer = r * 2.6;
  const x0 = Math.max(0, Math.floor(cx - outer)), x1 = Math.min(img.width, Math.ceil(cx + outer));
  const y0 = Math.max(0, Math.floor(cy - outer)), y1 = Math.min(img.height, Math.ceil(cy + outer));
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const d = Math.hypot(x - cx, y - cy), at = y * img.stride + x * img.channels, l = luma(img.pixels, at);
    if (d <= r * .8) { inSum += l; inN++; inside.push(l); if (l > inMax) inMax = l; }
    else if (d >= r * 1.5 && d <= outer) { outSum += l; outN++; }
  }
  if (!inN || !outN) return { measured: false, reason: 'disc-outside-frame', centre: [Math.round(cx), Math.round(cy)], radiusPx: Math.round(r) };
  inside.sort((a, b) => a - b);
  const round = n => Math.round(n * 100) / 100;
  const surround = outSum / outN;
  return {
    measured: true,
    centre: [Math.round(cx), Math.round(cy)], radiusPx: Math.round(r * 10) / 10, discPixels: inN,
    discMeanLuma: round(inSum / inN), discMedianLuma: round(inside[Math.floor(inside.length / 2)]), discMaxLuma: round(inMax),
    surroundMeanLuma: round(surround),
    contrastRatio: round((inSum / inN) / Math.max(.5, surround)),
  };
}

/** Mean channel values over a fractional region — the night ground-tint check.
 *  Blue-minus-red alone does not separate the two cases: the old lavender
 *  paving was blue-dominant too. What made it read as lavender was red and
 *  green being *equal* under that blue, i.e. a magenta component. Cool
 *  moonlight grades red < green < blue, so green-minus-red is the number that
 *  tells them apart. */
function regionTint(img, [fx0, fy0, fx1, fy1]) {
  const x0 = Math.floor(fx0 * img.width), x1 = Math.min(img.width, Math.ceil(fx1 * img.width));
  const y0 = Math.floor(fy0 * img.height), y1 = Math.min(img.height, Math.ceil(fy1 * img.height));
  let r = 0, g = 0, b = 0, n = 0;
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const at = y * img.stride + x * img.channels;
    r += img.pixels[at]; g += img.pixels[at + 1]; b += img.pixels[at + 2]; n++;
  }
  const round = v => Math.round((v / n) * 100) / 100;
  return {
    region: [fx0, fy0, fx1, fy1], pixels: n, meanRgb: [round(r), round(g), round(b)],
    blueMinusRed: Math.round(((b - r) / n) * 100) / 100,
    greenMinusRed: Math.round(((g - r) / n) * 100) / 100,
  };
}

// --- browser ----------------------------------------------------------------
// ANGLE over OpenGL, not Metal. Job 19 bug 2 recorded an intermittent block of
// exact RGB 0,0,0 in the rendered frame and could only work around it by
// re-aiming; on this pose it is not intermittent at all. Measured here, four
// browsers side by side on the same app and the same frame:
//
//   --use-angle=metal                          67,447 black px at [595,28]-[909,310]
//   --use-angle=metal --disable-gpu-rasterization  67,447, same rectangle
//   --use-angle=metal --in-process-gpu             67,447, same rectangle
//   --use-angle=gl                                      0
//
// The same rectangle to the pixel across separate browser processes is an ANGLE
// **Metal** backend defect, not a compositor race and not a page bug, so this
// harness drives the GL backend. It is therefore not comparable to job 18/19's
// FPS numbers, and does not try to be: nothing here measures frame cost.
const gpuArgs = (process.env.ANGLE_BACKEND === 'metal'
  ? ['--use-angle=metal']
  : ['--use-angle=gl']
).concat(['--enable-gpu', '--ignore-gpu-blocklist', '--disable-frame-rate-limit', '--disable-gpu-vsync']);
const channel = process.env.BROWSER_CHANNEL === undefined ? 'chrome' : (process.env.BROWSER_CHANNEL || undefined);
const browser = await chromium.launch({ headless: true, channel, args: gpuArgs });
const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
await context.addInitScript(([key, value]) => { try { localStorage.setItem(key, value); } catch { /* storage blocked */ } }, ['permagent-daemon-token', token]);

// The job-18/19 read-only instrumentation: a DevTools hook stub gives us r3f's
// fiber roots, and every r3f-created THREE object carries `__r3f.root`.
await context.addInitScript(() => {
  if (!window.__REACT_DEVTOOLS_GLOBAL_HOOK__) {
    const roots = [];
    window.__probeRoots = roots;
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
      supportsFiber: true, isDisabled: false, renderers: new Map(),
      inject(renderer) { const id = this.renderers.size + 1; this.renderers.set(id, renderer); return id; },
      onCommitFiberRoot(_id, root) { if (root && !roots.includes(root)) roots.push(root); },
      onPostCommitFiberRoot() {}, onCommitFiberUnmount() {}, checkDCE() {},
      on() {}, off() {}, emit() {}, sub() { return () => {}; },
      getFiberRoots() { return new Set(roots); },
    };
  }
  window.__findR3FStore = function findR3FStore() {
    let budget = 400000;
    for (const root of window.__probeRoots || []) {
      const stack = [root.current], seen = new Set();
      while (stack.length && budget-- > 0) {
        const fiber = stack.pop();
        if (!fiber || seen.has(fiber)) continue;
        seen.add(fiber);
        const r3f = fiber.stateNode && fiber.stateNode.__r3f;
        if (r3f && r3f.root && typeof r3f.root.getState === 'function') return r3f.root;
        if (fiber.child) stack.push(fiber.child);
        if (fiber.sibling) stack.push(fiber.sibling);
      }
    }
    return null;
  };
  /** The live camera, and where each landmark sample point lands on it. */
  window.__vistaProbe = function vistaProbe(table) {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const state = store.getState();
    const camera = state.camera, size = state.size;
    if (!camera) return { ok: false, reason: 'no-camera' };
    camera.updateMatrixWorld();
    const e = camera.matrixWorld.elements;
    const position = { x: e[12], y: e[13], z: e[14] };
    const direction = { x: -e[8], y: -e[9], z: -e[10] };
    const round = n => Math.round(n * 1000) / 1000;
    const Vec = camera.position.constructor;
    const project = ([x, y, z]) => {
      const v = new Vec(x, y, z);
      const world = v.clone();
      v.project(camera);
      return {
        ndc: [round(v.x), round(v.y)],
        px: [Math.round((v.x * .5 + .5) * size.width), Math.round((-v.y * .5 + .5) * size.height)],
        inFrame: v.z > -1 && v.z < 1 && Math.abs(v.x) <= 1 && Math.abs(v.y) <= 1,
        distanceM: Math.round(world.distanceTo(camera.position)),
      };
    };
    const landmarks = table.landmarks.map(l => {
      const points = l.points.map(project);
      const inside = points.filter(p => p.inFrame);
      const nearest = points.slice().sort((a, b) => Math.max(...a.ndc.map(Math.abs)) - Math.max(...b.ndc.map(Math.abs)))[0];
      return {
        id: l.id, label: l.label, source: l.source,
        points: points.length, inFrame: inside.length, visible: inside.length > 0,
        nearestToCentre: nearest && nearest.inFrame !== undefined ? { ndc: nearest.ndc, px: nearest.px, distanceM: nearest.distanceM, inFrame: nearest.inFrame } : null,
        samplePixels: inside.slice(0, 6).map(p => p.px),
      };
    });
    // Both sky bodies, found the way the other harnesses find them: by the
    // sphere radius ForumSky builds them at.
    const bodies = [];
    state.scene.traverse(o => {
      if (!o.isMesh || !o.geometry || o.geometry.type !== 'SphereGeometry') return;
      const r = o.geometry.parameters && o.geometry.parameters.radius;
      const label = r === 140 ? 'gasGiant' : r === 46 ? 'moon' : null;
      if (!label) return;
      o.updateMatrixWorld();
      const world = new Vec(o.matrixWorld.elements[12], o.matrixWorld.elements[13], o.matrixWorld.elements[14]);
      const v = world.clone(); v.project(camera);
      const edge = world.clone().add(camera.up.clone().multiplyScalar(r)); edge.project(camera);
      bodies.push({
        label, material: o.material && o.material.name,
        world: { x: round(world.x), y: round(world.y), z: round(world.z) },
        ndc: [round(v.x), round(v.y)],
        px: [Math.round((v.x * .5 + .5) * size.width), Math.round((-v.y * .5 + .5) * size.height)],
        radiusPx: Math.round(Math.abs(edge.y - v.y) * .5 * size.height),
        inFrame: v.z > -1 && v.z < 1 && Math.abs(v.x) <= 1 && Math.abs(v.y) <= 1,
      });
    });
    return {
      ok: true,
      canvas: { width: size.width, height: size.height, aspect: Math.round((size.width / size.height) * 1e4) / 1e4 },
      camera: {
        position: { x: round(position.x), y: round(position.y), z: round(position.z) },
        fov: camera.fov ?? null,
        yawDegEastOfNorth: round(Math.atan2(direction.x, -direction.z) * 180 / Math.PI),
        pitchDeg: round(Math.asin(Math.max(-1, Math.min(1, direction.y))) * 180 / Math.PI),
      },
      landmarks, bodies,
    };
  };
});

const page = await context.newPage();
page.setDefaultTimeout(90000);
const consoleErrors = [];
page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 240)); });

const hasTool = (node, tool) => {
  if (!node || typeof node !== 'object') return false;
  if (node.tool === tool || node.type === tool || node.id === tool) return true;
  return Array.isArray(node.children) && node.children.some(c => hasTool(c, tool));
};
const ws = await daemon('/api/workspaces');
if (ws.status !== 200 || !Array.isArray(ws.body)) throw new Error(`GET /api/workspaces -> ${ws.status}`);
const worldWorkspace = ws.body
  .map(w => ({ id: w.id, name: w.name, layout: typeof w.layoutJson === 'string' ? JSON.parse(w.layoutJson) : w.layoutJson }))
  .find(w => hasTool(w.layout, 'world'));
if (!worldWorkspace) throw new Error("no workspace hosts the 'world' tool");

await page.goto(appUrl, { waitUntil: 'domcontentloaded', timeout: 60000 });
await page.waitForFunction(() => !document.body.textContent.includes('Loading workspaces'), undefined, { timeout: 60000 }).catch(() => {});
if (!(await page.locator('.forum-shell').first().isVisible().catch(() => false))) {
  await page.getByRole('button', { name: worldWorkspace.name, exact: false }).first().click({ timeout: 20000 });
}
await page.waitForFunction(() => [...document.querySelectorAll('.forum-shell')].some(s => s.getBoundingClientRect().width > 1), undefined, { timeout: 90000 });
await page.waitForFunction(() => Boolean(window.__worldPerf), undefined, { timeout: 120000 }).catch(() => {});
// Park the pointer off every hoverable surface: a sidebar tooltip ("World
// Cmd-8") was being captured over the frame's left edge, which is exactly the
// column the ridge occupies.
await page.mouse.move(viewport.width - 6, 6);
await page.waitForTimeout(4000);

/** Job 19 bug 2: on this machine some frames come back from ANGLE/Metal with a
 *  solid axis-aligned block of exact RGB 0,0,0 across the canvas, and it sticks
 *  to a camera pose. Job 19 cleared it by re-aiming, which this job cannot do
 *  — the pose IS the evidence — so the drawing buffer is reallocated instead by
 *  nudging the viewport a pixel and back. Every recorded frame reports the
 *  pure-black count it was accepted with. */
async function capture(name) {
  const file = name ? `assets/world/forum/browser/${name}.png` : null;
  let last = null;
  for (let attempt = 1; attempt <= 4; attempt++) {
    const buffer = await page.screenshot({ timeout: 120000 });
    const img = decodePng(buffer);
    if (!img.supported) return { file, image: null, decode: img };
    const black = pureBlackPixels(img);
    last = { file, image: img, pureBlackPixels: black, attempts: attempt, buffer };
    if (black < 2000) break;
    await page.setViewportSize({ width: viewport.width + (attempt % 2 ? 1 : -1), height: viewport.height });
    await page.waitForTimeout(700);
    await page.setViewportSize(viewport);
    await page.waitForTimeout(1200);
  }
  if (file && last && last.buffer) await writeFile(path.join(repo, file), last.buffer);
  if (last) delete last.buffer;
  return last;
}

const HUD = '.forum-brand, .forum-districts, .forum-location, .forum-controls, .forum-play, .forum-live, .forum-waypoint';
/** The sky bodies sit behind the World HUD — the title card and the district
 *  pills are DOM on top of the canvas, so a disc sampled through them measures
 *  the card, not the sky. The overlay is hidden for the measurement frame only
 *  and restored immediately; the evidence screenshots are the app as shipped. */
async function withHudHidden(fn) {
  await page.evaluate(selector => {
    for (const el of document.querySelectorAll(selector)) el.style.visibility = 'hidden';
  }, HUD);
  await page.waitForTimeout(700);
  try { return await fn(); }
  finally {
    await page.evaluate(selector => {
      for (const el of document.querySelectorAll(selector)) el.style.visibility = '';
    }, HUD);
    await page.waitForTimeout(400);
  }
}

async function measure(appearance, shotName) {
  await page.emulateMedia({ colorScheme: appearance === 'night' ? 'dark' : 'light' });
  await page.waitForTimeout(2500);
  const probe = await page.evaluate(table => window.__vistaProbe(table), landmarkTable);
  const canvasBox = await page.locator('.forum-shell canvas').first().boundingBox();
  const skyShot = await withHudHidden(() => capture(`${shotName}-sky`));
  // ...and the same frame again with the world itself hidden. Job 19 used this
  // to prove the ridge and the Maker hall roof really do occlude a body; here
  // it separates "the body does not read against the day sky" from "the first
  // orbital ring is standing in front of a third of its disc", which is real
  // geometry and not a lighting failure.
  const bareSky = await withHudHidden(async () => {
    const hidden = await page.evaluate(() => {
      const store = window.__findR3FStore();
      if (!store) return { ok: false };
      const keep = new Set([140, 46, 4800, 5.2]);
      const marked = [];
      store.getState().scene.traverse(o => {
        if (!o.isMesh || !o.visible) return;
        const p = o.geometry && o.geometry.parameters;
        const isSky = o.geometry && ((o.geometry.type === 'SphereGeometry' && keep.has(p && p.radius))
          || (o.geometry.type === 'PlaneGeometry' && p && p.width === 650));
        if (isSky) return;
        o.visible = false;
        marked.push(o);
      });
      window.__vistaHidden = marked;
      return { ok: true, hidden: marked.length };
    });
    const frame = await capture(null);
    await page.evaluate(() => { for (const o of window.__vistaHidden || []) o.visible = true; window.__vistaHidden = []; });
    return { hidden, frame };
  });
  const sky = {};
  if (skyShot && skyShot.image && probe.ok && canvasBox) {
    for (const body of probe.bodies) {
      if (!body.inFrame) { sky[body.label] = { measured: false, reason: 'not-in-frame', ndc: body.ndc }; continue; }
      sky[body.label] = discLuma(skyShot.image, canvasBox.x + body.px[0], canvasBox.y + body.px[1], Math.max(8, body.radiusPx));
    }
  }
  const bare = {};
  if (bareSky && bareSky.frame && bareSky.frame.image && probe.ok && canvasBox) {
    for (const body of probe.bodies) {
      if (!body.inFrame) continue;
      bare[body.label] = discLuma(bareSky.frame.image, canvasBox.x + body.px[0], canvasBox.y + body.px[1], Math.max(8, body.radiusPx));
    }
  }
  const shot = await capture(shotName);
  // The commons paving, in the lower-middle of the canvas, as a fraction of the
  // whole 1280 x 1000 page.
  const pavingRegion = canvasBox && shot.image
    ? [(canvasBox.x + canvasBox.width * .25) / shot.image.width, (canvasBox.y + canvasBox.height * .72) / shot.image.height,
       (canvasBox.x + canvasBox.width * .75) / shot.image.width, (canvasBox.y + canvasBox.height * .95) / shot.image.height]
    : null;
  return {
    appearance,
    appearanceLabel: await page.locator('[data-testid="forum-appearance"]').first().textContent().catch(() => null),
    screenshot: shot.file,
    pureBlackPixels: shot.pureBlackPixels ?? null,
    captureAttempts: shot.attempts ?? null,
    skyMeasurementFrame: skyShot ? { screenshot: skyShot.file, pureBlackPixels: skyShot.pureBlackPixels ?? null, attempts: skyShot.attempts ?? null, hudHidden: HUD } : null,
    canvasBox: canvasBox ? { x: Math.round(canvasBox.x), y: Math.round(canvasBox.y), width: Math.round(canvasBox.width), height: Math.round(canvasBox.height) } : null,
    probe,
    sky,
    skyWithWorldHidden: { meshesHidden: bareSky && bareSky.hidden ? bareSky.hidden.hidden : null, pureBlackPixels: bareSky && bareSky.frame ? bareSky.frame.pureBlackPixels : null, bodies: bare },
    groundTint: pavingRegion && shot.image ? regionTint(shot.image, pavingRegion) : null,
    // Recorded, but NOT a frame-cost claim: this harness runs the ANGLE GL
    // backend to get an uncorrupted frame (see `gpuArgs`), which is far slower
    // than the Metal backend the product actually uses. Job 19's Metal numbers
    // are the FPS record; nothing here supersedes them.
    perfOnGlBackendNotComparable: await page.evaluate(() => (window.__worldPerf ? { ...window.__worldPerf } : null)),
  };
}

/** Drag to orbit, scroll to explore — still working from the new start pose,
 *  and the pose comes back when the Commons is reselected. */
async function orbitCheck(box) {
  let restoreError = null;
  const read = async () => (await page.evaluate(() => window.__vistaProbe({ landmarks: [] }))).camera;
  const start = await read();
  const cx = box.x + box.width / 2, cy = box.y + box.height * .62;
  await page.mouse.move(cx, cy); await page.mouse.down();
  await page.mouse.move(cx - 90, cy - 30, { steps: 12 });
  await page.mouse.up();
  await page.waitForTimeout(600);
  const dragged = await read();
  await page.mouse.move(cx, cy);
  await page.mouse.wheel(0, -320);
  await page.waitForTimeout(600);
  const scrolled = await read();
  // Put the default vantage back: the district effect only re-runs when the
  // district object changes, so this leaves and returns.
  // The district pills sit under the in-canvas waypoint labels, so Playwright's
  // actionability wait does not settle on them; job 11's `robustClick` fallback.
  const districts = page.locator('.forum-districts button');
  const pick = async name => {
    const button = districts.filter({ hasText: name }).first();
    try { await button.click({ timeout: 8000 }); }
    catch {
      try { await button.click({ force: true, timeout: 8000 }); }
      catch (e) { restoreError = `${name}: ${String(e.message).slice(0, 100)} (fell back to dispatchEvent)`; await button.dispatchEvent('click'); }
    }
    await page.waitForTimeout(1500);
  };
  await pick('Gallery');
  await pick('Commons');
  const restored = await read();
  const moved = (a, b) => Math.round(Math.hypot(a.position.x - b.position.x, a.position.y - b.position.y, a.position.z - b.position.z) * 100) / 100;
  return {
    start, afterDrag: dragged, afterScroll: scrolled, restored,
    dragMovedCameraM: moved(start, dragged),
    scrollMovedCameraM: moved(dragged, scrolled),
    restoredWithinM: moved(start, restored),
    restoredFovMatches: restored.fov === start.fov,
    restoreError,
  };
}

const day = await measure('day', 'v4-vista-day');
const night = await measure('night', 'v4-vista-night');
await page.emulateMedia({ colorScheme: 'light' });
await page.waitForTimeout(1500);
// Last, because it deliberately moves the camera: the evidence frames above are
// the untouched default vantage.
const orbit = await orbitCheck(await page.locator('.forum-shell canvas').first().boundingBox());

const report = {
  schema: 'permagent.forum.vista-frame.v4',
  job: 'docs/design/solar-forum/jobs/20-vista-camera.md',
  capturedAt: new Date().toISOString(),
  appUrl, viewport,
  browser: { channel: channel || 'playwright-bundled-chromium', version: browser.version(), args: gpuArgs },
  tokenSource: `${tokenFile} (value never recorded)`,
  landmarkTable: 'ui/command-center/src/components/world/forum/vistaLandmarks.json',
  orbitControls: orbit,
  day, night,
  consoleErrorSample: consoleErrors.slice(0, 12),
  consoleErrors: consoleErrors.length,
};
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
await browser.close();

const summary = rows => rows.map(r => `${r.visible ? 'IN ' : 'OUT'} ${r.id} ${r.inFrame}/${r.points}`).join('\n  ');
console.log(`camera ${JSON.stringify(day.probe.camera)} canvas ${JSON.stringify(day.probe.canvas)}`);
console.log(`landmarks (day):\n  ${summary(day.probe.landmarks)}`);
for (const [name, run] of [['day', day], ['night', night]]) {
  for (const [body, m] of Object.entries(run.sky)) console.log(`${name} ${body}: ${m.measured ? `disc ${m.discMeanLuma} / sky ${m.surroundMeanLuma} = ${m.contrastRatio}` : `not measured (${m.reason})`}`);
  for (const [body, m] of Object.entries(run.skyWithWorldHidden.bodies)) console.log(`${name} ${body} (world hidden): ${m.measured ? `disc ${m.discMeanLuma} / sky ${m.surroundMeanLuma} = ${m.contrastRatio}` : `not measured (${m.reason})`}`);
  console.log(`${name} paving tint ${JSON.stringify(run.groundTint && run.groundTint.meanRgb)} blue-red ${run.groundTint && run.groundTint.blueMinusRed} green-red ${run.groundTint && run.groundTint.greenMinusRed}`);
}
console.log(`orbit: drag moved ${orbit.dragMovedCameraM} m, scroll moved ${orbit.scrollMovedCameraM} m, pose restored within ${orbit.restoredWithinM} m (fov match ${orbit.restoredFovMatches})`);
console.log(`report: ${path.relative(repo, reportPath)}`);
