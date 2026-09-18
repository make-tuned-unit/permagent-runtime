#!/usr/bin/env node
/*
 * Night re-check (job 19) — the checks job 18's "Fixes after v2 verification"
 * asks for that `verify-world-in-app-v2.mjs` does not already make.
 *
 * v2 measures: night contrast from the commons and the gate court, the
 * emissive population, day/night FPS, and the MESH walk. Run that first; this
 * script adds only what is missing:
 *
 *   sky      disc/surround contrast for BOTH bodies in DAY as well as night,
 *            from the commons overlook, the gate approach and the harbour,
 *            with the day and night frames taken from the IDENTICAL camera
 *            (`prefers-color-scheme` is a live matchMedia subscription in
 *            `useSystemAppearance.ts`, so the appearance flips without a
 *            reload and the pose cannot drift between the pair).
 *   occlude  a raycast from the live camera towards each body, reporting the
 *            first real mesh on that line — evidence that the world occludes
 *            a body when it should, rather than a body being overpainted.
 *   stars    isolated bright spikes inside the moon's disc versus the same
 *            detector run on the sky annulus around it: a starfield punching
 *            through the disc shows up as spikes on both, a disc that writes
 *            depth shows spikes only outside.
 *   windows  where the 'Forum Warm Window Emission' boxes actually are (the
 *            export now merges them into one mesh spanning the world, so the
 *            centroid is useless), clustered; then day/night pixels at the
 *            same camera from the commons overlook and a garden vantage.
 *   shadows  gl.shadowMap type/enabled read straight off the live renderer,
 *            plus a shadows-on/shadows-off pair of frames from the same camera
 *            so "shadows still render" is a measured delta, not a claim.
 *
 * Read-only with respect to `src/`: the only page mutation is the deliberate
 * shadow toggle in the `shadows` gate, which is restored before the frame is
 * left. Credentials come from the daemon secrets file and are never printed.
 *
 * Usage (from ui/command-center, dev server on 5284):
 *   node scripts/verify-world-night-v3.mjs
 *   GATES=sky,windows node scripts/verify-world-night-v3.mjs
 */
import { chromium } from 'playwright';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { inflateSync, deflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const appUrl = process.env.APP_URL || 'http://127.0.0.1:5284/ui/';
const origin = new URL(appUrl).origin;
const shotDir = path.join(repo, 'assets/world/forum/browser');
const reportPath = path.join(repo, 'docs/design/solar-forum/night-recheck-v3.json');
const tokenFile = process.env.PERMAGENT_TOKEN_FILE || path.join(os.homedir(), '.permagent/secrets/daemon_token.json');
const only = (process.env.GATES || '').split(',').map(s => s.trim()).filter(Boolean);
const viewport = { width: Number(process.env.VIEW_W || 1280), height: Number(process.env.VIEW_H || 1000) };

await mkdir(shotDir, { recursive: true });
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

const report = {
  appUrl, viewport,
  startedAt: new Date().toISOString(),
  tokenSource: `${tokenFile} (value never recorded)`,
  notes: [
    'Companion to verify-world-in-app-v2.mjs; only the checks job 18 left for a browser.',
    'Day/night pairs share one camera: the appearance is a live matchMedia subscription, so no reload happens between them.',
  ],
  gates: {},
};
// A subset run (GATES=...) must not throw away what an earlier pass recorded:
// merge whatever is on disk and replace only the gates this run redoes.
if (only.length) {
  try {
    const previous = JSON.parse(await readFile(reportPath, 'utf8'));
    if (previous && previous.gates) {
      report.gates = previous.gates;
      report.mergedFromPreviousRun = { startedAt: previous.startedAt, gates: Object.keys(previous.gates), replaced: only };
    }
  } catch { /* no previous report */ }
}
function gateResult(id, patch) { report.gates[id] = { ...patch, at: new Date().toISOString() }; }
async function flush() {
  report.updatedAt = new Date().toISOString();
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

// --- PNG decode and pixel measures (job 11/v2 code, unchanged behaviour) ----
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
const lumaAt = (img, x, y) => {
  const at = y * img.stride + x * img.channels;
  return .2126 * img.pixels[at] + .7152 * img.pixels[at + 1] + .0722 * img.pixels[at + 2];
};
const round2 = n => Math.round(n * 100) / 100;

/**
 * Disc against the sky immediately around it. Two contaminants that job 18's
 * numbers contain and this one excludes:
 *  - the walk-mode crosshair (`forum.css:361`, `--forum-accent` #00D5FF, luma
 *    170.75) sits at the exact canvas centre, which is where an aimed body
 *    lands — it is the `discMaxLuma: 170.75` that recurs all through the v2
 *    report. Pixels within 12 px of the canvas centre are dropped.
 *  - medians alongside means, so one stray pixel cannot carry a reading.
 */
function discLuma(buffer, cx, cy, r, canvasCentre = null) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const inside = [], outside = [];
  const outer = r * 2.6;
  let crosshairDropped = 0;
  const ccx = canvasCentre ? canvasCentre[0] : img.width / 2, ccy = canvasCentre ? canvasCentre[1] : img.height / 2;
  for (let y = Math.max(0, Math.floor(cy - outer)); y < Math.min(img.height, Math.ceil(cy + outer)); y++) {
    for (let x = Math.max(0, Math.floor(cx - outer)); x < Math.min(img.width, Math.ceil(cx + outer)); x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d > r * .8 && (d < r * 1.5 || d > outer)) continue;
      if (Math.hypot(x - ccx, y - ccy) <= 12) { crosshairDropped++; continue; }
      (d <= r * .8 ? inside : outside).push(lumaAt(img, x, y));
    }
  }
  if (!inside.length || !outside.length) return { measured: false, reason: 'disc-outside-frame', cx: Math.round(cx), cy: Math.round(cy), r };
  const mean = a => a.reduce((t, v) => t + v, 0) / a.length;
  const med = a => { const s = [...a].sort((p, q) => p - q); return s[Math.floor(s.length / 2)]; };
  return {
    measured: true, centre: [Math.round(cx), Math.round(cy)], radiusPx: Math.round(r * 10) / 10,
    discPixels: inside.length, crosshairPixelsDropped: crosshairDropped,
    discMeanLuma: round2(mean(inside)), discMedianLuma: round2(med(inside)),
    discMinLuma: round2(Math.min(...inside)), discMaxLuma: round2(Math.max(...inside)),
    surroundMeanLuma: round2(mean(outside)), surroundMedianLuma: round2(med(outside)),
    contrastRatio: round2(mean(inside) / Math.max(.5, mean(outside))),
    medianContrastRatio: round2(med(inside) / Math.max(.5, med(outside))),
  };
}

