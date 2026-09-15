#!/usr/bin/env node
/*
 * In-app Solar Forum evidence, second pass — the NEW World export.
 *
 * `verify-world-in-app.mjs` (job 11) proved the shipping route. Since then the
 * export changed completely: a 14.4 MB meshopt-compressed GLB with a lagoon,
 * towers, a ridge, groves and the MESH gate court, a gas-giant sunrise sky with
 * bloom, and emissive night window strips. This harness re-runs the parts of
 * job 11 that could regress and adds the checks that only exist now:
 *
 *   load   the real app's World tool loads the new revision, decoded by
 *          MeshoptDecoder, with FPS/draw calls/triangles on ANGLE/Metal
 *   mesh   walk mode reaches the gate approach, walks OUT through the ring
 *          plane, the Agora opens with its honest plaque, Escape/Return puts
 *          the walker back on the spur
 *   night  prefers-color-scheme: dark -> night sky, celestial bodies, lit
 *          emissive window strips, and the FPS cost of bloom at night
 *   ask    the job-11 query gate ("reply with the single word PONG")
 *   skills the job-11 skills-panel seam
 *   remount switch away/back with a WASD leak check
 *
 * Auth, canvas sampling, robust clicking and the FPS probe are the job-11 code:
 * the daemon bearer token is read from the daemon's own secrets file and placed
 * in localStorage under `permagent-daemon-token` before any page script runs,
 * exactly as the app's `browserToken()` does. The token is never printed and
 * never written to the report.
 *
 * Walker position is read WITHOUT touching `src/`. `ForumWalk` keeps the
 * walker's feet in a React ref and publishes nothing, so this harness installs
 * a minimal React DevTools hook stub in an init script; react-three-fiber calls
 * `injectIntoDevTools`, which hands us its fiber roots, and any r3f-created
 * THREE object carries `__r3f.root` — the zustand store holding the live
 * camera. Read-only: nothing in the page is mutated.
 *
 * Every prompt this script sends is prefixed "[world-e2e-test]".
 *
 * Usage (from ui/command-center, with the dev server on 5284):
 *   node scripts/verify-world-in-app-v2.mjs
 *   GATES=load,mesh node scripts/verify-world-in-app-v2.mjs
 */
import { chromium } from 'playwright';
import { mkdir, writeFile, readFile, stat } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const appUrl = process.env.APP_URL || process.argv[2] || 'http://127.0.0.1:5284/ui/';
const origin = new URL(appUrl).origin;
const shotDir = path.join(repo, 'assets/world/forum/browser');
const reportPath = path.join(repo, 'docs/design/solar-forum/in-app-report-v2.json');
const tokenFile = process.env.PERMAGENT_TOKEN_FILE || path.join(os.homedir(), '.permagent/secrets/daemon_token.json');
const only = (process.env.GATES || '').split(',').map(s => s.trim()).filter(Boolean);
// The task's viewport: 1280x1000, narrower than job 11's 1600x1000.
const viewport = { width: Number(process.env.VIEW_W || 1280), height: Number(process.env.VIEW_H || 1000) };
const TAG = '[world-e2e-test]';

await mkdir(shotDir, { recursive: true });

// --- credential (never logged) ---------------------------------------------
const token = JSON.parse(await readFile(tokenFile, 'utf8')).token;
if (typeof token !== 'string' || !token) throw new Error(`no daemon token in ${tokenFile}`);