function regionLuma(buffer, [fx0, fy0, fx1, fy1]) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const x0 = Math.floor(fx0 * img.width), x1 = Math.min(img.width, Math.ceil(fx1 * img.width));
  const y0 = Math.floor(fy0 * img.height), y1 = Math.min(img.height, Math.ceil(fy1 * img.height));
  let sum = 0, count = 0, max = 0, min = 255;
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const l = lumaAt(img, x, y); sum += l; count++;
    if (l > max) max = l; if (l < min) min = l;
  }
  return { region: [fx0, fy0, fx1, fy1], pixels: count, meanLuma: round2(sum / count), minLuma: round2(min), maxLuma: round2(max) };
}

function pointLuma(dayBuffer, nightBuffer, points) {
  const a = decodePng(dayBuffer), b = decodePng(nightBuffer);
  if (!a.supported || !b.supported) return { measured: false, reason: 'png-decode' };
  if (a.width !== b.width || a.height !== b.height) return { measured: false, reason: 'size-mismatch' };
  const day = [], night = [];
  for (const p of points) {
    if (p.x < 0 || p.y < 0 || p.x >= a.width || p.y >= a.height) continue;
    day.push(lumaAt(a, p.x, p.y)); night.push(lumaAt(b, p.x, p.y));
  }
  if (!day.length) return { measured: false, reason: 'no-point-inside-frame' };
  const stat = arr => {
    const s = [...arr].sort((x, y) => x - y);
    return { n: s.length, min: round2(s[0]), median: round2(s[Math.floor(s.length / 2)]), p90: round2(s[Math.floor(s.length * .9)]), max: round2(s[s.length - 1]), mean: round2(arr.reduce((t, v) => t + v, 0) / arr.length) };
  };
  let brighter = 0;
  for (let i = 0; i < day.length; i++) if (night[i] > day[i] + 10) brighter++;
  return { measured: true, points: day.length, day: stat(day), night: stat(night), pointsBrighterAtNight: brighter };
}

function brighterAtNight(dayBuffer, nightBuffer, [fx0, fy0, fx1, fy1], delta = 25) {
  const a = decodePng(dayBuffer), b = decodePng(nightBuffer);
  if (!a.supported || !b.supported) return { supported: false, reason: 'png-decode' };
  if (a.width !== b.width || a.height !== b.height) return { supported: false, reason: 'size-mismatch' };
  const x0 = Math.floor(fx0 * a.width), x1 = Math.min(a.width, Math.ceil(fx1 * a.width));
  const y0 = Math.floor(fy0 * a.height), y1 = Math.min(a.height, Math.ceil(fy1 * a.height));
  let n = 0, count = 0, sumDelta = 0, maxDelta = 0;
  for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
    const d = lumaAt(b, x, y) - lumaAt(a, x, y);
    count++;
    if (d > maxDelta) maxDelta = d;
    if (d >= delta) { n++; sumDelta += d; }
  }
  return {
    region: [fx0, fy0, fx1, fy1], pixelsCompared: count, deltaThreshold: delta,
    brighterAtNightPixels: n, brighterAtNightFraction: Math.round((n / count) * 1e5) / 1e5,
    meanDeltaWhereBrighter: n ? round2(sumDelta / n) : 0, maxDelta: round2(maxDelta),
  };
}

/**
 * Isolated bright spikes: a pixel far above the median of its own 7x7
 * neighbourhood. A 1-3 px starfield point is exactly that; the moon's own
 * shading is not. Run over the disc and over the sky annulus around it, the
 * annulus is the control that proves the detector sees stars at all.
 */
function spikeCount(buffer, cx, cy, r, { inner = true, above = 35, canvasCentre = null } = {}) {
  const img = decodePng(buffer);
  if (!img.supported) return img;
  const lo = inner ? 0 : r * 1.5, hi = inner ? r * .8 : r * 2.6;
  const ccx = canvasCentre ? canvasCentre[0] : img.width / 2, ccy = canvasCentre ? canvasCentre[1] : img.height / 2;
  let spikes = 0, sampled = 0, maxExcess = 0;
  const neigh = [];
  for (let y = Math.max(3, Math.floor(cy - hi)); y < Math.min(img.height - 3, Math.ceil(cy + hi)); y++) {
    for (let x = Math.max(3, Math.floor(cx - hi)); x < Math.min(img.width - 3, Math.ceil(cx + hi)); x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d < lo || d > hi) continue;
      if (Math.hypot(x - ccx, y - ccy) <= 14) continue; // the walk-mode crosshair
      neigh.length = 0;
      for (let dy = -3; dy <= 3; dy++) for (let dx = -3; dx <= 3; dx++) neigh.push(lumaAt(img, x + dx, y + dy));
      neigh.sort((p, q) => p - q);
      const excess = lumaAt(img, x, y) - neigh[Math.floor(neigh.length / 2)];
      sampled++;
      if (excess > maxExcess) maxExcess = excess;
      if (excess >= above) spikes++;
    }
  }
  return { where: inner ? 'moon-disc' : 'sky-annulus', pixelsSampled: sampled, spikeThreshold: above, spikes, spikeFraction: sampled ? Math.round((spikes / sampled) * 1e5) / 1e5 : null, maxExcessOverNeighbourhood: round2(maxExcess) };
}

// --- browser ----------------------------------------------------------------
const console_ = [];
const pageErrors = [];
const gpuArgs = ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist', '--disable-frame-rate-limit', '--disable-gpu-vsync'];
const channel = process.env.BROWSER_CHANNEL === undefined ? 'chrome' : (process.env.BROWSER_CHANNEL || undefined);
const browser = await chromium.launch({ headless: true, channel, args: gpuArgs });
report.browser = { channel: channel || 'playwright-bundled-chromium', args: gpuArgs, version: browser.version() };
const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
await context.addInitScript(([key, value]) => { try { localStorage.setItem(key, value); } catch { /* blocked */ } }, ['permagent-daemon-token', token]);

await context.addInitScript(() => {
  try { performance.setResourceTimingBufferSize(3000); } catch { /* unsupported */ }
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
    const roots = window.__probeRoots || [];
    let budget = 400000;
    for (const root of roots) {
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
  const state = () => { const s = window.__findR3FStore(); return s ? s.getState() : null; };
  const round3 = n => Math.round(n * 1000) / 1000;

  window.__forumProbe = function forumProbe() {
    const s = state();
    if (!s || !s.camera) return { ok: false, reason: 'no-r3f-store' };
    s.camera.updateMatrixWorld();
    const e = s.camera.matrixWorld.elements;
    const pos = { x: e[12], y: e[13], z: e[14] }, dir = { x: -e[8], y: -e[9], z: -e[10] };
    return {
      ok: true,
      camera: { x: round3(pos.x), y: round3(pos.y), z: round3(pos.z) },
      feet: { x: round3(pos.x), y: round3(pos.y - 1.7), z: round3(pos.z) },
      yaw: round3(Math.atan2(-dir.x, -dir.z)),
      pitch: round3(Math.asin(Math.max(-1, Math.min(1, dir.y)))),
      pointerLock: document.pointerLockElement ? document.pointerLockElement.tagName : null,
    };
  };

  /** Both bodies with their live world position and screen projection. */
  window.__celestialProbe = function celestialProbe() {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { scene, camera, size } = s;
    camera.updateMatrixWorld();
    const bodies = [];
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry || o.geometry.type !== 'SphereGeometry') return;
      const r = o.geometry.parameters && o.geometry.parameters.radius;
      const label = r === 140 ? 'gasGiant' : r === 46 ? 'moon' : null;
      if (!label) return;
      o.updateMatrixWorld();
      const m = o.matrixWorld.elements;
      const world = { x: m[12], y: m[13], z: m[14] };
      const v = new o.position.constructor(world.x, world.y, world.z); v.project(camera);
      const sx = (v.x * .5 + .5) * size.width, sy = (-v.y * .5 + .5) * size.height;
      const dx = world.x - camera.position.x, dy = world.y - camera.position.y, dz = world.z - camera.position.z;
      const dist = Math.hypot(dx, dy, dz);
      const radiusPx = Math.atan(r / dist) * ((size.height / 2) / Math.tan((camera.fov * Math.PI / 180) / 2));
      bodies.push({
        body: label, name: o.name || '(unnamed)', radiusM: r, distanceM: Math.round(dist),
        world: { x: Math.round(world.x * 10) / 10, y: Math.round(world.y * 10) / 10, z: Math.round(world.z * 10) / 10 },
        materialDepthWrite: o.material ? o.material.depthWrite : null,
        renderOrder: o.renderOrder,
        screen: { x: Math.round(sx), y: Math.round(sy) }, radiusPx: Math.round(radiusPx * 10) / 10,
        inFrustum: v.z > -1 && v.z < 1 && sx > -radiusPx && sx < size.width + radiusPx && sy > -radiusPx && sy < size.height + radiusPx,
      });
    });
    return { ok: true, canvas: { width: size.width, height: size.height }, bodies };
  };

  /**
   * What stands between the camera and a body. Raycast along the line to the
   * body's centre and report the first hit that is not the body itself, the
   * sky dome (radius 4800), the starfield or the cloud plane.
   */
  window.__occlusionProbe = function occlusionProbe(bodyLabel) {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { scene, camera, raycaster } = s;
    camera.updateMatrixWorld();
    let target = null, bodyMesh = null;
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry || o.geometry.type !== 'SphereGeometry') return;
      const r = o.geometry.parameters && o.geometry.parameters.radius;
      const label = r === 140 ? 'gasGiant' : r === 46 ? 'moon' : null;
      if (label !== bodyLabel) return;
      o.updateMatrixWorld();
      const m = o.matrixWorld.elements;
      target = { x: m[12], y: m[13], z: m[14] };
      bodyMesh = o;
    });
    if (!target) return { ok: false, reason: `no-body-${bodyLabel}` };
    const V = camera.position.constructor;
    const dir = new V(target.x - camera.position.x, target.y - camera.position.y, target.z - camera.position.z).normalize();
    if (!raycaster) return { ok: false, reason: 'no-raycaster' };
    const rc = new raycaster.constructor();
    rc.far = Infinity; rc.near = 0;
    rc.set(camera.position.clone(), dir);
    // three's Mesh.raycast honours material.side, so a ray leaving a camera
    // that stands UNDER a roof passes straight through it — the roof's
    // triangles face away. That is a false "nothing is in the way" for a body
    // the roof plainly hides. Raycast double-sided and put every side back.
    const sides = [];
    scene.traverse(o => {
      if (!o.isMesh) return;
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      for (const m of mats) if (m && m.side !== 2) { sides.push([m, m.side]); m.side = 2; }
    });
    let hits;
    try { hits = rc.intersectObject(scene, true); }
    finally { for (const [m, side] of sides) m.side = side; }
    const bodyDistance = camera.position.distanceTo(new V(target.x, target.y, target.z));
    const ignored = o => {
      if (o === bodyMesh) return true;
      if (o.isPoints || o.isLine) return true;
      const g = o.geometry;
      if (!g) return true;
      if (g.type === 'SphereGeometry' && g.parameters && (g.parameters.radius === 4800 || g.parameters.radius === 140 || g.parameters.radius === 46 || g.parameters.radius === 5.2)) return true;
      if (g.type === 'PlaneGeometry' && g.parameters && g.parameters.width === 650) return true; // cloud plane
      return false;
    };
    const real = hits.filter(h => h.object && h.object.visible && !ignored(h.object) && h.distance < bodyDistance);
    const first = real[0] || null;
    return {
      ok: true, body: bodyLabel,
      bodyDistanceM: Math.round(bodyDistance),
      occluded: Boolean(first),
      firstBlockingMesh: first ? { name: first.object.name || '(unnamed)', material: first.object.material && first.object.material.name || null, distanceM: Math.round(first.distance * 10) / 10, point: { x: Math.round(first.point.x), y: Math.round(first.point.y), z: Math.round(first.point.z) } } : null,
      realHitsBeforeBody: real.length,
    };
  };

  /** Window-strip boxes, clustered in world space: the export merges them into
   *  one mesh, so the merged centroid is meaningless. */
  window.__stripClusters = function stripClusters(fragment, cell) {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { scene, camera, size } = s;
    camera.updateMatrixWorld();
    const grid = new Map();
    let meshes = 0, vertices = 0;
    const V = camera.position.constructor;
    scene.traverse(o => {
      if (!o.isMesh || !o.geometry) return;
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      if (!mats.some(m => m && typeof m.name === 'string' && m.name.toLowerCase().includes(fragment.toLowerCase()))) return;
      meshes++;
      o.updateMatrixWorld();
      const pos = o.geometry.attributes.position;
      if (!pos) return;
      vertices += pos.count;
      const p = new V();
      for (let i = 0; i < pos.count; i++) {
        p.set(pos.getX(i), pos.getY(i), pos.getZ(i)).applyMatrix4(o.matrixWorld);
        const key = `${Math.round(p.x / cell)},${Math.round(p.y / cell)},${Math.round(p.z / cell)}`;
        const bucket = grid.get(key) || { n: 0, x: 0, y: 0, z: 0, points: [] };
        bucket.n++; bucket.x += p.x; bucket.y += p.y; bucket.z += p.z;
        if (bucket.points.length < 400) bucket.points.push([p.x, p.y, p.z]);
        grid.set(key, bucket);
      }
    });
    const clusters = [...grid.entries()].map(([key, b]) => {
      const centre = { x: b.x / b.n, y: b.y / b.n, z: b.z / b.n };
      const onScreen = [];
      for (const [px, py, pz] of b.points) {
        const q = new V(px, py, pz); q.project(camera);
        const sx = (q.x * .5 + .5) * size.width, sy = (-q.y * .5 + .5) * size.height;
        if (q.z > -1 && q.z < 1 && sx >= 4 && sx < size.width - 4 && sy >= 4 && sy < size.height - 4) onScreen.push({ x: Math.round(sx), y: Math.round(sy) });
      }
      return {
        key, vertices: b.n,
        centre: { x: Math.round(centre.x * 10) / 10, y: Math.round(centre.y * 10) / 10, z: Math.round(centre.z * 10) / 10 },
        distanceM: Math.round(camera.position.distanceTo(new V(centre.x, centre.y, centre.z)) * 10) / 10,
        sampledPoints: b.points.length,
        onScreenPoints: onScreen.length,
        onScreenSamples: onScreen.slice(0, 260),
      };
    }).sort((a, b) => b.vertices - a.vertices);
    return {
      ok: true, fragment, cellM: cell, meshesWithThatMaterial: meshes, totalVertices: vertices,
      boxesImplied: Math.round(vertices / 24), clusterCount: clusters.length,
      canvas: { width: size.width, height: size.height },
      clusters,
    };
  };

  /**
   * Hide (and restore) every mesh that is not sky. Occlusion by raycast is not
   * available on this export — the GLB is meshopt/KHR_mesh_quantization, so
   * `Raycaster.intersectObject` returns nothing for the merged meshes (their
   * local boundingSphere radius is 1 and the scale lives on the node matrix).
   * Hiding the world and re-measuring the same disc answers the same question
   * from the pixels: if the body brightens sharply once the world is gone, the
   * world was standing in front of it.
   */
  window.__setWorldVisible = function setWorldVisible(on) {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { scene } = s;
    if (!window.__hiddenWorld) window.__hiddenWorld = [];
    if (!on) {
      scene.traverse(o => {
        if (!o.isMesh || !o.geometry || !o.visible) return;
        const g = o.geometry, p = g.parameters;
        if (g.type === 'SphereGeometry' && p && [4800, 140, 46, 5.2].includes(p.radius)) return;
        if (g.type === 'PlaneGeometry' && p && p.width === 650) return;
        window.__hiddenWorld.push(o);
        o.visible = false;
      });
      return { ok: true, hidden: window.__hiddenWorld.length };
    }
    const n = window.__hiddenWorld.length;
    for (const o of window.__hiddenWorld) o.visible = true;
    window.__hiddenWorld = [];
    return { ok: true, restored: n };
  };

  /** The live renderer's shadow state, straight off the WebGLRenderer. */
  window.__shadowInfo = function shadowInfo() {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { gl, scene } = s;
    let casters = 0, receivers = 0, shadowLights = 0;
    scene.traverse(o => {
      if (o.isMesh) { if (o.castShadow) casters++; if (o.receiveShadow) receivers++; }
      if (o.isLight && o.castShadow) shadowLights++;
    });
    return {
      ok: true,
      enabled: gl.shadowMap.enabled,
      // three constants: 0 Basic, 1 PCF, 2 PCFSoft (deprecated in 0.184), 3 VSM
      type: gl.shadowMap.type,
      typeName: ['BasicShadowMap', 'PCFShadowMap', 'PCFSoftShadowMap', 'VSMShadowMap'][gl.shadowMap.type] ?? String(gl.shadowMap.type),
      threeRevision: (window.THREE && window.THREE.REVISION) || null,
      shadowCastingMeshes: casters, shadowReceivingMeshes: receivers, shadowCastingLights: shadowLights,
    };
  };
  /** Deliberate, restored-immediately test mutation: turn the shadow pass off
   *  so the difference it makes to the frame can be measured. */
  window.__setShadows = function setShadows(on) {
    const s = state();
    if (!s) return { ok: false, reason: 'no-r3f-store' };
    const { gl, scene } = s;
    gl.shadowMap.enabled = Boolean(on);
    gl.shadowMap.needsUpdate = true;
    scene.traverse(o => {
      if (!o.isMesh) return;
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      for (const m of mats) if (m) m.needsUpdate = true;
    });
    return { ok: true, enabled: gl.shadowMap.enabled };
  };
});