async function daemon(endpoint, init) {
  const res = await fetch(`${origin}${endpoint}`, {
    ...init,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}`, ...(init?.headers || {}) },
  });
  const text = await res.text();
  let body = null;
  try { body = JSON.parse(text); } catch { body = text.slice(0, 400); }
  return { status: res.status, body };
}

const glbPath = path.join(repo, 'ui/command-center/public/world/solar-forum.glb');
const glbBytes = (await stat(glbPath)).size;

const report = {
  appUrl,
  viewport,
  startedAt: new Date().toISOString(),
  tokenSource: `${tokenFile} (value never recorded)`,
  shippedGlb: { path: 'ui/command-center/public/world/solar-forum.glb', bytes: glbBytes },
  notes: [
    'Drives the real Command Center app against the live local daemon.',
    'Every sent prompt is prefixed "[world-e2e-test]".',
    'Walker position comes from a read-only React DevTools hook stub, not a src/ change.',
  ],
  gates: {},
};
// A subset run (GATES=...) must not throw away the gates recorded by an earlier
// pass: merge whatever is already on disk and replace only what this run redid.
if (only.length) {
  try {
    const previous = JSON.parse(await readFile(reportPath, 'utf8'));
    if (previous && typeof previous === 'object' && previous.gates) {
      report.gates = previous.gates;
      report.mergedFromPreviousRun = { startedAt: previous.startedAt, gates: Object.keys(previous.gates), replaced: only };
    }
  } catch { /* no previous report */ }
}

// A gate re-run must not inherit fields from the merged previous report — a
// stale `failure` key left on a now-passing gate is exactly the kind of wrong
// evidence this harness exists to avoid.
const freshGates = new Set();
function gateResult(id, patch) {
  const base = freshGates.has(id) ? report.gates[id] : {};
  freshGates.add(id);
  report.gates[id] = { ...base, ...patch, at: new Date().toISOString() };
}
async function flush() {
  report.updatedAt = new Date().toISOString();
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

// --- PNG decode (job 11's decoder, split so regions can be measured too) ----
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
  if (bitDepth !== 8 || (colorType !== 2 && colorType !== 6) || !width || !height || width > 16384 || height > 16384) {
    return { supported: false, reason: 'unsupported-png-format' };
  }
  const channels = colorType === 6 ? 4 : 3;
  const stride = width * channels;
  let raw;
  try { raw = inflateSync(Buffer.concat(chunks)); } catch { return { supported: false, reason: 'invalid-png-deflate' }; }
  const expectedRawLength = height * (stride + 1);
  if (expectedRawLength > 256 * 1024 * 1024 || raw.length < expectedRawLength) return { supported: false, reason: 'truncated-png-pixels' };
  const pixels = Buffer.alloc(height * stride);
  let src = 0;
  const paeth = (a, b, c) => { const p = a + b - c; const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < height; y++) {
    const filter = raw[src++];
    if (filter > 4 || src + stride > raw.length) return { supported: false, reason: 'invalid-png-filter-data' };
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

/** Job 11's non-blank measure, unchanged in behaviour: 8x8 grid RGB variance. */
function pngRgbVariance(buffer) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const { width, height, channels, stride, pixels } = img;
  const values = [];
  for (let row = 0; row < 8; row++) for (let col = 0; col < 8; col++) {
    const x = Math.min(width - 1, Math.floor((col + .5) * width / 8));
    const y = Math.min(height - 1, Math.floor((row + .5) * height / 8));
    const at = y * stride + x * channels;
    values.push(pixels[at], pixels[at + 1], pixels[at + 2]);
  }
  const mean = values.reduce((sum, value) => sum + value, 0) / values.length;
  const variance = values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / values.length;
  return { supported: true, width, height, rgbVariance: variance, alphaIgnored: true };
}

/**
 * Luminance statistics over a fractional region of the frame, plus a count of
 * "bright" pixels. Night evidence needs more than "not blank": an emissive
 * window strip is a small number of very bright pixels on a dark ground, which
 * a mean alone hides.
 */
function regionLuma(buffer, [fx0, fy0, fx1, fy1], brightAt = 190) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const { width, height, channels, stride, pixels } = img;
  const x0 = Math.floor(fx0 * width), x1 = Math.min(width, Math.ceil(fx1 * width));
  const y0 = Math.floor(fy0 * height), y1 = Math.min(height, Math.ceil(fy1 * height));
  let sum = 0, count = 0, bright = 0, max = 0;
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const at = y * stride + x * channels;
      const l = .2126 * pixels[at] + .7152 * pixels[at + 1] + .0722 * pixels[at + 2];
      sum += l; count++;
      if (l > max) max = l;
      if (l >= brightAt) bright++;
    }
  }
  const mean = sum / count;
  // An emissive strip is a handful of pixels far above their own surroundings,
  // which a fixed threshold gets wrong in both directions (day stone is bright;
  // night stone is not). Count pixels at 3x the region's own mean as well.
  let hot = 0;
  const hotAt = Math.max(60, Math.min(235, mean * 3));
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const at = y * stride + x * channels;
      const l = .2126 * pixels[at] + .7152 * pixels[at + 1] + .0722 * pixels[at + 2];
      if (l >= hotAt) hot++;
    }
  }
  return {
    region: [fx0, fy0, fx1, fy1],
    pixels: count,
    meanLuma: Math.round(mean * 100) / 100,
    maxLuma: Math.round(max * 100) / 100,
    brightPixels: bright,
    brightFraction: Math.round((bright / count) * 1e5) / 1e5,
    brightThreshold: brightAt,
    hotPixels: hot,
    hotThreshold: Math.round(hotAt * 100) / 100,
  };
}

/** Mean luminance of a disc in pixel coordinates, and of the annulus around it:
 *  how a projected celestial body reads against the sky immediately behind it. */
function discLuma(buffer, cx, cy, r) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const { width, height, channels, stride, pixels } = img;
  let inSum = 0, inN = 0, outSum = 0, outN = 0, inMax = 0;
  const outer = r * 2.6;
  const x0 = Math.max(0, Math.floor(cx - outer)), x1 = Math.min(width, Math.ceil(cx + outer));
  const y0 = Math.max(0, Math.floor(cy - outer)), y1 = Math.min(height, Math.ceil(cy + outer));
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const d = Math.hypot(x - cx, y - cy);
      const at = y * stride + x * channels;
      const l = .2126 * pixels[at] + .7152 * pixels[at + 1] + .0722 * pixels[at + 2];
      if (d <= r * .8) { inSum += l; inN++; if (l > inMax) inMax = l; }
      else if (d >= r * 1.5 && d <= outer) { outSum += l; outN++; }
    }
  }
  if (!inN || !outN) return { measured: false, reason: 'disc-outside-frame', cx, cy, r };
  const round = n => Math.round(n * 100) / 100;
  return {
    measured: true,
    centre: [Math.round(cx), Math.round(cy)],
    radiusPx: Math.round(r * 10) / 10,
    discMeanLuma: round(inSum / inN),
    discMaxLuma: round(inMax),
    surroundMeanLuma: round(outSum / outN),
    contrastRatio: round((inSum / inN) / Math.max(.5, outSum / outN)),
  };
}

/** Pixels that are BRIGHTER at night than in day from the same camera. An
 *  emissive surface holds its radiance while the lighting drops, so this is the
 *  measure that separates "still lit" from "merely bright". */
function brighterAtNight(dayBuffer, nightBuffer, [fx0, fy0, fx1, fy1], delta = 25) {
  const a = decodePng(dayBuffer), b = decodePng(nightBuffer);
  if (!a.supported) return a;
  if (!b.supported) return b;
  if (a.width !== b.width || a.height !== b.height) return { supported: false, reason: 'size-mismatch' };
  const x0 = Math.floor(fx0 * a.width), x1 = Math.min(a.width, Math.ceil(fx1 * a.width));
  const y0 = Math.floor(fy0 * a.height), y1 = Math.min(a.height, Math.ceil(fy1 * a.height));
  let n = 0, count = 0, sumDelta = 0, maxDelta = 0;
  const luma = (img, at) => .2126 * img.pixels[at] + .7152 * img.pixels[at + 1] + .0722 * img.pixels[at + 2];
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const la = luma(a, y * a.stride + x * a.channels);
      const lb = luma(b, y * b.stride + x * b.channels);
      count++;
      const d = lb - la;
      if (d > maxDelta) maxDelta = d;
      if (d >= delta) { n++; sumDelta += d; }
    }
  }
  return {
    region: [fx0, fy0, fx1, fy1],
    pixelsCompared: count,
    deltaThreshold: delta,
    brighterAtNightPixels: n,
    brighterAtNightFraction: Math.round((n / count) * 1e5) / 1e5,
    meanDeltaWhereBrighter: n ? Math.round((sumDelta / n) * 100) / 100 : 0,
    maxDelta: Math.round(maxDelta * 100) / 100,
  };
}

/** Day-vs-night luminance at a set of exact screen points (the projected
 *  vertices of a named emissive surface). */
function pointLuma(dayBuffer, nightBuffer, points) {
  const a = decodePng(dayBuffer), b = decodePng(nightBuffer);
  if (!a.supported || !b.supported) return { measured: false, reason: 'png-decode' };
  const luma = (img, x, y) => {
    const at = y * img.stride + x * img.channels;
    return .2126 * img.pixels[at] + .7152 * img.pixels[at + 1] + .0722 * img.pixels[at + 2];
  };
  const day = [], night = [];
  for (const p of points) {
    if (p.x < 0 || p.y < 0 || p.x >= a.width || p.y >= a.height) continue;
    day.push(luma(a, p.x, p.y));
    night.push(luma(b, p.x, p.y));
  }
  if (!day.length) return { measured: false, reason: 'no-point-inside-frame' };
  const stat = arr => {
    const s = [...arr].sort((x, y) => x - y);
    const round = n => Math.round(n * 100) / 100;
    return { n: s.length, min: round(s[0]), median: round(s[Math.floor(s.length / 2)]), p90: round(s[Math.floor(s.length * .9)]), max: round(s[s.length - 1]), mean: round(arr.reduce((t, v) => t + v, 0) / arr.length) };
  };
  let brighterAtNight = 0;
  for (let i = 0; i < day.length; i++) if (night[i] > day[i] + 10) brighterAtNight++;
  return { measured: true, points: day.length, day: stat(day), night: stat(night), pointsBrighterAtNight: brighterAtNight };
}

function hasTool(node, tool) {
  if (!node || typeof node !== 'object') return false;
  if (node.type === 'panel') return node.tool === tool;
  return Array.isArray(node.children) && node.children.some(c => hasTool(c, tool));
}

// --- browser session --------------------------------------------------------
const console_ = [];
const responses = [];
const requestFailures = [];
const pageErrors = [];

const gpuArgs = process.env.FORUM_SOFTWARE_GL === '1'
  ? ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
  // --disable-frame-rate-limit/--disable-gpu-vsync: without them Chrome pins the
  // frameloop to the 60 Hz display and every FPS reading is 60.0, which hides
  // both headroom and regressions. Job 11's numbers (77-97 FPS) are uncapped.
  : ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist', '--disable-frame-rate-limit', '--disable-gpu-vsync'];
// Playwright's bundled chromium is not installed in this clone (only webkit and
// ffmpeg are in ~/Library/Caches/ms-playwright), and the bundled build ships as
// `chrome-headless-shell`, which has no GPU path worth measuring. The installed
// Google Chrome does, so the harness drives that channel by default; set
// BROWSER_CHANNEL='' to fall back to the bundled build after
// `npx playwright install chromium`.
const channel = process.env.BROWSER_CHANNEL === undefined ? 'chrome' : (process.env.BROWSER_CHANNEL || undefined);
const browser = await chromium.launch({ headless: true, channel, args: gpuArgs });
report.browser = { channel: channel || 'playwright-bundled-chromium', args: gpuArgs, version: browser.version() };
const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });

// The app's own browser credential path: localStorage['permagent-daemon-token'].
await context.addInitScript(([key, value]) => {
  try { localStorage.setItem(key, value); } catch { /* storage blocked */ }
}, ['permagent-daemon-token', token]);

// Read-only instrumentation. React and react-three-fiber both look for a
// DevTools hook at startup; r3f calls `injectIntoDevTools`, so a stub that
// records committed fiber roots gives us a handle on the r3f tree. Every THREE
// object r3f creates carries `__r3f.root` — the zustand store with the live
// camera, scene and renderer. Nothing is mutated.
await context.addInitScript(() => {
  // The app issues well over the default 250 resource-timing entries before the
  // GLB lands, so without this the GLB's own timing is evicted before we read it.
  try { performance.setResourceTimingBufferSize(3000); } catch { /* not supported */ }
  if (!window.__REACT_DEVTOOLS_GLOBAL_HOOK__) {
    const roots = [];
    window.__probeRoots = roots;
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
      supportsFiber: true,
      isDisabled: false,
      renderers: new Map(),
      inject(renderer) { const id = this.renderers.size + 1; this.renderers.set(id, renderer); return id; },
      onCommitFiberRoot(_id, root) { if (root && !roots.includes(root)) roots.push(root); },
      onPostCommitFiberRoot() {},
      onCommitFiberUnmount() {},
      checkDCE() {},
      on() {}, off() {}, emit() {}, sub() { return () => {}; },
      getFiberRoots() { return new Set(roots); },
    };
  }
  window.__findR3FStore = function findR3FStore() {
    const roots = window.__probeRoots || [];
    let budget = 400000;
    for (const root of roots) {
      const stack = [root.current];
      const seen = new Set();
      while (stack.length && budget-- > 0) {
        const fiber = stack.pop();
        if (!fiber || seen.has(fiber)) continue;
        seen.add(fiber);
        const node = fiber.stateNode;
        const r3f = node && node.__r3f;
        if (r3f && r3f.root && typeof r3f.root.getState === 'function') return r3f.root;
        if (fiber.child) stack.push(fiber.child);
        if (fiber.sibling) stack.push(fiber.sibling);
      }
    }
    return null;
  };
  // Gate geometry, mirrored from src/components/world/forum/meshPortal.ts so
  // the probe reports the same signed distance the product code tests.
  const AXIS = Math.SQRT1_2;
  const CENTER = [-39.597980, 4.77, -39.597980];
  const NORMAL = [-AXIS, 0, -AXIS];
  const TANGENT = [-NORMAL[2], 0, NORMAL[0]];
  window.__forumProbe = function forumProbe() {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const state = store.getState();
    const camera = state.camera;
    if (!camera) return { ok: false, reason: 'no-camera' };
    camera.updateMatrixWorld();
    const e = camera.matrixWorld.elements;
    const pos = { x: e[12], y: e[13], z: e[14] };
    const dir = { x: -e[8], y: -e[9], z: -e[10] };
    const feet = { x: pos.x, y: pos.y - 1.7, z: pos.z };
    const signed = (feet.x - CENTER[0]) * NORMAL[0] + (feet.z - CENTER[2]) * NORMAL[2];
    const lateral = Math.abs((feet.x - CENTER[0]) * TANGENT[0] + (feet.z - CENTER[2]) * TANGENT[2]);
    const round = n => Math.round(n * 1000) / 1000;
    return {
      ok: true,
      camera: { x: round(pos.x), y: round(pos.y), z: round(pos.z) },
      feet: { x: round(feet.x), y: round(feet.y), z: round(feet.z) },
      yaw: round(Math.atan2(-dir.x, -dir.z)),
      pitch: round(Math.asin(Math.max(-1, Math.min(1, dir.y)))),
      pointerLock: document.pointerLockElement ? document.pointerLockElement.tagName : null,
      gate: { signedDistance: round(signed), lateralOffset: round(lateral), polarRadius: round(Math.hypot(feet.x, feet.z)) },
    };
  };
  // Scene census: proves the meshopt payload actually decoded into geometry.
  window.__forumCensus = function forumCensus() {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const scene = store.getState().scene;
    let meshes = 0, triangles = 0, vertices = 0, emissive = 0, named = [];
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry) return;
      meshes++;
      const g = o.geometry;
      const count = g.index ? g.index.count : (g.attributes.position ? g.attributes.position.count : 0);
      triangles += Math.floor(count / 3);
      vertices += g.attributes.position ? g.attributes.position.count : 0;
      const materials = Array.isArray(o.material) ? o.material : [o.material];
      for (const m of materials) {
        if (!m) continue;
        const i = m.emissiveIntensity;
        if (m.emissive && (m.emissive.r || m.emissive.g || m.emissive.b) && (i === undefined || i > 0)) {
          emissive++;
          if (named.length < 12) named.push({ mesh: o.name || '(unnamed)', material: m.name || '(unnamed)', emissiveIntensity: i ?? null });
        }
      }
    });
    return { ok: true, meshes, triangles, vertices, emissiveMaterials: emissive, emissiveSample: named };
  };
  // Where the gas giant and the moon actually land on screen. ForumSky builds
  // them as plain spheres (radius 140 at 900 m, radius 46 at 1080 m) with their
  // own shader, so they are identifiable by geometry alone and their projection
  // can be checked instead of eyeballed.
  window.__celestialProbe = function celestialProbe() {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const state = store.getState();
    const scene = state.scene, camera = state.camera, size = state.size;
    camera.updateMatrixWorld();
    const bodies = [];
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry || o.geometry.type !== 'SphereGeometry') return;
      const r = o.geometry.parameters && o.geometry.parameters.radius;
      const label = r === 140 ? 'gasGiant' : r === 46 ? 'moon' : r === 5.2 ? 'sunriseSun' : null;
      if (!label) return;
      o.updateMatrixWorld();
      const world = { x: o.matrixWorld.elements[12], y: o.matrixWorld.elements[13], z: o.matrixWorld.elements[14] };
      const v = new o.position.constructor(world.x, world.y, world.z);
      v.project(camera);
      const sx = (v.x * .5 + .5) * size.width;
      const sy = (-v.y * .5 + .5) * size.height;
      // Angular radius -> pixels, using the vertical fov the Canvas was given.
      const dx = world.x - camera.position.x, dy = world.y - camera.position.y, dz = world.z - camera.position.z;
      const dist = Math.hypot(dx, dy, dz);
      const halfFov = (camera.fov * Math.PI / 180) / 2;
      const pxPerRad = (size.height / 2) / Math.tan(halfFov);
      const radiusPx = Math.atan(r / dist) * pxPerRad;
      bodies.push({
        body: label, radiusM: r, distanceM: Math.round(dist),
        // The body's LIVE world position, so the harness aims at where the
        // scene actually puts it instead of a constant copied out of the
        // source (job 19: GAS_GIANT_POS moved and the copy went stale).
        world: { x: Math.round(world.x * 10) / 10, y: Math.round(world.y * 10) / 10, z: Math.round(world.z * 10) / 10 },
        screen: { x: Math.round(sx), y: Math.round(sy) }, radiusPx: Math.round(radiusPx * 10) / 10,
        inFrustum: v.z > -1 && v.z < 1 && sx > -radiusPx && sx < size.width + radiusPx && sy > -radiusPx && sy < size.height + radiusPx,
        ndcZ: Math.round(v.z * 1000) / 1000,
      });
    });
    return { ok: true, canvas: { width: size.width, height: size.height }, bodies };
  };
  // Every emissive surface in the scene, with its own projected vertices, so
  // "the emissives read as lit" can be measured per surface instead of guessed
  // from a whole-frame average.
  window.__emissiveProbe = function emissiveProbe() {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const state = store.getState();
    const scene = state.scene, camera = state.camera, size = state.size;
    camera.updateMatrixWorld();
    const out = [];
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry) return;
      const materials = Array.isArray(o.material) ? o.material : [o.material];
      const lit = materials.find(m => m && m.emissive && (m.emissive.r || m.emissive.g || m.emissive.b) && (m.emissiveIntensity === undefined || m.emissiveIntensity > 0));
      if (!lit) return;
      o.updateMatrixWorld();
      if (!o.geometry.boundingBox) o.geometry.computeBoundingBox();
      const bb = o.geometry.boundingBox.clone().applyMatrix4(o.matrixWorld);
      const pos = o.geometry.attributes.position;
      const points = [];
      if (pos) {
        const stride = Math.max(1, Math.floor(pos.count / 20000));
        const p = new o.position.constructor();
        for (let i = 0; i < pos.count && points.length < 300; i += stride) {
          p.set(pos.getX(i), pos.getY(i), pos.getZ(i)).applyMatrix4(o.matrixWorld);
          const q = p.clone(); q.project(camera);
          const px = (q.x * .5 + .5) * size.width, py = (-q.y * .5 + .5) * size.height;
          if (q.z > -1 && q.z < 1 && px >= 4 && px < size.width - 4 && py >= 4 && py < size.height - 4) {
            points.push({ x: Math.round(px), y: Math.round(py) });
          }
        }
      }
      out.push({
        mesh: o.name || '(unnamed)',
        material: lit.name || '(unnamed)',
        emissiveIntensity: lit.emissiveIntensity ?? null,
        vertexCount: pos ? pos.count : 0,
        worldBounds: { min: [Math.round(bb.min.x), Math.round(bb.min.y), Math.round(bb.min.z)], max: [Math.round(bb.max.x), Math.round(bb.max.y), Math.round(bb.max.z)] },
        onScreenVertices: points,
      });
    });
    return { ok: true, canvas: { width: size.width, height: size.height }, count: out.length, meshes: out };
  };
  // Where a named emissive material actually lands on screen. Used for the
  // night window strips ('Forum Warm Window Emission'), so "reads as lit" is a
  // pixel measurement at a known projected position rather than an impression.
  window.__namedMeshProbe = function namedMeshProbe(fragment) {
    const store = window.__findR3FStore();
    if (!store) return { ok: false, reason: 'no-r3f-store' };
    const state = store.getState();
    const scene = state.scene, camera = state.camera, size = state.size;
    camera.updateMatrixWorld();
    const out = [];
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry) return;
      const materials = Array.isArray(o.material) ? o.material : [o.material];
      if (!materials.some(m => m && typeof m.name === 'string' && m.name.toLowerCase().includes(fragment.toLowerCase()))) return;
      o.updateMatrixWorld();
      if (!o.geometry.boundingBox) o.geometry.computeBoundingBox();
      const bb = o.geometry.boundingBox;
      const c = new o.position.constructor((bb.min.x + bb.max.x) / 2, (bb.min.y + bb.max.y) / 2, (bb.min.z + bb.max.z) / 2);
      c.applyMatrix4(o.matrixWorld);
      const size3 = new o.position.constructor(bb.max.x - bb.min.x, bb.max.y - bb.min.y, bb.max.z - bb.min.z);
      const extent = Math.max(size3.x, size3.y, size3.z) * Math.max(o.scale.x, o.scale.y, o.scale.z) / 2;
      const v = c.clone(); v.project(camera);
      const sx = (v.x * .5 + .5) * size.width, sy = (-v.y * .5 + .5) * size.height;
      const dist = c.distanceTo(camera.position);
      const pxPerRad = (size.height / 2) / Math.tan((camera.fov * Math.PI / 180) / 2);
      const radiusPx = Math.max(2, Math.atan(Math.max(extent, .05) / Math.max(dist, .01)) * pxPerRad);
      // The GLB merges by material, so one mesh can span the whole world and its
      // centroid projects nowhere useful. Sample its vertices instead.
      const pos = o.geometry.attributes.position;
      const points = [];
      if (pos) {
        const stride = Math.max(1, Math.floor(pos.count / 20000));
        const p = new o.position.constructor();
        for (let i = 0; i < pos.count && points.length < 400; i += stride) {
          p.set(pos.getX(i), pos.getY(i), pos.getZ(i)).applyMatrix4(o.matrixWorld);
          const q = p.clone(); q.project(camera);
          const px = (q.x * .5 + .5) * size.width, py = (-q.y * .5 + .5) * size.height;
          if (q.z > -1 && q.z < 1 && px >= 4 && px < size.width - 4 && py >= 4 && py < size.height - 4) {
            points.push({ x: Math.round(px), y: Math.round(py), d: Math.round(p.distanceTo(camera.position) * 10) / 10 });
          }
        }
      }
      const worldBox = bb.clone().applyMatrix4(o.matrixWorld);
      out.push({
        mesh: o.name || '(unnamed)',
        worldBounds: {
          min: [Math.round(worldBox.min.x), Math.round(worldBox.min.y), Math.round(worldBox.min.z)],
          max: [Math.round(worldBox.max.x), Math.round(worldBox.max.y), Math.round(worldBox.max.z)],
        },
        vertexCount: pos ? pos.count : 0,
        vertexSamples: points.length,
        onScreenVertices: points.slice(0, 200),
        world: { x: Math.round(c.x * 100) / 100, y: Math.round(c.y * 100) / 100, z: Math.round(c.z * 100) / 100 },
        distanceM: Math.round(dist * 10) / 10,
        screen: { x: Math.round(sx), y: Math.round(sy) },
        radiusPx: Math.round(radiusPx * 10) / 10,
        onScreen: v.z > -1 && v.z < 1 && sx >= 0 && sx < size.width && sy >= 0 && sy < size.height,
      });
    });
    return { ok: true, canvas: { width: size.width, height: size.height }, count: out.length, meshes: out };
  };
});

const page = await context.newPage();
page.setDefaultTimeout(90000);
page.on('pageerror', e => pageErrors.push({ message: e.message }));
page.on('console', m => {
  if (m.type() !== 'error' && m.type() !== 'warning') return;
  console_.push({ type: m.type(), text: m.text().slice(0, 500) });
});
// Redaction, not decoration: `getStreamToken()` puts the master daemon token
// in the SSE query string, and job 11's harness only redacted it on the
// RESPONSE path — a failed/aborted EventSource wrote the live credential into
// the committed report. Every recorded URL goes through the same scrub here.
const scrubUrl = u => u.replace(origin, '').replace(/([?&]token=)[^&]*/g, '$1REDACTED');
page.on('requestfailed', r => requestFailures.push({ url: scrubUrl(r.url()), error: r.failure()?.errorText || 'failed' }));
page.on('response', r => {
  const u = r.url();
  if (/\/(api|sessions|reply|agent|permagent|events|config|status)\b/.test(u) || /solar-forum\.glb/.test(u)) {
    responses.push({ url: scrubUrl(u), status: r.status(), method: r.request().method() });
  }
});

async function forumVisible() {
  return page.evaluate(() => {
    const shells = [...document.querySelectorAll('.forum-shell')];
    const shown = shells.find(s => s.getBoundingClientRect().width > 1);
    if (!shown) return { visible: false, count: shells.length };
    const canvas = shown.querySelector('canvas');
    const box = canvas?.getBoundingClientRect();
    return {
      visible: true,
      count: shells.length,
      canvas: canvas ? { width: Math.round(box.width), height: Math.round(box.height), drawingBuffer: [canvas.width, canvas.height] } : null,
    };
  });
}

async function robustClick(locator, label = '') {
  try {
    await locator.click({ timeout: 15000 });
    return 'normal';
  } catch {
    await locator.evaluate(node => node.scrollIntoView({ block: 'center' })).catch(() => {});
    await locator.click({ force: true, timeout: 15000 });
    return `forced (actionability wait did not settle${label ? ` for ${label}` : ''})`;
  }
}

async function openWorldWorkspace(wsName) {
  const already = await forumVisible();
  if (already.visible) return { clicked: false, ...already };
  await robustClick(page.getByRole('button', { name: wsName, exact: false }).first(), `workspace ${wsName}`);
  await page.waitForFunction(() => [...document.querySelectorAll('.forum-shell')].some(s => s.getBoundingClientRect().width > 1), undefined, { timeout: 90000 });
  return { clicked: true, ...(await forumVisible()) };
}

async function shot(name, opts = {}) {
  const file = `assets/world/forum/browser/${name}.png`;
  await page.screenshot({ path: path.join(repo, file), timeout: 120000, ...opts });
  return file;
}

async function canvasShotBuffer() {
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  if (!box) return null;
  return page.screenshot({ clip: box, timeout: 120000 });
}

/** Sample the shared perf probe repeatedly: one reading is a single second. */
async function samplePerf(seconds = 6) {
  const samples = [];
  await page.evaluate(() => { window.__perfSeen = []; });
  for (let i = 0; i < seconds; i++) {
    await page.waitForTimeout(1050);
    const s = await page.evaluate(() => (window.__worldPerf ? { ...window.__worldPerf } : null));
    if (s) samples.push(s);
  }
  const fps = samples.map(s => s.fps).filter(n => Number.isFinite(n)).sort((a, b) => a - b);
  const last = samples[samples.length - 1] || null;
  return {
    samples,
    fps: fps.length ? { min: fps[0], median: fps[Math.floor(fps.length / 2)], max: fps[fps.length - 1], n: fps.length } : null,
    calls: last?.calls ?? null,
    triangles: last?.triangles ?? null,
    geometries: last?.geometries ?? null,
    textures: last?.textures ?? null,
    programs: last?.programs ?? null,
    dpr: last?.dpr ?? null,
  };
}

const probe = () => page.evaluate(() => window.__forumProbe());
const census = () => page.evaluate(() => window.__forumCensus());
const celestial = () => page.evaluate(() => window.__celestialProbe());
const namedMeshes = frag => page.evaluate(f => window.__namedMeshProbe(f), frag);
const emissiveProbe = () => page.evaluate(() => window.__emissiveProbe());

const workspaces = await (async () => {
  const ws = await daemon('/api/workspaces');
  if (ws.status !== 200 || !Array.isArray(ws.body)) return { error: `GET /api/workspaces -> ${ws.status}` };
  const parsed = ws.body.map(w => ({ id: w.id, name: w.name, layout: typeof w.layoutJson === 'string' ? JSON.parse(w.layoutJson) : w.layoutJson }));
  const world = parsed.find(w => hasTool(w.layout, 'world'));
  const other = parsed.find(w => w.id !== world?.id);
  return { world, other, all: parsed.map(w => w.name) };
})();
report.workspaces = { names: workspaces.all, world: workspaces.world?.name, other: workspaces.other?.name, error: workspaces.error };
await flush();
if (!workspaces.world) {
  report.fatal = `No workspace hosts the 'world' tool (${workspaces.error || 'not found'})`;
  await flush();
  await browser.close();
  console.log(JSON.stringify(report, null, 2));
  process.exit(1);
}

const want = id => only.length === 0 || only.includes(id);

/** Navigate, open the World workspace, and time the GLB. */
async function bootWorld(label) {
  const glbWait = page.waitForResponse(r => r.url().includes('solar-forum.glb'), { timeout: 120000 }).catch(() => null);
  const t0 = Date.now();
  await page.goto(appUrl, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForFunction(() => !document.body.textContent.includes('Loading workspaces'), undefined, { timeout: 60000 }).catch(() => {});
  const opened = await openWorldWorkspace(workspaces.world.name).catch(e => ({ error: String(e.message) }));
  const glb = await glbWait;
  // The forum is "up" when the perf probe has published at least one second.
  await page.waitForFunction(() => Boolean(window.__worldPerf), undefined, { timeout: 120000 }).catch(() => {});
  const readyMs = Date.now() - t0;
  const timing = await page.evaluate(() => {
    const e = performance.getEntriesByType('resource').find(r => r.name.includes('solar-forum.glb'));
    return e ? {
      name: e.name.replace(location.origin, ''),
      durationMs: Math.round(e.duration),
      transferSize: e.transferSize,
      encodedBodySize: e.encodedBodySize,
      decodedBodySize: e.decodedBodySize,
      startTimeMs: Math.round(e.startTime),
    } : null;
  });
  return {
    label,
    navigationToFirstPerfSampleMs: readyMs,
    opened,
    glb: glb ? { url: glb.url().replace(origin, ''), status: glb.status() } : null,
    glbResourceTiming: timing,
  };
}

// --------------------------------------------------------------------- load
if (want('load')) {
  try {
    const boot = await bootWorld('day');
    await page.waitForTimeout(3000);
    const renderer = await page.evaluate(() => {
      try {
        const cv = document.createElement('canvas');
        const gl = cv.getContext('webgl2') || cv.getContext('webgl');
        const d = gl.getExtension('WEBGL_debug_renderer_info');
        return {
          unmasked: d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : 'n/a',
          vendor: d ? gl.getParameter(d.UNMASKED_VENDOR_WEBGL) : 'n/a',
          version: gl.getParameter(gl.VERSION),
        };
      } catch (e) { return { error: String(e && e.message) }; }
    });
    const perf = await samplePerf(6);
    const sceneCensus = await census();
    const buffer = await canvasShotBuffer();
    const file = await shot('v2-world-overview');
    gateResult('load', {
      description: 'the app world tool loads the new meshopt GLB revision and renders it on ANGLE/Metal',
      ...boot,
      expectedRevisionQuery: `?v=${(await readFile(path.join(repo, 'ui/command-center/src/components/world/forum/assetRevision.ts'), 'utf8')).match(/"([0-9a-f]+)"/)[1]}`,
      renderer,
      perf,
      sceneCensus,
      sampling: buffer ? pngRgbVariance(buffer) : { supported: false, reason: 'no-canvas-box' },
      canvasAfterSettle: (await forumVisible()).canvas,
      appearance: await page.locator('[data-testid="forum-appearance"]').first().textContent().catch(() => null),
      districts: await page.locator('.forum-districts').first().innerText().catch(() => null),
      meshoptConsole: console_.filter(c => /meshopt|EXT_meshopt|KHR_mesh_quantization|no DRACO|decoder/i.test(c.text)),
      probe: await probe(),
      screenshot: file,
    });
  } catch (e) { gateResult('load', { failure: String(e && e.message || e) }); }
  await flush();
}

// --------------------------------------------------------------------- mesh
async function enterWalk() {
  await robustClick(page.locator('.forum-play button').first(), 'Walk the world');
  await page.waitForTimeout(1200);
  return page.locator('.forum-controls').first().innerText();
}

/** Turn to a world yaw using the product's own ArrowLeft/ArrowRight look keys
 *  (1.6 rad/s), closing the loop on the live camera rather than guessing. */
async function faceYaw(target, tol = 0.035, maxMs = 8000) {
  const t0 = Date.now();
  let last = null;
  while (Date.now() - t0 < maxMs) {
    const s = await probe();
    if (!s.ok) return { ok: false, reason: s.reason };
    last = s.yaw;
    let d = target - s.yaw;
    while (d > Math.PI) d -= 2 * Math.PI;
    while (d < -Math.PI) d += 2 * Math.PI;
    if (Math.abs(d) < tol) return { ok: true, yaw: s.yaw, error: Math.round(d * 1000) / 1000 };
    const key = d > 0 ? 'ArrowLeft' : 'ArrowRight';
    const ms = Math.min(700, Math.max(25, (Math.abs(d) / 1.6) * 1000 * 0.85));
    await page.keyboard.down(key);
    await page.waitForTimeout(ms);
    await page.keyboard.up(key);
    await page.waitForTimeout(90);
  }
  return { ok: false, reason: 'yaw-not-reached', yaw: last };
}

const agoraOpen = () => page.locator('[data-testid="mesh-plaque"]').count().then(n => n > 0);

/** Hold W and sample the walker until the Agora opens or the walker stalls. */
async function holdForward(maxMs, stallMs = 2500) {
  const path_ = [];
  await page.keyboard.down('KeyW');
  const t0 = Date.now();
  let stalledSince = t0;
  let lastKey = null;
  let opened = false;
  let stalled = false;
  while (Date.now() - t0 < maxMs) {
    await page.waitForTimeout(140);
    const s = await probe();
    if (s.ok) {
      path_.push({ t: Date.now() - t0, feet: s.feet, signed: s.gate.signedDistance, lateral: s.gate.lateralOffset });
      const key = `${s.feet.x.toFixed(2)},${s.feet.y.toFixed(2)},${s.feet.z.toFixed(2)}`;
      if (key !== lastKey) { lastKey = key; stalledSince = Date.now(); }
      else if (Date.now() - stalledSince > stallMs) { stalled = true; break; }
    }
    if (await agoraOpen()) { opened = true; break; }
  }
  await page.keyboard.up('KeyW');
  await page.waitForTimeout(250);
  return { path: path_, opened, stalled, heldMs: Date.now() - t0 };
}

if (want('mesh')) {
  try {
    if (!report.gates.load) await bootWorld('day');
    await openWorldWorkspace(workspaces.world.name).catch(() => {});
    await page.waitForTimeout(1500);

    // 1. district focus: "Mesh gate" moves the overview camera to the approach.
    await robustClick(page.locator('.forum-districts button', { hasText: 'Mesh gate' }).first(), 'Mesh gate district');
    await page.waitForTimeout(2500);
    const districtShot = await shot('v2-mesh-district');
    const location = await page.locator('.forum-location').first().innerText();

    // 2. walk mode from the gate district -> ForumWalk's start is
    //    gateApproachPoint(), 6 m inside the ring on the court axis.
    const controlsWalking = await enterWalk();
    const walkPerf = await samplePerf(5);
    const atStart = await probe();
    const approachShot = await shot('v2-mesh-approach');

    // 3. face the outward walk direction (-0.7071, 0, -0.7071) => yaw +pi/4,
    //    then walk out through the ring.
    const facing = await faceYaw(Math.PI / 4);
    const facedProbe = await probe();
    const outward = await holdForward(20000);
    const afterOut = await probe();
    const plaquePresent = await agoraOpen();
    const plaque = plaquePresent ? await page.locator('[data-testid="mesh-plaque"]').first().innerText() : null;
    const plaqueState = plaquePresent ? await page.locator('[data-testid="mesh-plaque"]').first().getAttribute('data-state') : null;
    const agoraShot = await shot('v2-mesh-agora');
    const agoraPerf = plaquePresent ? await samplePerf(4) : null;
    const agoraLocation = await page.locator('.forum-location').first().innerText();

    // 4. Escape -> back on the spur at the gate approach.
    let escapeProbe = null, afterEscapeShot = null, agoraAfterEscape = null;
    if (plaquePresent) {
      await page.keyboard.press('Escape');
      await page.waitForTimeout(2000);
      agoraAfterEscape = await agoraOpen();
      escapeProbe = await probe();
      afterEscapeShot = await shot('v2-mesh-return');
    }

    // 5. the no-walk path: select the gate district twice to open the Agora,
    //    then leave with the "Return to the Forum" button.
    await robustClick(page.locator('.forum-play button').first(), 'Return to overview').catch(() => {});
    await page.waitForTimeout(1200);
    await robustClick(page.locator('.forum-districts button', { hasText: 'Mesh gate' }).first(), 'Mesh gate district (again)');
    await page.waitForTimeout(1500);
    const agoraByDistrict = await agoraOpen();
    let returnedByButton = null;
    if (agoraByDistrict) {
      await robustClick(page.locator('.forum-agora-return button').first(), 'Return to the Forum');
      await page.waitForTimeout(1800);
      returnedByButton = !(await agoraOpen());
    }

    gateResult('mesh', {
      description: 'MESH portal flow against the real geometry: gate focus, outward ring crossing, Agora, return',
      districtLocation: location,
      districtScreenshot: districtShot,
      controlsWhileWalking: controlsWalking,
      walkModePerf: walkPerf,
      walkerAtGateDistrictStart: atStart,
      facing,
      walkerAfterFacing: facedProbe,
      outwardWalk: {
        heldMs: outward.heldMs,
        stalled: outward.stalled,
        agoraOpened: outward.opened,
        samples: outward.path.length,
        firstSample: outward.path[0] ?? null,
        lastSample: outward.path[outward.path.length - 1] ?? null,
        signedDistanceTrack: outward.path.map(p => p.signed),
        path: outward.path,
      },
      walkerAfterOutwardWalk: afterOut,
      agora: { plaquePresent, plaqueText: plaque, plaqueState, location: agoraLocation, perf: agoraPerf, screenshot: agoraShot },
      escape: { agoraStillOpen: agoraAfterEscape, walkerAfterEscape: escapeProbe, screenshot: afterEscapeShot },
      overviewPath: { agoraOpenedByDistrictReselect: agoraByDistrict, returnedByButton },
      approachScreenshot: approachShot,
    });
  } catch (e) { gateResult('mesh', { failure: String(e && e.message || e), screenshot: await shot('v2-mesh-failure').catch(() => null) }); }
  await flush();
}

// -------------------------------------------------------------------- night
/**
 * Aim the walker's view with the product's own look controls, closing the loop
 * on the live camera. Pitch only exists on the mouse path (`ForumWalk`'s
 * mousemove handler, which also runs while the canvas is dragged), and under
 * pointer lock the browser's movementX/Y are not the pixel deltas Playwright
 * asked for, so the gain is re-learned from what actually happened.
 */
async function aimTo(targetYaw, targetPitch, tol = 0.05, maxSteps = 24) {
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  if (!box) return { ok: false, reason: 'no-canvas-box' };
  const cx = box.x + box.width / 2, cy = box.y + box.height / 2;
  let gain = 1 / 0.002; // px per radian, the handler's nominal sensitivity
  const history = [];
  const yawFirst = await faceYaw(targetYaw, tol);
  for (let i = 0; i < maxSteps; i++) {
    const s = await probe();
    if (!s.ok) return { ok: false, reason: s.reason };
    history.push({ yaw: s.yaw, pitch: s.pitch, gain: Math.round(gain), pointerLock: s.pointerLock });
    const dPitch = targetPitch - s.pitch;
    if (Math.abs(dPitch) < tol) break;
    const dy = Math.max(-260, Math.min(260, -dPitch * gain));
    // Press, drag, release, then re-centre with the button UP: ForumWalk only
    // rotates while the canvas is being dragged (or under pointer lock), so the
    // return trip injects no rotation and the cursor never runs off the canvas.
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx, Math.max(box.y + 8, Math.min(box.y + box.height - 8, cy + dy)), { steps: 1 });
    await page.waitForTimeout(60);
    await page.mouse.up();
    await page.waitForTimeout(60);
    const after = await probe();
    const got = after.ok ? after.pitch - s.pitch : 0;
    if (Math.abs(dy) > 8 && Math.abs(got) > 1e-3) {
      const measured = Math.abs(dy / got);
      if (Number.isFinite(measured) && measured > 1 && measured < 20000) gain = gain * 0.5 + measured * 0.5;
    }
  }
  // The drag can nudge yaw; correct it last, on the keys, which cannot drift.
  const yawFinal = await faceYaw(targetYaw, tol);
  const end = await probe();
  const dYawOk = yawFinal.ok;
  return {
    ok: dYawOk && Math.abs(targetPitch - end.pitch) < tol * 2,
    targetYaw: Math.round(targetYaw * 1000) / 1000,
    targetPitch: Math.round(targetPitch * 1000) / 1000,
    yaw: end.yaw, pitch: end.pitch, pointerLock: end.pointerLock,
    yawPasses: [yawFirst, yawFinal],
    gain: Math.round(gain),
    history: history.slice(-6),
  };
}

/** Deterministic commons eye-level view: walk mode from the commons start puts
 *  the camera at (0, 1.7, 14) looking down -Z, and unmounts ForumNavigation —
 *  so no drei <Html> district chips are painted over the frame and the pixels
 *  measured are the world's own. */
async function commonsWalkFrame(name) {
  await robustClick(page.locator('.forum-districts button', { hasText: 'Commons' }).first(), 'Commons').catch(() => {});
  await page.waitForTimeout(2200);
  const controls = await enterWalk();
  await page.waitForTimeout(2600);
  const walker = await probe();
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  const buffer = await canvasShotBuffer();
  const file = box ? await shot(name, { clip: box }) : null;
  return { controls, walker, buffer, file, box };
}

/** Aim at a celestial body from wherever the walker is standing, then measure
 *  its disc against the sky immediately around it. */
// Fallback only. The aim target is read from the live scene (`__celestialProbe`
// now reports each body's world position); this copy of ForumSky's constants is
// what the harness used to aim by, and it went stale the moment GAS_GIANT_POS
// moved (job 18 fix -> job 19 re-check), which silently aimed the gas-giant
// shot at the opposite side of the sky.
const CELESTIAL_WORLD = (() => {
  const g = Math.hypot(1, 0.35, 1), m = Math.hypot(0.6, 0.55, 1);
  return {
    gasGiant: { x: -900 / g, y: 900 * 0.35 / g, z: -900 / g },
    moon: { x: 1080 * 0.6 / m, y: 1080 * 0.55 / m, z: -1080 / m },
  };
})();

async function bodyWorldPosition(body) {
  const live = await celestial();
  const found = live.ok ? live.bodies.find(b => b.body === body && b.world) : null;
  return found ? { ...found.world, source: 'live-scene' } : { ...CELESTIAL_WORLD[body], source: 'harness-constant' };
}

async function skyShot(body, name) {
  const walker = await probe();
  if (!walker.ok) return { ok: false, reason: walker.reason };
  const w = await bodyWorldPosition(body);
  const dx = w.x - walker.camera.x, dy = w.y - walker.camera.y, dz = w.z - walker.camera.z;
  const aim = await aimTo(Math.atan2(-dx, -dz), Math.atan2(dy, Math.hypot(dx, dz)));
  await page.waitForTimeout(1200);
  const bodies = await celestial();
  const buffer = await canvasShotBuffer();
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  const file = box ? await shot(name, { clip: box }) : null;
  const discs = buffer && bodies.ok
    ? bodies.bodies.map(b => ({ body: b.body, screen: b.screen, radiusPx: b.radiusPx, inFrustum: b.inFrustum, luma: discLuma(buffer, b.screen.x, b.screen.y, b.radiusPx) }))
    : null;
  return {
    ok: true, aimedAt: body, aimTarget: w, from: walker, aim, discs, file,
    contrastRatio: discs?.find(d => d.body === body)?.luma?.contrastRatio ?? null,
    skyBand: buffer ? regionLuma(buffer, [0, 0, 1, .45]) : null,
  };
}

/** Leave walk mode the way the product does, so pointer lock is released and
 *  ordinary DOM clicks reach the HUD again. */
async function leaveWalk() {
  await page.keyboard.press('Escape');
  await page.waitForTimeout(1200);
  return page.locator('.forum-controls').first().innerText().catch(() => null);
}

if (want('night')) {
  try {
    const BAND = [0, .30, 1, .76];
    // ---- day baseline -----------------------------------------------------
    await page.emulateMedia({ colorScheme: 'light' });
    const dayBoot = await bootWorld('day-baseline');
    const dayAppearance = await page.locator('[data-testid="forum-appearance"]').first().textContent();
    await robustClick(page.locator('.forum-districts button', { hasText: 'Commons' }).first(), 'Commons').catch(() => {});
    await page.waitForTimeout(3000);
    const dayOverviewPerf = await samplePerf(5);
    const dayOverviewBuffer = await canvasShotBuffer();
    const dayWalk = await commonsWalkFrame('v2-commons-day-walk');
    const dayWalkPerf = await samplePerf(5);

    // ---- night ------------------------------------------------------------
    await page.emulateMedia({ colorScheme: 'dark' });
    const nightBoot = await bootWorld('night');
    const nightAppearance = await page.locator('[data-testid="forum-appearance"]').first().textContent();
    await robustClick(page.locator('.forum-districts button', { hasText: 'Commons' }).first(), 'Commons').catch(() => {});
    await page.waitForTimeout(3500);
    const nightOverviewPerf = await samplePerf(6);
    const nightOverviewBuffer = await canvasShotBuffer();
    const nightShot = await shot('v2-world-night');
    const nightWalk = await commonsWalkFrame('v2-commons-night-walk');
    const nightWalkPerf = await samplePerf(5);

    // ---- the emissive window strips ---------------------------------------
    // Where the 'Forum Warm Window Emission' meshes land on screen from this
    // exact camera, so "reads as lit" is a sampled disc, not an impression.
    // Two vantages: the overview (the commons overlook, which is what the task
    // means by "from the commons overlook") and eye level in walk mode.
    await leaveWalk();
    await robustClick(page.locator('.forum-districts button', { hasText: 'Commons' }).first(), 'Commons').catch(() => {});
    await page.waitForTimeout(2500);
    const stripsFromOverlook = await namedMeshes('Warm Window Emission');
    const overlookPoints = stripsFromOverlook.ok ? stripsFromOverlook.meshes.flatMap(m => m.onScreenVertices || []) : [];
    const emissivesFromOverlook = await emissiveProbe();
    const nightOverviewBuffer2 = await canvasShotBuffer();
    const overlookLuma = (dayOverviewBuffer && nightOverviewBuffer2 && overlookPoints.length)
      ? pointLuma(dayOverviewBuffer, nightOverviewBuffer2, overlookPoints)
      : { measured: false, reason: overlookPoints.length ? 'no-frame' : 'no-window-strip-vertex-projects-into-the-overlook-view' };
    const overlookDiff = (dayOverviewBuffer && nightOverviewBuffer2)
      ? brighterAtNight(dayOverviewBuffer, nightOverviewBuffer2, [0, .30, 1, .76])
      : null;
    await enterWalk();
    await page.waitForTimeout(2600);
    const windowStrips = await namedMeshes('Warm Window Emission');
    const stripPoints = windowStrips.ok ? windowStrips.meshes.flatMap(m => m.onScreenVertices || []) : [];
    const stripLuma = (dayWalk.buffer && nightWalk.buffer && stripPoints.length)
      ? pointLuma(dayWalk.buffer, nightWalk.buffer, stripPoints)
      : { measured: false, reason: stripPoints.length ? 'no-frame' : 'no-window-strip-vertex-projects-into-this-view' };

    // ---- the celestial bodies ---------------------------------------------
    // The moon sits about 25 degrees up on a north-north-easterly bearing; the
    // gas giant is only ~14 degrees up on the same north-west axis as the gate,
    // which the new ridge can close off from ground level.
    // ---- go and look at the warm window strips themselves -----------------
    // They are 5 small linked boxes on the market hall (Blender
    // scripts/blender/forum_towers.py:373-376), not a city-wide set, so the
    // only honest way to show them lit is to stand where they are.
    let stripSite = null;
    const stripBounds = windowStrips.ok && windowStrips.meshes[0] ? windowStrips.meshes[0].worldBounds : null;
    if (stripBounds) {
      const centre = {
        x: (stripBounds.min[0] + stripBounds.max[0]) / 2,
        y: (stripBounds.min[1] + stripBounds.max[1]) / 2,
        z: (stripBounds.min[2] + stripBounds.max[2]) / 2,
      };
      await leaveWalk();
      await robustClick(page.locator('.forum-districts button', { hasText: 'Maker hall' }).first(), 'Maker hall').catch(() => {});
      await page.waitForTimeout(2200);
      await enterWalk();
      await page.waitForTimeout(2200);
      const from = await probe();
      const dx = centre.x - from.camera.x, dy = centre.y - from.camera.y, dz = centre.z - from.camera.z;
      const aim = await aimTo(Math.atan2(-dx, -dz), Math.atan2(dy, Math.hypot(dx, dz)));
      await page.waitForTimeout(1200);
      const here = await namedMeshes('Warm Window Emission');
      const pts = here.ok ? here.meshes.flatMap(m => m.onScreenVertices || []) : [];
      const buffer = await canvasShotBuffer();
      const box = await page.locator('.forum-shell canvas').first().boundingBox();
      const file = box ? await shot('v2-night-window-strips', { clip: box }) : null;
      let disc = null;
      if (buffer && pts.length) {
        const cx = pts.reduce((t, p) => t + p.x, 0) / pts.length;
        const cy = pts.reduce((t, p) => t + p.y, 0) / pts.length;
        const r = Math.max(8, Math.max(...pts.map(p => Math.hypot(p.x - cx, p.y - cy))));
        disc = discLuma(buffer, cx, cy, r);
      }
      stripSite = { walkerAt: from.feet, distanceM: Math.round(Math.hypot(dx, dy, dz) * 10) / 10, aim, verticesOnScreen: pts.length, strips: disc, screenshot: file };
      await leaveWalk();
      await robustClick(page.locator('.forum-districts button', { hasText: 'Commons' }).first(), 'Commons').catch(() => {});
      await page.waitForTimeout(2000);
      await enterWalk();
      await page.waitForTimeout(2200);
    }

    const moonShot = await skyShot('moon', 'v2-world-night-sky');
    const giantShot = await skyShot('gasGiant', 'v2-world-night-gasgiant');
    // The commons sits inside the new ridge. The gate court is the forum's
    // north-west edge, on the gas giant's own bearing, so try there too before
    // concluding anything about whether the planet can be seen at all.
    await leaveWalk();
    await robustClick(page.locator('.forum-districts button', { hasText: 'Mesh gate' }).first(), 'Mesh gate').catch(() => {});
    await page.waitForTimeout(2200);
    await enterWalk();
    await page.waitForTimeout(2000);
    const giantFromGate = await skyShot('gasGiant', 'v2-world-night-gasgiant-gate');
    await leaveWalk();

    gateResult('night', {
      description: 'prefers-color-scheme: dark gives the night sky, the celestial bodies and lit emissive strips',
      dayAppearanceLabel: dayAppearance,
      nightAppearanceLabel: nightAppearance,
      dayBoot,
      nightBoot,
      perf: {
        note: 'Overview mounts ForumNavigation (OrbitControls + 14 drei <Html> district chips); walk mode does not.',
        dayOverview: { fps: dayOverviewPerf.fps, calls: dayOverviewPerf.calls, triangles: dayOverviewPerf.triangles },
        nightOverview: { fps: nightOverviewPerf.fps, calls: nightOverviewPerf.calls, triangles: nightOverviewPerf.triangles },
        dayWalk: { fps: dayWalkPerf.fps, calls: dayWalkPerf.calls, triangles: dayWalkPerf.triangles },
        nightWalk: { fps: nightWalkPerf.fps, calls: nightWalkPerf.calls, triangles: nightWalkPerf.triangles },
      },
      bloomCostAtNight: dayOverviewPerf.fps && nightOverviewPerf.fps ? {
        overviewDeltaFps: Math.round((nightOverviewPerf.fps.median - dayOverviewPerf.fps.median) * 10) / 10,
        walkDeltaFps: dayWalkPerf.fps && nightWalkPerf.fps ? Math.round((nightWalkPerf.fps.median - dayWalkPerf.fps.median) * 10) / 10 : null,
        extraDrawCallsAtNight: (nightOverviewPerf.calls ?? 0) - (dayOverviewPerf.calls ?? 0),
      } : null,
      emissives: {
        note: 'Identical walk-mode camera (0, 1.7, 14) looking down -Z from the commons, day vs night. Walk mode paints no district chips over the frame, so these are the world\'s own pixels.',
        dayWalkCamera: dayWalk.walker,
        nightWalkCamera: nightWalk.walker,
        dayBand: dayWalk.buffer ? regionLuma(dayWalk.buffer, BAND) : null,
        nightBand: nightWalk.buffer ? regionLuma(nightWalk.buffer, BAND) : null,
        brighterAtNight: dayWalk.buffer && nightWalk.buffer ? brighterAtNight(dayWalk.buffer, nightWalk.buffer, [0, .05, 1, .95]) : null,
        windowStripMeshes: windowStrips.ok
          ? { meshesWithThatMaterial: windowStrips.count, worldBounds: windowStrips.meshes.map(m => m.worldBounds), vertexCount: windowStrips.meshes.map(m => m.vertexCount), verticesProjectingIntoTheWalkView: stripPoints.length }
          : windowStrips,
        windowStripPixelsAtEyeLevel: stripLuma,
        emissiveSurfacesFromTheOverlook: (() => {
          if (!emissivesFromOverlook.ok || !dayOverviewBuffer || !nightOverviewBuffer2) return emissivesFromOverlook;
          const visible = emissivesFromOverlook.meshes.filter(m => m.onScreenVertices.length);
          const rows = visible
            .map(m => ({ mesh: m.mesh, material: m.material, emissiveIntensity: m.emissiveIntensity, worldBounds: m.worldBounds, pixels: pointLuma(dayOverviewBuffer, nightOverviewBuffer2, m.onScreenVertices) }))
            .sort((a, b) => (b.pixels.points || 0) - (a.pixels.points || 0));
          const all = visible.flatMap(m => m.onScreenVertices);
          return {
            note: 'Every mesh whose material has a non-black emissive, projected into the overlook frame and sampled in the day and night renders of that same frame.',
            emissiveMeshesInScene: emissivesFromOverlook.count,
            emissiveMeshesProjectingIntoView: visible.length,
            allEmissivePixels: pointLuma(dayOverviewBuffer, nightOverviewBuffer2, all),
            perSurface: rows.slice(0, 14),
          };
        })(),
        warmWindowStripsInSitu: stripSite,
        fromTheCommonsOverlook: {
          note: 'Default overview camera at the commons: the raised vantage the task calls the overlook. Same camera in day and night.',
          verticesProjectingIntoView: overlookPoints.length,
          windowStripPixels: overlookLuma,
          brighterAtNightInArchitectureBand: overlookDiff,
        },
        dayFrame: dayWalk.file,
        nightFrame: nightWalk.file,
      },
      nightOverview: {
        sampling: nightOverviewBuffer ? pngRgbVariance(nightOverviewBuffer) : null,
        band: nightOverviewBuffer ? regionLuma(nightOverviewBuffer, BAND) : null,
        screenshot: nightShot,
      },
      celestial: {
        note: 'Both bodies exist in the scene in day and night alike (ForumSky mounts CelestialBodies outside the appearance branch); these are night measurements from the commons in walk mode.',
        moon: moonShot,
        gasGiantFromCommons: giantShot,
        gasGiantFromGateCourt: giantFromGate,
      },
      sceneCensus: await census(),
    });
    await page.emulateMedia({ colorScheme: 'light' });
  } catch (e) { gateResult('night', { failure: String(e && e.message || e), screenshot: await shot('v2-night-failure').catch(() => null) }); }
  await flush();
}

// ----------------------------------------------------- job-11 regression set
let lastHitTest = null;
const obstructions = [];

async function hitTest(selector, expectText) {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) return { selector, error: 'no box' };
  const hit = await page.evaluate(([x, y]) => {
    const el = document.elementFromPoint(x, y);
    if (!el) return null;
    return { tag: el.tagName, id: el.id, class: String(el.className).slice(0, 120), text: (el.closest('button') || el).textContent?.slice(0, 60) };
  }, [box.x + box.width / 2, box.y + box.height / 2]);
  const ok = hit && (hit.id === selector.replace('#', '') || (expectText && String(hit.text || '').includes(expectText)));
  const result = { selector, box, hit, obstructed: !ok };
  if (!ok) obstructions.push(result);
  return result;
}

async function safeFill(selector, value) {
  const el = page.locator(selector).first();
  await el.evaluate(node => node.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(500);
  let probeResult = await hitTest(selector);
  if (probeResult.obstructed) {
    await page.locator(selector).first().evaluate(node => node.scrollIntoView({ block: 'center' }));
    await page.waitForTimeout(400);
    probeResult = await hitTest(selector);
  }
  try {
    await el.fill(value, { timeout: 12000 });
  } catch (e) {
    await el.evaluate((node, v) => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
      setter.call(node, v);
      node.dispatchEvent(new Event('input', { bubbles: true }));
    }, value);
    probeResult.filledViaEventDispatch = String(e.message).slice(0, 120);
  }
  return probeResult;
}

async function sendBrief(kind, text, criteria = '') {
  await robustClick(page.getByRole('button', { name: 'Give a brief' }).first(), 'Give a brief tab');
  await page.locator('#forum-kind').selectOption(kind);
  await safeFill('#forum-brief', text);
  if (criteria) await safeFill('#forum-criteria', criteria);
  const button = page.locator('.forum-panel button', { hasText: kind === 'Query' ? /^Ask / : kind === 'Job' ? /^Ask to run job/ : /^Develop capability/ }).first();
  const label = await button.textContent();
  await button.evaluate(node => node.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(600);
  const box = await button.boundingBox();
  const hit = await page.evaluate(([x, y]) => {
    const el = document.elementFromPoint(x, y);
    if (!el) return null;
    const btn = el.closest('button');
    return { tag: el.tagName, class: String(el.className).slice(0, 120), text: (btn || el).textContent?.slice(0, 60) };
  }, [box.x + box.width / 2, box.y + box.height / 2]);
  const obstructed = !hit || !String(hit.text || '').startsWith(label.trim().slice(0, 6));
  lastHitTest = { label, box, hit, obstructed, disabled: await button.isDisabled() };
  if (obstructed) obstructions.push(lastHitTest);
  try {
    await button.click({ timeout: 20000 });
    lastHitTest.clickMode = 'normal';
  } catch (e) {
    lastHitTest.clickMode = 'forced-after-actionability-timeout';
    lastHitTest.actionabilityError = String(e.message).split('\n')[0];
    await button.click({ force: true, timeout: 20000 });
  }
  return label;
}

if (want('ask')) {
  try {
    await bootWorld('ask');
    await page.waitForTimeout(2000);
    const before = responses.length;
    const label = await sendBrief('Query', `${TAG} reply with the single word PONG`, 'a one-word reply');
    await page.waitForFunction(() => Boolean(document.querySelector('.forum-transcript')), undefined, { timeout: 60000 });
    // job 11 waited for /PONG/i anywhere in the transcript — which the echoed
    // prompt ("reply with the single word PONG") satisfies on its own, so the
    // gate passed even when the agent never answered. Look only at what comes
    // AFTER the prompt.
    const afterPrompt = () => page.evaluate(() => {
      const text = document.querySelector('.forum-transcript')?.innerText || '';
      const at = text.lastIndexOf('reply with the single word PONG');
      return at < 0 ? '' : text.slice(at + 'reply with the single word PONG'.length);
    });
    let replied = false;
    try {
      await page.waitForFunction(() => {
        const text = document.querySelector('.forum-transcript')?.innerText || '';
        const at = text.lastIndexOf('reply with the single word PONG');
        return at >= 0 && /\bPONG\b/i.test(text.slice(at + 31));
      }, undefined, { timeout: 180000 });
      replied = true;
    } catch { replied = false; }
    await page.waitForFunction(() => !/reply in progress/.test(document.querySelector('.forum-conversation .eyebrow')?.textContent || ''), undefined, { timeout: 120000 }).catch(() => {});
    const transcript = await page.locator('.forum-transcript').innerText();
    const reply = await afterPrompt();
    gateResult('ask', {
      description: 'forum ask control streams a real reply into the existing conversation (job-11 gate, re-run)',
      agentRepliedPong: replied,
      replyAfterPrompt: reply.trim().slice(0, 400),
      replyLooksLikeTransportError: /network error|could not connect|ECONNREFUSED/i.test(reply),
      buttonLabel: label,
      hitTest: lastHitTest,
      connection: await page.locator('.forum-conversation .eyebrow').innerText(),
      transcriptTail: transcript.slice(-700),
      network: responses.slice(before).filter(r => /\/sessions|\/reply/.test(r.url)).slice(0, 12),
      screenshot: await shot('v2-ask'),
    });
  } catch (e) {
    gateResult('ask', { failure: String(e && e.message || e), transcriptTail: await page.locator('.forum-transcript').innerText().catch(() => null), screenshot: await shot('v2-ask-failure').catch(() => null) });
  }
  await flush();
}

if (want('skills')) {
  try {
    await openWorldWorkspace(workspaces.world.name).catch(() => {});
    await page.waitForTimeout(1500);
    const apiSkills = await daemon('/permagent/skills');
    const forumText = await page.locator('section[aria-label="Live capabilities"]').first().innerText();
    const savedCount = /(\d+)\s+saved skills/.exec(forumText)?.[1] ?? null;
    const workerCount = /(\d+)\s+registered workers/.exec(forumText)?.[1] ?? null;
    await robustClick(page.locator('section[aria-label="Live capabilities"] button', { hasText: 'Open Skills' }).first(), 'Open Skills');
    await page.waitForTimeout(2500);
    const panelOpen = await page.evaluate(() => /skills/i.test(document.body.innerText) && !document.querySelector('.forum-shell')?.getBoundingClientRect().width);
    const panelShot = await shot('v2-skills-panel');
    gateResult('skills', {
      description: 'saved-skills list matches /permagent/skills and Open Skills reaches the app Skills panel (job-11 gate, re-run)',
      apiStatus: apiSkills.status,
      apiSkillCount: Array.isArray(apiSkills.body) ? apiSkills.body.length : null,
      forumSavedSkills: savedCount,
      forumRegisteredWorkers: workerCount,
      matches: savedCount !== null && Array.isArray(apiSkills.body) ? Number(savedCount) === apiSkills.body.length : null,
      skillsPanelOpened: panelOpen,
      skillsPanelText: await page.evaluate(() => document.body.innerText.slice(0, 600)),
      screenshot: panelShot,
    });
    await robustClick(page.getByRole('button', { name: workspaces.world.name, exact: false }).first(), 'World tab');
    await page.waitForTimeout(2500);
  } catch (e) { gateResult('skills', { failure: String(e && e.message || e), screenshot: await shot('v2-skills-failure').catch(() => null) }); }
  await flush();
}

if (want('remount')) {
  try {
    await openWorldWorkspace(workspaces.world.name).catch(() => {});
    await page.waitForTimeout(2000);
    const controlsWalking = await enterWalk();
    const walkerBefore = await probe();
    await page.evaluate(() => {
      window.__gateTicks = 0;
      let last = window.__worldPerf;
      clearInterval(window.__gateTimer);
      window.__gateTimer = setInterval(() => { if (window.__worldPerf !== last) { last = window.__worldPerf; window.__gateTicks++; } }, 100);
    });
    await page.waitForTimeout(3500);
    const ticksVisible = await page.evaluate(() => window.__gateTicks);

    // The sidebar cannot be clicked while walking: walk mode holds pointer lock,
    // so the pointer never reaches the DOM. The app's own Cmd+N workspace
    // shortcut does reach it, and ForumWalk explicitly ignores keys with a
    // modifier (ForumWalk.tsx:58), which is exactly the path a real user has.
    const otherName = workspaces.other?.name;
    const shortcutFor = name => {
      const at = (workspaces.all || []).indexOf(name);
      return at >= 0 && at < 9 ? `Meta+${at + 1}` : null;
    };
    let switched = null, switchMethod = null;
    const pointerLockedWhileWalking = (await probe()).pointerLock;
    if (otherName) {
      const key = shortcutFor(otherName);
      if (key) {
        await page.keyboard.press(key);
        await page.waitForTimeout(2500);
        switchMethod = `keyboard ${key}`;
      }
      const stillShowing = await page.evaluate(() => [...document.querySelectorAll('.forum-shell')].some(s => s.getBoundingClientRect().width > 1));
      if (stillShowing) {
        await robustClick(page.getByRole('button', { name: otherName, exact: false }).first(), `workspace ${otherName}`);
        await page.waitForTimeout(2500);
        switchMethod = `${switchMethod ? switchMethod + ' then ' : ''}sidebar click`;
      }
      switched = otherName;
    }
    const hiddenState = await page.evaluate(() => ({
      forumBox: [...document.querySelectorAll('.forum-shell')].map(s => { const b = s.getBoundingClientRect(); return [Math.round(b.width), Math.round(b.height)]; }),
      pointerLock: document.pointerLockElement ? document.pointerLockElement.tagName : null,
      controls: [...document.querySelectorAll('.forum-controls')].map(c => c.textContent),
    }));
    const walkerWhenHidden = await probe();
    await page.evaluate(() => { window.__gateTicks = 0; });
    for (const key of ['w', 'a', 's', 'd', 'w', 'w']) { await page.keyboard.press(key); await page.waitForTimeout(120); }
    await page.waitForTimeout(3000);
    const ticksHidden = await page.evaluate(() => window.__gateTicks);
    const walkerAfterHiddenWasd = await probe();
    const awayShot = await shot('v2-other-workspace');

    const backKey = shortcutFor(workspaces.world.name);
    if (backKey) { await page.keyboard.press(backKey); await page.waitForTimeout(2500); }
    if (!(await forumVisible()).visible) {
      await robustClick(page.getByRole('button', { name: workspaces.world.name, exact: false }).first(), 'World tab');
    }
    await page.waitForTimeout(4000);
    await page.evaluate(() => { window.__gateTicks = 0; });
    await page.waitForTimeout(3500);
    const ticksBack = await page.evaluate(() => window.__gateTicks);
    const backVisible = await forumVisible();
    const buffer = await canvasShotBuffer();
    gateResult('remount', {
      description: 'switch away and back: hidden panel stops rendering, WASD does not leak, forum keeps rendering (job-11 gate, re-run)',
      controlsWhileWalking: controlsWalking,
      walkerBefore,
      perfTicksWhileVisible: ticksVisible,
      switchedTo: switched,
      switchMethod,
      pointerLockedWhileWalking,
      hiddenState,
      walkerWhenHidden,
      walkerAfterHiddenWasd,
      cameraMovedWhileHidden: walkerWhenHidden.ok && walkerAfterHiddenWasd.ok
        ? Math.round(Math.hypot(walkerAfterHiddenWasd.camera.x - walkerWhenHidden.camera.x, walkerAfterHiddenWasd.camera.z - walkerWhenHidden.camera.z) * 1000) / 1000
        : null,
      perfTicksWhileHiddenAfterWASD: ticksHidden,
      perfTicksAfterReturn: ticksBack,
      canvasAfterReturn: backVisible,
      samplingAfterReturn: buffer ? pngRgbVariance(buffer) : null,
      perfAfterReturn: await samplePerf(4),
      screenshots: [awayShot, await shot('v2-world-return')],
    });
  } catch (e) { gateResult('remount', { failure: String(e && e.message || e), screenshot: await shot('v2-remount-failure').catch(() => null) }); }
  await flush();
}

report.obstructions = obstructions;
report.console = console_.slice(0, 60);
report.consoleErrorCount = console_.filter(c => c.type === 'error').length;
// The raw list is dominated by repeats; group the whole run, not the first 60.
report.consoleSummary = Object.entries(console_.reduce((acc, c) => {
  const key = `${c.type}: ${c.text.slice(0, 110)}`;
  acc[key] = (acc[key] || 0) + 1;
  return acc;
}, {})).sort((a, b) => b[1] - a[1]).map(([text, count]) => ({ count, text }));
report.pageErrorSummary = Object.entries(pageErrors.reduce((acc, e) => {
  acc[e.message.slice(0, 120)] = (acc[e.message.slice(0, 120)] || 0) + 1;
  return acc;
}, {})).sort((a, b) => b[1] - a[1]).map(([message, count]) => ({ count, message }));
report.pageErrors = pageErrors;
report.requestFailures = requestFailures.slice(0, 40);
report.daemonHttpFailures = responses.filter(r => r.status >= 400);
report.glbResponses = responses.filter(r => /solar-forum\.glb/.test(r.url));
report.finishedAt = new Date().toISOString();
await flush();
// Teardown must not be able to hold the run open: observed on this machine,
// the browser went away mid-close and node then sat on a handle forever with
// the report already written.
await Promise.race([
  (async () => {
    await page.close().catch(() => {});
    await context.close().catch(() => {});
    await browser.close().catch(() => {});
  })(),
  new Promise(resolve => setTimeout(resolve, 20000)),
]);
console.log(JSON.stringify({
  report: path.relative(repo, reportPath),
  gates: Object.fromEntries(Object.entries(report.gates).map(([k, v]) => [k, v.failure ? `FAILURE: ${v.failure}` : 'recorded'])),
}, null, 2));
// Observed once on this machine: the browser process went away while Playwright
// was still tearing the context down, and node then sat on a handle that never
// settled — with the whole report already written. Exit on the evidence, not on
// the transport's good manners.
process.exit(0);