const page = await context.newPage();
page.setDefaultTimeout(90000);
page.on('pageerror', e => pageErrors.push({ message: e.message }));
page.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') console_.push({ type: m.type(), text: m.text().slice(0, 300) }); });

const probe = () => page.evaluate(() => window.__forumProbe());
const celestial = () => page.evaluate(() => window.__celestialProbe());
const occlusion = body => page.evaluate(b => window.__occlusionProbe(b), body);
const clusters = (frag, cell) => page.evaluate(([f, c]) => window.__stripClusters(f, c), [frag, cell]);

function hasTool(node, tool) {
  if (!node || typeof node !== 'object') return false;
  if (node.type === 'panel') return node.tool === tool;
  return Array.isArray(node.children) && node.children.some(c => hasTool(c, tool));
}
const ws = await daemon('/api/workspaces');
const worldWorkspace = (Array.isArray(ws.body) ? ws.body : []).map(w => ({ id: w.id, name: w.name, layout: typeof w.layoutJson === 'string' ? JSON.parse(w.layoutJson) : w.layoutJson })).find(w => hasTool(w.layout, 'world'));
if (!worldWorkspace) { report.fatal = `no world workspace (GET /api/workspaces -> ${ws.status})`; await flush(); await browser.close(); process.exit(1); }
report.workspace = worldWorkspace.name;

async function robustClick(locator, label = '') {
  try { await locator.click({ timeout: 15000 }); return 'normal'; }
  catch { await locator.evaluate(n => n.scrollIntoView({ block: 'center' })).catch(() => {}); await locator.click({ force: true, timeout: 15000 }); return `forced (${label})`; }
}
async function forumVisible() {
  return page.evaluate(() => {
    const shown = [...document.querySelectorAll('.forum-shell')].find(s => s.getBoundingClientRect().width > 1);
    return { visible: Boolean(shown) };
  });
}
async function shot(name, opts = {}) {
  const file = `assets/world/forum/browser/${name}.png`;
  await page.screenshot({ path: path.join(repo, file), timeout: 120000, ...opts });
  return file;
}
// --- PNG encode, so a crop can be written back out as evidence -------------
const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xFF] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}
function pngChunk(type, data) {
  const out = Buffer.alloc(8 + data.length + 4);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, 'ascii');
  data.copy(out, 8);
  out.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, 'ascii'), data])), 8 + data.length);
  return out;
}
function encodePng(width, height, rgb) {
  const raw = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y++) {
    raw[y * (width * 3 + 1)] = 0;
    rgb.copy(raw, y * (width * 3 + 1) + 1, y * width * 3, (y + 1) * width * 3);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; ihdr[9] = 2; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw, { level: 6 })),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}
function cropPng(buffer, box) {
  const img = decodePng(buffer);
  if (!img.supported) return null;
  const x0 = Math.max(0, Math.round(box.x)), y0 = Math.max(0, Math.round(box.y));
  const w = Math.min(img.width - x0, Math.round(box.width)), h = Math.min(img.height - y0, Math.round(box.height));
  if (w <= 0 || h <= 0) return null;
  const rgb = Buffer.alloc(w * h * 3);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const at = (y0 + y) * img.stride + (x0 + x) * img.channels, to = (y * w + x) * 3;
    rgb[to] = img.pixels[at]; rgb[to + 1] = img.pixels[at + 1]; rgb[to + 2] = img.pixels[at + 2];
  }
  return encodePng(w, h, rgb);
}

/**
 * The canvas, cropped out of a FULL-PAGE screenshot.
 *
 * `page.screenshot({ clip })` over this continuously animating WebGL canvas
 * returns tiles that were never painted — a solid, axis-aligned block of exact
 * RGB 0,0,0 across the middle of the frame, reproducible at some camera poses
 * (job 19: 120,314 pure-black pixels in three consecutive clipped captures at
 * the gate approach, while an unclipped capture of the same frame is clean).
 * It is in job 18's own evidence and it silently destroys a disc measurement.
 * Capturing the whole page and cropping in Node avoids the clip path entirely.
 */
async function canvasShotBuffer() {
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  if (!box) return null;
  const full = await page.screenshot({ timeout: 120000 });
  return cropPng(full, box);
}
/**
 * The same frame read straight out of the WebGL drawing buffer, bypassing the
 * page compositor altogether. `preserveDrawingBuffer` is off, so this has to
 * run inside a rAF callback registered after r3f's own loop callback for that
 * frame — which is what the nested rAF below gets. Used as a second opinion
 * whenever the composited capture comes back with a block of pure black.
 */
async function canvasGlBuffer() {
  const dataUrl = await page.evaluate(() => new Promise(resolve => {
    const canvas = document.querySelector('.forum-shell canvas');
    if (!canvas) return resolve(null);
    requestAnimationFrame(() => requestAnimationFrame(() => {
      try { resolve(canvas.toDataURL('image/png')); } catch { resolve(null); }
    }));
  }));
  if (typeof dataUrl !== 'string' || !dataUrl.startsWith('data:image/png;base64,')) return null;
  return Buffer.from(dataUrl.slice('data:image/png;base64,'.length), 'base64');
}
/** Pixels that are exactly RGB 0,0,0 — the signature of the capture artifact
 *  above, counted so the choice between captures is on the record. */
function zeroPixels(buffer) {
  const img = decodePng(buffer);
  if (!img.supported) return Number.POSITIVE_INFINITY;
  let n = 0;
  for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) {
    const at = y * img.stride + x * img.channels;
    if (!img.pixels[at] && !img.pixels[at + 1] && !img.pixels[at + 2]) n++;
  }
  return n;
}
/**
 * `page.screenshot({clip})` over a continuously animating WebGL canvas
 * intermittently returns tiles that were never painted — a solid, axis-aligned
 * block of exact RGB 0,0,0 across the middle of the frame. It is visible in
 * job 18's own evidence and it silently destroys a disc measurement (a body
 * sampled inside such a block reads ~0 against a ~0 sky). Capture repeatedly
 * and keep the frame with the fewest pure-black pixels; every count is
 * reported, so a frame that is legitimately dark is not mistaken for a repair.
 */
async function stableCanvasShot(tries = 3) {
  const counts = [];
  let best = null, bestCount = Number.POSITIVE_INFINITY, bestSource = null;
  for (let i = 0; i < tries; i++) {
    for (const [source, take] of [['composited', canvasShotBuffer], ['webgl-readback', canvasGlBuffer]]) {
      const buffer = await take();
      if (!buffer) continue;
      const n = zeroPixels(buffer);
      counts.push({ source, zeroPixels: n });
      if (n < bestCount) { best = buffer; bestCount = n; bestSource = source; }
      if (n === 0) break;
    }
    if (bestCount === 0) break;
    await page.waitForTimeout(320);
  }
  return { buffer: best, zeroPixelCounts: counts, chosenZeroPixels: bestCount === Number.POSITIVE_INFINITY ? null : bestCount, chosenSource: bestSource };
}
async function saveBuffer(name, buffer) {
  const file = `assets/world/forum/browser/${name}.png`;
  if (!buffer) return null;
  await writeFile(path.join(repo, file), buffer);
  return file;
}
async function samplePerf(seconds = 5) {
  const samples = [];
  for (let i = 0; i < seconds; i++) {
    await page.waitForTimeout(1050);
    const s = await page.evaluate(() => (window.__worldPerf ? { ...window.__worldPerf } : null));
    if (s) samples.push(s);
  }
  const fps = samples.map(s => s.fps).filter(Number.isFinite).sort((a, b) => a - b);
  const last = samples[samples.length - 1] || null;
  return { fps: fps.length ? { min: fps[0], median: fps[Math.floor(fps.length / 2)], max: fps[fps.length - 1], n: fps.length } : null, calls: last?.calls ?? null, triangles: last?.triangles ?? null };
}
async function bootWorld() {
  await page.goto(appUrl, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForFunction(() => !document.body.textContent.includes('Loading workspaces'), undefined, { timeout: 60000 }).catch(() => {});
  if (!(await forumVisible()).visible) {
    await robustClick(page.getByRole('button', { name: worldWorkspace.name, exact: false }).first(), 'workspace');
    await page.waitForFunction(() => [...document.querySelectorAll('.forum-shell')].some(s => s.getBoundingClientRect().width > 1), undefined, { timeout: 90000 });
  }
  await page.waitForFunction(() => Boolean(window.__worldPerf), undefined, { timeout: 150000 }).catch(() => {});
  await page.waitForTimeout(2500);
}
const appearanceLabel = () => page.locator('[data-testid="forum-appearance"]').first().textContent().catch(() => null);
async function setAppearance(scheme) {
  await page.emulateMedia({ colorScheme: scheme });
  await page.waitForTimeout(2600);
  return appearanceLabel();
}
async function district(name) {
  await robustClick(page.locator('.forum-districts button', { hasText: name }).first(), `district ${name}`).catch(() => {});
  await page.waitForTimeout(2400);
}
async function enterWalk() {
  await robustClick(page.locator('.forum-play button').first(), 'Walk the world');
  await page.waitForTimeout(1600);
  return page.locator('.forum-controls').first().innerText().catch(() => null);
}
async function leaveWalk() {
  await page.keyboard.press('Escape');
  await page.waitForTimeout(1400);
}
async function faceYaw(target, tol = 0.035, maxMs = 9000) {
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
    await page.keyboard.down(key);
    await page.waitForTimeout(Math.min(700, Math.max(25, (Math.abs(d) / 1.6) * 1000 * 0.85)));
    await page.keyboard.up(key);
    await page.waitForTimeout(90);
  }
  return { ok: false, reason: 'yaw-not-reached', yaw: last };
}
async function aimTo(targetYaw, targetPitch, tol = 0.05, maxSteps = 22) {
  const box = await page.locator('.forum-shell canvas').first().boundingBox();
  if (!box) return { ok: false, reason: 'no-canvas-box' };
  const cx = box.x + box.width / 2, cy = box.y + box.height / 2;
  let gain = 1 / 0.002;
  await faceYaw(targetYaw, tol);
  for (let i = 0; i < maxSteps; i++) {
    const s = await probe();
    if (!s.ok) return { ok: false, reason: s.reason };
    const dPitch = targetPitch - s.pitch;
    if (Math.abs(dPitch) < tol) break;
    const dy = Math.max(-260, Math.min(260, -dPitch * gain));
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
      if (Number.isFinite(measured) && measured > 1 && measured < 20000) gain = gain * .5 + measured * .5;
    }
  }
  const yawFinal = await faceYaw(targetYaw, tol);
  const end = await probe();
  return { ok: yawFinal.ok && Math.abs(targetPitch - end.pitch) < tol * 2, targetYaw: round2(targetYaw), targetPitch: round2(targetPitch), yaw: end.yaw, pitch: end.pitch };
}
async function aimAtWorldPoint(p) {
  const w = await probe();
  if (!w.ok) return { ok: false, reason: w.reason };
  const dx = p.x - w.camera.x, dy = p.y - w.camera.y, dz = p.z - w.camera.z;
  return aimTo(Math.atan2(-dx, -dz), Math.atan2(dy, Math.hypot(dx, dz)));
}

const want = id => only.length === 0 || only.includes(id);

// ------------------------------------------------------------------- sky
/**
 * Day and night contrast for both bodies, from three vantages, with the pair
 * taken from one camera. Also the occlusion raycast at each aim.
 */
if (want('sky')) {
  try {
    await page.emulateMedia({ colorScheme: 'light' });
    await bootWorld();
    const vantages = [
      { id: 'commons-overlook', districtName: 'Commons' },
      { id: 'gate-approach', districtName: 'Mesh gate' },
      { id: 'harbour', districtName: 'Harbor' },
    ];
    const rows = [];
    for (const v of vantages) {
      await district(v.districtName);
      const walkControls = await enterWalk();
      await page.waitForTimeout(2200);
      for (const body of ['moon', 'gasGiant']) {
       // The black-block artifact (see canvasShotBuffer) sticks to a camera
       // pose: retrying the capture in place returns the same block from both
       // the compositor AND the WebGL readback, so the frame itself carries
       // it. Nudging the view and re-aiming clears it.
       let attempt = 0;
       for (;;) {
        const live = await celestial();
        const target = live.ok ? live.bodies.find(b => b.body === body) : null;
        if (!target) { rows.push({ vantage: v.id, body, error: 'body-not-in-scene' }); break; }
        if (attempt) {
          const here = await probe();
          await faceYaw((here.yaw || 0) + 0.35 * attempt, 0.05);
          await page.waitForTimeout(900);
        }
        const aim = await aimAtWorldPoint(target.world);
        await page.waitForTimeout(1100);
        const camera = await probe();
        const occluded = await occlusion(body);

        // day frame
        const dayBodies = await celestial();
        const dayCapture = await stableCanvasShot();
        const dayBuffer = dayCapture.buffer;
        const dayFile = await saveBuffer(`v3-${v.id}-${body}-day`, dayBuffer);
        // same camera, appearance flipped live (no reload, no pose drift)
        const nightLabel = await setAppearance('dark');
        const nightBodies = await celestial();
        const nightCapture = await stableCanvasShot();
        const nightBuffer = nightCapture.buffer;
        const nightFile = await saveBuffer(`v3-${v.id}-${body}-night`, nightBuffer);
        const cameraAfter = await probe();
        await setAppearance('light');

        const canvasSize = dayBodies.ok ? [dayBodies.canvas.width / 2, dayBodies.canvas.height / 2] : null;
        const pick = (probeResult) => probeResult.ok ? probeResult.bodies.find(b => b.body === body) : null;
        const d = pick(dayBodies), n = pick(nightBodies);
        const dirty = Math.max(dayCapture.chosenZeroPixels ?? 0, nightCapture.chosenZeroPixels ?? 0) > 5000;
        if (dirty && attempt < 3) { attempt++; continue; }
        rows.push({
          vantage: v.id, body, walkControls,
          captureArtifact: dirty ? `unrecovered after ${attempt} re-aims` : (attempt ? `cleared after ${attempt} re-aim(s)` : null),
          camera: camera.ok ? { position: camera.camera, yaw: camera.yaw, pitch: camera.pitch } : camera,
          cameraUnchangedAfterFlip: camera.ok && cameraAfter.ok
            ? Math.round(Math.hypot(cameraAfter.camera.x - camera.camera.x, cameraAfter.camera.y - camera.camera.y, cameraAfter.camera.z - camera.camera.z) * 1000) / 1000
            : null,
          nightLabel,
          aim, occlusion: occluded,
          renderState: d ? { depthWrite: d.materialDepthWrite, renderOrder: d.renderOrder, name: d.name, distanceM: d.distanceM, radiusPx: d.radiusPx } : null,
          capture: { day: dayCapture.zeroPixelCounts, night: nightCapture.zeroPixelCounts, chosenDayZeroPixels: dayCapture.chosenZeroPixels, chosenNightZeroPixels: nightCapture.chosenZeroPixels },
          day: d && d.inFrustum && dayBuffer ? discLuma(dayBuffer, d.screen.x, d.screen.y, d.radiusPx, canvasSize) : { measured: false, reason: d ? 'not-in-frustum' : 'no-body' },
          night: n && n.inFrustum && nightBuffer ? discLuma(nightBuffer, n.screen.x, n.screen.y, n.radiusPx, canvasSize) : { measured: false, reason: n ? 'not-in-frustum' : 'no-body' },
          starSpikes: n && n.inFrustum && nightBuffer && body === 'moon'
            ? {
              note: 'Isolated pixels >=35 luma above their own 7x7 median. The sky annulus is the control: it is full of stars, so a detector that finds none there is not working.',
              disc: spikeCount(nightBuffer, n.screen.x, n.screen.y, n.radiusPx, { inner: true, canvasCentre: canvasSize }),
              sky: spikeCount(nightBuffer, n.screen.x, n.screen.y, n.radiusPx, { inner: false, canvasCentre: canvasSize }),
            }
            : null,
          screenshots: { day: dayFile, night: nightFile },
        });
        await flush();
        break;
       }
      }
      await leaveWalk();
    }
    gateResult('sky', {
      description: 'gas giant and moon, disc-vs-sky contrast in day and night from one camera, plus what really blocks the line of sight',
      numbersToBeat: { moon: 0.82, gasGiantCommons: 1.09, gasGiantGateCourt: 0.70 },
      rows,
    });
  } catch (e) { gateResult('sky', { failure: String(e && e.message || e), screenshot: await shot('v3-sky-failure').catch(() => null) }); }
  await flush();
}

// --------------------------------------------------------------- occlusion
/** The world must still be able to hide a body. Walk behind the tall market
 *  habitats and raycast at the moon and the gas giant from there. */
if (want('occlude')) {
  try {
    await page.emulateMedia({ colorScheme: 'dark' });
    await bootWorld();
    const label = await appearanceLabel();
    const probes = [];
    for (const place of ['Maker hall', 'Conservatory', 'Commons', 'Harbor', 'Mesh gate']) {
      await district(place);
      await enterWalk();
      await page.waitForTimeout(2000);
      const at = await probe();
      for (const body of ['moon', 'gasGiant']) {
        const live = await celestial();
        const target = live.ok ? live.bodies.find(b => b.body === body) : null;
        if (!target) continue;
        const aim = await aimAtWorldPoint(target.world);
        await page.waitForTimeout(900);
        const occ = await occlusion(body);
        const after = await celestial();
        const b = after.ok ? after.bodies.find(x => x.body === body) : null;
        const capture = await stableCanvasShot();
        const buffer = capture.buffer;
        const centre = after.ok ? [after.canvas.width / 2, after.canvas.height / 2] : null;
        const withWorld = b && b.inFrustum && buffer ? discLuma(buffer, b.screen.x, b.screen.y, b.radiusPx, centre) : { measured: false, reason: 'not-in-frustum' };
        const shotFile = await saveBuffer(`v3-occlude-${place.replace(/\W+/g, '-').toLowerCase()}-${body}`, buffer);
        // same camera, world geometry hidden: what the body looks like with
        // nothing in front of it.
        const hidden = await page.evaluate(() => window.__setWorldVisible(false));
        await page.waitForTimeout(1400);
        const bareCapture = await stableCanvasShot(2);
        const bare = b && b.inFrustum && bareCapture.buffer ? discLuma(bareCapture.buffer, b.screen.x, b.screen.y, b.radiusPx, centre) : { measured: false, reason: 'not-in-frustum' };
        const bareFile = await saveBuffer(`v3-occlude-${place.replace(/\W+/g, '-').toLowerCase()}-${body}-noworld`, bareCapture.buffer);
        const restored = await page.evaluate(() => window.__setWorldVisible(true));
        await page.waitForTimeout(1200);
        probes.push({
          from: place, walkerAt: at.ok ? at.camera : at, body, aim,
          occlusionRaycast: { ...occ, note: 'Raycaster finds nothing on this export (quantized merged GLB meshes); the hide-the-world measurement below is the evidence.' },
          capture: capture.zeroPixelCounts,
          disc: withWorld,
          discWithWorldHidden: bare,
          worldMeshesHidden: hidden, worldRestored: restored,
          occludedByTheWorld: withWorld.measured && bare.measured
            ? { discLumaLostToGeometry: round2(bare.discMeanLuma - withWorld.discMeanLuma), fractionOfBodyLuma: bare.discMeanLuma > 1 ? round2(1 - withWorld.discMeanLuma / bare.discMeanLuma) : null }
            : null,
          screenshots: { asRendered: shotFile, worldHidden: bareFile },
        });
        await flush();
      }
      await leaveWalk();
    }
    gateResult('occlude', {
      description: 'raycast from the live camera to each body: a body behind real geometry must report a blocking mesh and a low-contrast disc',
      appearance: label,
      probes,
    });
  } catch (e) { gateResult('occlude', { failure: String(e && e.message || e), screenshot: await shot('v3-occlude-failure').catch(() => null) }); }
  await flush();
}

// ----------------------------------------------------------------- windows
if (want('windows')) {
  try {
    await page.emulateMedia({ colorScheme: 'dark' });
    await bootWorld();
    const nightLabel = await appearanceLabel();

    // 1. the overview camera at the commons: "the commons overlook".
    await district('Commons');
    await page.waitForTimeout(1200);
    const overlookClusters = await clusters('Warm Window Emission', 30);
    const nightOverlookCapture = await stableCanvasShot();
    const nightOverlook = nightOverlookCapture.buffer;
    const nightOverviewShot = await shot('v3-world-night');
    await saveBuffer('v3-world-night-canvas', nightOverlook);
    await setAppearance('light');
    const dayOverlookCapture = await stableCanvasShot();
    const dayOverlook = dayOverlookCapture.buffer;
    await setAppearance('dark');
    const overlookPoints = overlookClusters.ok ? overlookClusters.clusters.flatMap(c => c.onScreenSamples) : [];

    // 2. ground vantages that can actually see a cluster.
    const vantages = [];
    for (const place of ['Council garden', 'Arrival gardens', 'Maker hall']) {
      await leaveWalk().catch(() => {});
      await district(place);
      await enterWalk();
      await page.waitForTimeout(2200);
      const near = await clusters('Warm Window Emission', 30);
      if (!near.ok || !near.clusters.length) { vantages.push({ place, error: 'no-clusters' }); continue; }
      const pick = [...near.clusters].sort((a, b) => a.distanceM - b.distanceM)[0];
      const aim = await aimAtWorldPoint(pick.centre);
      await page.waitForTimeout(1200);
      const aimed = await clusters('Warm Window Emission', 30);
      const visible = aimed.ok ? aimed.clusters.filter(c => c.onScreenPoints > 0) : [];
      const points = visible.flatMap(c => c.onScreenSamples);
      const slug = place.replace(/\W+/g, '-').toLowerCase();
      const nightCapture = await stableCanvasShot();
      const nightBuffer = nightCapture.buffer;
      const nightFile = await saveBuffer(`v3-windows-${slug}-night`, nightBuffer);
      await setAppearance('light');
      const dayCapture = await stableCanvasShot();
      const dayBuffer = dayCapture.buffer;
      const dayFile = await saveBuffer(`v3-windows-${slug}-day`, dayBuffer);
      await setAppearance('dark');
      const cameraNow = await probe();
      vantages.push({
        place, aimedAtCluster: { centre: pick.centre, vertices: pick.vertices, distanceM: pick.distanceM },
        aim, camera: cameraNow.ok ? { position: cameraNow.camera, yaw: cameraNow.yaw, pitch: cameraNow.pitch } : cameraNow,
        capture: { day: dayCapture.zeroPixelCounts, night: nightCapture.zeroPixelCounts },
        clustersVisible: visible.length,
        clusterSummary: visible.slice(0, 6).map(c => ({ centre: c.centre, distanceM: c.distanceM, onScreenPoints: c.onScreenPoints, vertices: c.vertices })),
        verticesProjectingIntoView: points.length,
        stripPixels: points.length && dayBuffer && nightBuffer ? pointLuma(dayBuffer, nightBuffer, points) : { measured: false, reason: 'no-strip-vertex-in-view' },
        wholeFrameBrighterAtNight: dayBuffer && nightBuffer ? brighterAtNight(dayBuffer, nightBuffer, [0, .05, 1, .95]) : null,
        screenshots: { day: dayFile, night: nightFile },
      });
      await flush();
    }
    await leaveWalk().catch(() => {});
    gateResult('windows', {
      description: 'where the warm window strips actually are, and whether they read as lit at night from the commons overlook and a garden vantage',
      nightLabel,
      strips: overlookClusters.ok ? {
        meshesWithThatMaterial: overlookClusters.meshesWithThatMaterial,
        totalVertices: overlookClusters.totalVertices,
        boxesImplied: overlookClusters.boxesImplied,
        clusterCount: overlookClusters.clusterCount,
        clusters: overlookClusters.clusters.map(c => ({ centre: c.centre, vertices: c.vertices, boxes: Math.round(c.vertices / 24), distanceFromOverlookM: c.distanceM, onScreenPoints: c.onScreenPoints })),
      } : overlookClusters,
      fromTheCommonsOverlook: {
        note: 'ForumNavigation puts the overview camera at (29,29,36) looking at (0,1,0) for the commons; fov 48 on a 702x972 canvas.',
        verticesProjectingIntoView: overlookPoints.length,
        stripPixels: overlookPoints.length && dayOverlook && nightOverlook ? pointLuma(dayOverlook, nightOverlook, overlookPoints) : { measured: false, reason: 'no-window-strip-vertex-projects-into-the-overlook-view' },
        wholeFrameBrighterAtNight: dayOverlook && nightOverlook ? brighterAtNight(dayOverlook, nightOverlook, [0, .30, 1, .76]) : null,
        nightScreenshot: nightOverviewShot,
      },
      groundVantages: vantages,
    });
  } catch (e) { gateResult('windows', { failure: String(e && e.message || e), screenshot: await shot('v3-windows-failure').catch(() => null) }); }
  await flush();
}

// ----------------------------------------------------------------- shadows
if (want('shadows')) {
  try {
    await page.emulateMedia({ colorScheme: 'light' });
    await bootWorld();
    await district('Commons');
    await page.waitForTimeout(2200);
    const info = await page.evaluate(() => window.__shadowInfo());
    const onCapture = await stableCanvasShot();
    const withShadows = onCapture.buffer;
    const withFile = await saveBuffer('v3-shadows-on', withShadows);
    const off = await page.evaluate(() => window.__setShadows(false));
    await page.waitForTimeout(2200);
    const offCapture = await stableCanvasShot();
    const withoutShadows = offCapture.buffer;
    const withoutFile = await saveBuffer('v3-shadows-off', withoutShadows);
    const restored = await page.evaluate(() => window.__setShadows(true));
    await page.waitForTimeout(1800);
    const infoAfter = await page.evaluate(() => window.__shadowInfo());

    // The shadow camera is +-72 m around the key light, so the ground right in
    // front of the overview camera is where the pass actually lands.
    const groundBand = [0, .55, 1, .95];
    const brighterWithoutShadows = withShadows && withoutShadows ? brighterAtNight(withShadows, withoutShadows, groundBand, 8) : null;
    gateResult('shadows', {
      description: 'the deprecation warning is gone and the shadow pass still does something measurable',
      shadowMap: info,
      shadowMapAfterRestore: infoAfter,
      toggle: { off, restored },
      capture: { shadowsOn: onCapture.zeroPixelCounts, shadowsOff: offCapture.zeroPixelCounts },
      groundWithShadows: withShadows ? regionLuma(withShadows, groundBand) : null,
      groundWithoutShadows: withoutShadows ? regionLuma(withoutShadows, groundBand) : null,
      pixelsBrightenedByRemovingShadows: brighterWithoutShadows,
      screenshots: { on: withFile, off: withoutFile },
      perfWithShadows: await samplePerf(4),
    });
  } catch (e) { gateResult('shadows', { failure: String(e && e.message || e), screenshot: await shot('v3-shadows-failure').catch(() => null) }); }
  await flush();
}

// -------------------------------------------------------------- night shots
if (want('shots')) {
  try {
    await page.emulateMedia({ colorScheme: 'dark' });
    await bootWorld();
    const label = await appearanceLabel();
    await district('Commons');
    await page.waitForTimeout(2600);
    const overview = await shot('v3-world-night');
    const overviewPerf = await samplePerf(4);
    // A tower close-up: stand in the garden nearest the lit spires and look up.
    await district('Council garden');
    await enterWalk();
    await page.waitForTimeout(2200);
    const near = await clusters('Warm Window Emission', 30);
    const pick = near.ok && near.clusters.length ? [...near.clusters].sort((a, b) => a.distanceM - b.distanceM)[0] : null;
    const aim = pick ? await aimAtWorldPoint(pick.centre) : null;
    await page.waitForTimeout(1400);
    const capture = await stableCanvasShot();
    const buffer = capture.buffer;
    const closeUp = await saveBuffer('v3-tower-windows-night', buffer);
    await leaveWalk();
    gateResult('shots', {
      description: 'the two screenshots the job asks for',
      appearance: label,
      overviewScreenshot: overview, overviewPerf,
      towerCloseUp: { screenshot: closeUp, aimedAt: pick ? pick.centre : null, aim, capture: capture.zeroPixelCounts, frame: buffer ? regionLuma(buffer, [0, 0, 1, 1]) : null },
    });
  } catch (e) { gateResult('shots', { failure: String(e && e.message || e) }); }
  await flush();
}

report.console = console_.slice(0, 40);
report.consoleSummary = Object.entries(console_.reduce((acc, c) => {
  const key = `${c.type}: ${c.text.slice(0, 110)}`;
  acc[key] = (acc[key] || 0) + 1;
  return acc;
}, {})).sort((a, b) => b[1] - a[1]).map(([text, count]) => ({ count, text }));
report.shadowDeprecationLines = console_.filter(c => /PCFSoftShadowMap has been deprecated/.test(c.text)).length;
report.pageErrorSummary = Object.entries(pageErrors.reduce((acc, e) => {
  acc[e.message.slice(0, 120)] = (acc[e.message.slice(0, 120)] || 0) + 1;
  return acc;
}, {})).sort((a, b) => b[1] - a[1]).map(([message, count]) => ({ count, message }));
report.finishedAt = new Date().toISOString();
await flush();
await Promise.race([
  (async () => { await page.close().catch(() => {}); await context.close().catch(() => {}); await browser.close().catch(() => {}); })(),
  new Promise(resolve => setTimeout(resolve, 20000)),
]);
console.log(JSON.stringify({
  report: path.relative(repo, reportPath),
  gates: Object.fromEntries(Object.entries(report.gates).map(([k, v]) => [k, v.failure ? `FAILURE: ${v.failure}` : 'recorded'])),
  shadowDeprecationLines: report.shadowDeprecationLines,
}, null, 2));
process.exit(0);
