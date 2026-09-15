#!/usr/bin/env node
/*
 * In-app Solar Forum release-gate evidence.
 *
 * Unlike `verify-forum-browser.mjs` (which loads the standalone forum page),
 * this drives the REAL Command Center app served by `vite` and proxied to the
 * running local daemon, and exercises the release gates listed in
 * docs/design/solar-forum/jobs/03-report.md.
 *
 * Authentication uses the app's own browser credential path (`api.ts`
 * `browserToken()`): the daemon bearer token is placed in localStorage under
 * `permagent-daemon-token` before any page script runs. The token is read from
 * the daemon's own secrets file and is NEVER printed, written to the report,
 * or sent anywhere except the loopback daemon this page already talks to.
 *
 * Every prompt this script sends is prefixed "[world-e2e-test]" and is sent to
 * the real conversation, so keep them tiny.
 *
 * Usage (from ui/command-center, with `npm run dev -- --port 5284` running):
 *   node scripts/verify-world-in-app.mjs
 *   APP_URL=http://127.0.0.1:5284/ui/ node scripts/verify-world-in-app.mjs
 *   GATES=route,legacy,ask node scripts/verify-world-in-app.mjs   # subset
 */
import { chromium } from 'playwright';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import { execSync, spawn } from 'node:child_process';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const appUrl = process.env.APP_URL || process.argv[2] || 'http://127.0.0.1:5284/ui/';
const origin = new URL(appUrl).origin;
const shotDir = path.join(repo, 'assets/world/forum/browser');
const reportPath = path.join(repo, 'docs/design/solar-forum/in-app-report.json');
const tokenFile = process.env.PERMAGENT_TOKEN_FILE || path.join(os.homedir(), '.permagent/secrets/daemon_token.json');
const only = (process.env.GATES || '').split(',').map(s => s.trim()).filter(Boolean);
const viewport = { width: 1600, height: 1000 };
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

const report = {
  appUrl,
  viewport,
  startedAt: new Date().toISOString(),
  tokenSource: `${tokenFile} (value never recorded)`,
  notes: [
    'Drives the real Command Center app against the live local daemon.',
    'Every sent prompt is prefixed "[world-e2e-test]".',
  ],
  gates: {},
};

function gateResult(id, patch) {
  report.gates[id] = { ...(report.gates[id] || {}), ...patch, at: new Date().toISOString() };
}
async function flush() {
  report.updatedAt = new Date().toISOString();
  // Backstop: whatever a gate happened to record, and however it was spelled,
  // the live token never reaches the file.
  const serialised = JSON.stringify(report, null, 2).split(token).join('REDACTED');
  await writeFile(reportPath, `${serialised.replace(/([?&]token=)[^&\s"'\\]*/g, '$1REDACTED')}\n`);
}

/** Copied verbatim in behaviour from scripts/verify-forum-browser.mjs so the
 *  non-blank evidence is computed the same way in both harnesses. */
function pngRgbVariance(buffer) {
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

// Real GPU, not swiftshader: under software WebGL this scene renders at ~0.8
// FPS and animation frames arrive 5-16s apart, which makes every Playwright
// actionability wait (visible/enabled/stable) time out. With ANGLE/Metal the
// same page runs at ~97 FPS and the harness behaves like a user's machine.
// Set FORUM_SOFTWARE_GL=1 to reproduce the software path.
const gpuArgs = process.env.FORUM_SOFTWARE_GL === '1'
  ? ['--use-gl=swiftshader', '--enable-unsafe-swiftshader']
  : ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'];
const browser = await chromium.launch({ headless: true, args: gpuArgs });
const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
// The app's own browser credential path: localStorage['permagent-daemon-token'].
await context.addInitScript(([key, value]) => {
  try { localStorage.setItem(key, value); } catch { /* storage blocked */ }
}, ['permagent-daemon-token', token]);
const page = await context.newPage();
// The forum chunk (three.js + a 2.2M-triangle GLB) takes well over 20s to
// mount under software WebGL, so the default 30s is not a safe ceiling here.
page.setDefaultTimeout(90000);
// Redaction, not decoration (job 18 bug 1). `getStreamToken()`
// (`src/lib/streamToken.ts`) puts the master daemon token in the SSE query
// string, and this harness used to redact it on the RESPONSE path only — a
// failed or aborted EventSource (which happens on every reload) wrote the live
// credential straight into the committed report. Ported from
// `verify-world-in-app-v2.mjs`: every URL this file records, from any source,
// goes through the same scrub, console and page-error text included, and
// `flush()` holds a last-resort backstop over the whole serialised report.
const scrubUrl = u => String(u).replace(origin, '').replace(/([?&]token=)[^&\s"']*/g, '$1REDACTED');
const scrubText = t => String(t).replace(/([?&]token=)[^&\s"']*/g, '$1REDACTED');
page.on('pageerror', e => pageErrors.push({ message: scrubText(e.message) }));
page.on('console', m => {
  if (m.type() !== 'error' && m.type() !== 'warning') return;
  console_.push({ type: m.type(), text: scrubText(m.text()).slice(0, 500) });
});
page.on('requestfailed', r => requestFailures.push({ url: scrubUrl(r.url()), error: r.failure()?.errorText || 'failed' }));
page.on('response', r => {
  const u = r.url();
  if (/\/(api|sessions|reply|agent|permagent|events|config|status)\b/.test(u) || /solar-forum\.glb/.test(u)) {
    responses.push({ url: scrubUrl(u), status: r.status(), method: r.request().method() });
  }
});

function recent(filter, n = 12) {
  return responses.filter(r => filter.test(r.url)).slice(-n);
}

async function findWorldWorkspace() {
  const ws = await daemon('/api/workspaces');
  if (ws.status !== 200 || !Array.isArray(ws.body)) return { error: `GET /api/workspaces -> ${ws.status}` };
  const parsed = ws.body.map(w => ({
    id: w.id,
    name: w.name,
    layout: typeof w.layoutJson === 'string' ? JSON.parse(w.layoutJson) : w.layoutJson,
  }));
  const world = parsed.find(w => hasTool(w.layout, 'world'));
  const other = parsed.find(w => w.id !== world?.id);
  return { world, other, all: parsed.map(w => w.name) };
}

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

async function openWorldWorkspace(wsName) {
  const already = await forumVisible();
  if (already.visible) return { clicked: false, ...already };
  await robustClick(page.getByRole('button', { name: wsName, exact: false }).first(), `workspace ${wsName}`);
  await page.waitForFunction(() => [...document.querySelectorAll('.forum-shell')].some(s => s.getBoundingClientRect().width > 1), undefined, { timeout: 90000 });
  return { clicked: true, ...(await forumVisible()) };
}

/** click() also waits for Playwright "stability"; see safeFill for why that
 *  wait cannot settle here. Try a real click, then fall back to a forced one. */
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

async function shot(name, opts = {}) {
  const file = `assets/world/forum/browser/${name}.png`;
  // Software WebGL makes a 2.2M-triangle frame slow to capture; 30s is not enough.
  await page.screenshot({ path: path.join(repo, file), timeout: 120000, ...opts });
  return file;
}

const workspaces = await findWorldWorkspace();
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
const glbResponse = page.waitForResponse(r => r.url().includes('solar-forum.glb'), { timeout: 60000 }).catch(() => null);

// Navigation is unconditional so any subset of gates can run on its own.
await page.goto(appUrl, { waitUntil: 'domcontentloaded', timeout: 30000 });
await page.waitForFunction(() => !document.body.textContent.includes('Loading workspaces'), undefined, { timeout: 60000 }).catch(() => {});
const openedWorld = await openWorldWorkspace(workspaces.world.name).catch(e => ({ error: String(e.message) }));
const glbFirst = await glbResponse;
await page.waitForTimeout(4000);

// ---------------------------------------------------------------- gate: route
if (want('route')) {
  try {
    const opened = openedWorld;
    const glb = glbFirst;
    await page.waitForTimeout(4000);
    // Identity-change counter for the shared perf probe: a new object means a
    // frame budget elapsed, i.e. the frameloop is actually running.
    await page.evaluate(() => {
      window.__gateTicks = 0;
      let last = window.__worldPerf;
      clearInterval(window.__gateTimer);
      window.__gateTimer = setInterval(() => { if (window.__worldPerf !== last) { last = window.__worldPerf; window.__gateTicks++; } }, 100);
    });
    await page.waitForTimeout(3500);
    const perf = await page.evaluate(() => ({ snapshot: window.__worldPerf || null, ticks: window.__gateTicks }));
    const renderer = await page.evaluate(() => { try { const cv = document.createElement('canvas'); const gl = cv.getContext('webgl2') || cv.getContext('webgl'); const d = gl.getExtension('WEBGL_debug_renderer_info'); return d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : 'n/a'; } catch { return 'n/a'; } });
    const canvasBox = await page.locator('.forum-shell canvas').first().boundingBox();
    const sampling = canvasBox ? pngRgbVariance(await page.screenshot({ clip: canvasBox })) : { supported: false, reason: 'no-canvas-box' };
    const file = await shot('in-app-world', { fullPage: false });
    gateResult('route', {
      description: 'world tool renders ForumAppView inside the real app',
      workspace: workspaces.world.name,
      opened,
      glb: glb ? { url: scrubUrl(glb.url()), status: glb.status() } : null,
      perf,
      renderer,
      sampling,
      screenshot: file,
      districts: await page.locator('.forum-districts').first().innerText().catch(() => null),
      appearance: await page.locator('[data-testid="forum-appearance"]').first().textContent().catch(() => null),
      legacyFlagInStorage: await page.evaluate(() => { try { return localStorage.getItem('permagent.world.legacy'); } catch { return 'throw'; } }),
    });
  } catch (e) { gateResult('route', { failure: String(e && e.message || e) }); }
  await flush();
}

// --------------------------------------------------------------- gate: legacy
if (want('legacy')) {
  try {
    await page.evaluate(() => localStorage.setItem('permagent.world.legacy', '1'));
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(3000);
    const opened = await openWorldWorkspace(workspaces.world.name).catch(e => ({ error: String(e.message) }));
    await page.waitForTimeout(5000);
    const forumPresent = await page.evaluate(() => document.querySelectorAll('.forum-shell').length);
    const legacyMarkers = await page.evaluate(() => ({
      canvases: document.querySelectorAll('canvas').length,
      bodyHasWorldText: /WORLD \/ SOLAR FORUM/.test(document.body.innerText),
      text: document.body.innerText.slice(0, 600),
    }));
    const file = await shot('in-app-world-legacy');
    gateResult('legacy', {
      description: "rollback lever: localStorage['permagent.world.legacy']='1' renders the legacy WorldView",
      forumShellCount: forumPresent,
      legacyMarkers,
      opened,
      screenshot: file,
    });
    await page.evaluate(() => localStorage.removeItem('permagent.world.legacy'));
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(3000);
    await openWorldWorkspace(workspaces.world.name);
    await page.waitForTimeout(4000);
    gateResult('legacy', {
      clearedFlag: await page.evaluate(() => localStorage.getItem('permagent.world.legacy')),
      forumBackAfterClear: (await forumVisible()).visible,
    });
  } catch (e) { gateResult('legacy', { failure: String(e && e.message || e) }); }
  await flush();
}

// ------------------------------------------------------------ gate: sovereign
if (want('sovereign')) {
  try {
    const identity = await daemon('/api/agent/identity');
    const noteText = await page.locator('.forum-panel .note').first().innerText();
    const findButton = await page.getByRole('button', { name: /^Find / }).first().textContent();
    const option = await page.locator('#forum-agent option').first().textContent();
    gateResult('sovereign', {
      description: 'the forum shows the configured orchestrator name',
      daemonIdentity: identity.body && typeof identity.body === 'object'
        ? { status: identity.status, first_name: identity.body.first_name, last_name: identity.body.last_name }
        : { status: identity.status },
      forumNote: noteText,
      findButton,
      selectOption: option,
      matches: typeof identity.body === 'object' && identity.body
        ? noteText.includes(identity.body.first_name) && String(findButton).includes(identity.body.first_name)
        : null,
      screenshot: await shot('in-app-sovereign'),
    });
  } catch (e) { gateResult('sovereign', { failure: String(e && e.message || e) }); }
  await flush();
}

// ----------------------------------------------------------------- gate: ask
let lastHitTest = null;
const obstructions = [];

/** What a real user's click would actually land on at this element's centre. */
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

/** fill() waits for the field to receive events; if something is painted over
 *  it the wait never ends, so scroll it to the middle of the panel first and
 *  record what was in the way. */
async function safeFill(selector, value) {
  const el = page.locator(selector).first();
  // Not scrollIntoViewIfNeeded(): that waits for Playwright "stability", and
  // under software WebGL this page can take 5-16s per animation frame, so the
  // stability wait never settles. Scroll in the page instead.
  await el.evaluate(node => node.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(500);
  let probe = await hitTest(selector);
  if (probe.obstructed) {
    await page.locator(selector).first().evaluate(node => node.scrollIntoView({ block: 'center' }));
    await page.waitForTimeout(400);
    probe = await hitTest(selector);
  }
  try {
    await el.fill(value, { timeout: 12000 });
  } catch (e) {
    // Last resort so the gate still reports on the send path itself; the
    // obstruction is recorded above as the finding.
    await el.evaluate((node, v) => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
      setter.call(node, v);
      node.dispatchEvent(new Event('input', { bubbles: true }));
    }, value);
    probe.filledViaEventDispatch = String(e.message).slice(0, 120);
  }
  return probe;
}

async function sendBrief(kind, text, criteria = '') {
  await robustClick(page.getByRole('button', { name: 'Give a brief' }).first(), 'Give a brief tab');
  await page.locator('#forum-kind').selectOption(kind);
  await safeFill('#forum-brief', text);
  if (criteria) await safeFill('#forum-criteria', criteria);
  const button = page.locator('.forum-panel button', { hasText: kind === 'Query' ? /^Ask / : kind === 'Job' ? /^Ask to run job/ : /^Develop capability/ }).first();
  const label = await button.textContent();
  // The send button sits at the very bottom of the scrolling agent desk, so
  // put it in the middle of the panel first — that is also where a real user
  // would have it when they press it.
  await button.evaluate(node => node.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(600);
  // Hit-test: what would a real click at this button's centre actually reach?
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
    // Playwright's actionability wait (visible/enabled/STABLE) does not settle
    // on this page; the hit test above is the evidence that the click lands on
    // the button, so dispatch it at those coordinates.
    lastHitTest.clickMode = 'forced-after-actionability-timeout';
    lastHitTest.actionabilityError = String(e.message).split('\n')[0];
    await button.click({ force: true, timeout: 20000 });
  }
  return label;
}

if (want('ask')) {
  try {
    const before = responses.length;
    const PROMPT = `${TAG} reply with the single word PONG`;
    const label = await sendBrief('Query', PROMPT, 'a one-word reply');
    await page.waitForFunction(() => Boolean(document.querySelector('.forum-transcript')), undefined, { timeout: 60000 });
    // This gate used to wait for /PONG/i ANYWHERE in the transcript — which the
    // echoed prompt ("reply with the single word PONG") satisfies on its own,
    // so it could not fail and its PASS was never evidence that an agent
    // answered (job 18 bug 2). Only what comes AFTER the last echo of the
    // prompt counts, exactly as `verify-world-in-app-v2.mjs` does it.
    const TAIL = 'reply with the single word PONG';
    const afterPrompt = () => page.evaluate(marker => {
      const text = document.querySelector('.forum-transcript')?.innerText || '';
      const at = text.lastIndexOf(marker);
      return at < 0 ? '' : text.slice(at + marker.length);
    }, TAIL);
    let replied = false;
    try {
      await page.waitForFunction(marker => {
        const text = document.querySelector('.forum-transcript')?.innerText || '';
        const at = text.lastIndexOf(marker);
        return at >= 0 && /\bPONG\b/i.test(text.slice(at + marker.length));
      }, TAIL, { timeout: 180000 });
      replied = true;
    } catch { replied = false; }
    await page.waitForFunction(() => !/reply in progress/.test(document.querySelector('.forum-conversation .eyebrow')?.textContent || ''), undefined, { timeout: 120000 }).catch(() => {});
    const transcript = await page.locator('.forum-transcript').innerText();
    const reply = await afterPrompt();
    gateResult('ask', {
      description: 'forum ask control streams a real reply into the existing conversation',
      agentRepliedPong: replied,
      replyAfterPrompt: reply.trim().slice(0, 400),
      replyLooksLikeTransportError: /network error|could not connect|ECONNREFUSED/i.test(reply),
      buttonLabel: label,
      hitTest: lastHitTest,
      connection: await page.locator('.forum-conversation .eyebrow').innerText(),
      transcriptTail: transcript.slice(-900),
      network: responses.slice(before).filter(r => /\/sessions|\/reply/.test(r.url)).slice(0, 12),
      screenshot: await shot('in-app-ask'),
      // The gate passes only on a reply message distinct from the echoed
      // prompt; anything else is recorded as the failure it is.
      ...(replied ? {} : { failure: 'no agent reply containing PONG after the echoed prompt' }),
    });
  } catch (e) {
    gateResult('ask', { failure: String(e && e.message || e), transcriptTail: await page.locator('.forum-transcript').innerText().catch(() => null), screenshot: await shot('in-app-ask-failure').catch(() => null) });
  }
  await flush();
}

// ------------------------------------------------------- gate: job + cancel
if (want('job')) {
  try {
    const goalsBefore = await daemon('/api/goals/active');
    const before = responses.length;
    const label = await sendBrief(
      'Job',
      // Long enough to still be streaming when the cancel control is clicked;
      // a one-word answer finishes in ~2s and cancellation cannot be exercised.
      `${TAG} Do not use any tools and do not create files. Just print the numbers 1 to 60, one per line, nothing else.`,
      'the numbered lines only',
    );
    await page.waitForFunction(() => Boolean(document.querySelector('.forum-transcript')), undefined, { timeout: 60000 });
    // Cancel the in-flight turn through the forum's own control.
    const stop = page.locator('.forum-conversation button', { hasText: 'Stop reply' }).first();
    await stop.waitFor({ state: 'visible', timeout: 60000 });
    gateResult('job', { streamingObservedBeforeCancel: true });
    const liveBefore = await page.locator('.forum-live').first().innerText();
    const stopMode = await robustClick(stop, 'Stop reply');
    await page.waitForFunction(() => !/reply in progress/.test(document.querySelector('.forum-conversation .eyebrow')?.textContent || ''), undefined, { timeout: 60000 }).catch(() => {});
    await page.waitForTimeout(3000);
    const goalsAfter = await daemon('/api/goals/active');
    gateResult('job', {
      description: 'job brief path and cancellation through the forum conversation',
      buttonLabel: label,
      hitTest: lastHitTest,
      goalsBefore: goalsBefore.body?.goals?.map?.(g => ({ id: g.id, title: g.title, state: g.state })) ?? goalsBefore,
      goalsAfter: goalsAfter.body?.goals?.map?.(g => ({ id: g.id, title: g.title, state: g.state })) ?? goalsAfter,
      forumLiveBefore: liveBefore,
      forumLiveAfter: await page.locator('.forum-live').first().innerText(),
      stopClickMode: stopMode,
      stopVisibleAfterCancel: await page.locator('.forum-conversation button', { hasText: 'Stop reply' }).count(),
      connection: await page.locator('.forum-conversation .eyebrow').innerText(),
      cancelNetwork: responses.slice(before).filter(r => /cancel/.test(r.url)),
      transcriptTail: (await page.locator('.forum-transcript').innerText()).slice(-900),
      screenshot: await shot('in-app-job-cancel'),
    });
  } catch (e) { gateResult('job', { failure: String(e && e.message || e), screenshot: await shot('in-app-job-failure').catch(() => null) }); }
  await flush();
}

// ------------------------------------------------------------ gate: approvals
if (want('approvals')) {
  try {
    const inbox = await daemon('/api/decisions').catch(e => ({ status: 'error', body: String(e) }));
    const present = await page.locator('[data-testid="chat-pending-decisions"]').count();
    gateResult('approvals', {
      description: 'pending approvals render inside the forum conversation (ChatPendingDecisions)',
      daemonPendingDecisions: inbox,
      pendingDecisionsRendered: present,
      note: present === 0 ? 'No pending approval existed during this run; nothing was approved or rejected.' : 'A pending decision card was present; it was NOT acted on.',
      screenshot: await shot('in-app-approvals'),
    });
  } catch (e) { gateResult('approvals', { failure: String(e && e.message || e) }); }
  await flush();
}

// --------------------------------------------------------------- gate: skills
if (want('skills')) {
  try {
    const apiSkills = await daemon('/permagent/skills');
    const forumText = await page.locator('section[aria-label="Live capabilities"]').first().innerText();
    const savedCount = /(\d+)\s+saved skills/.exec(forumText)?.[1] ?? null;
    const workerCount = /(\d+)\s+registered workers/.exec(forumText)?.[1] ?? null;
    await robustClick(page.locator('section[aria-label="Live capabilities"] button', { hasText: 'Open Skills' }).first(), 'Open Skills');
    await page.waitForTimeout(2500);
    const panelOpen = await page.evaluate(() => /skills/i.test(document.body.innerText) && !document.querySelector('.forum-shell')?.getBoundingClientRect().width);
    const panelShot = await shot('in-app-skills-panel');
    const panelText = await page.evaluate(() => document.body.innerText.slice(0, 1200));
    gateResult('skills', {
      description: 'saved-skills list matches /permagent/skills, and Open Skills reaches the app Skills panel',
      apiStatus: apiSkills.status,
      apiSkillCount: Array.isArray(apiSkills.body) ? apiSkills.body.length : null,
      apiSkillNames: Array.isArray(apiSkills.body) ? apiSkills.body.slice(0, 8).map(s => s.name) : null,
      forumCapabilitiesText: forumText.slice(0, 900),
      forumSavedSkills: savedCount,
      forumRegisteredWorkers: workerCount,
      matches: savedCount !== null && Array.isArray(apiSkills.body) ? Number(savedCount) === apiSkills.body.length : null,
      skillsPanelOpened: panelOpen,
      skillsPanelText: panelText,
      screenshot: panelShot,
    });
    // back to the forum
    await robustClick(page.getByRole('button', { name: workspaces.world.name, exact: false }).first(), 'World tab');
    await page.waitForTimeout(2500);
  } catch (e) { gateResult('skills', { failure: String(e && e.message || e), screenshot: await shot('in-app-skills-failure').catch(() => null) }); }
  await flush();
}

// ------------------------------------------------------------ gate: reconnect
function portListening(port) {
  return new Promise(resolve => {
    const socket = net.connect({ host: '127.0.0.1', port }, () => { socket.destroy(); resolve(true); });
    socket.on('error', () => resolve(false));
    setTimeout(() => { socket.destroy(); resolve(false); }, 1000);
  });
}

if (want('reconnect')) {
  try {
    const port = Number(new URL(appUrl).port || 80);
    const statusText = async () => (await page.locator('.forum-conversation .eyebrow').first().innerText().catch(() => null));
    const waitForStatus = async (re, ms) => {
      const deadline = Date.now() + ms;
      let last = null;
      while (Date.now() < deadline) {
        last = await statusText();
        if (last && re.test(last)) return last;
        await page.waitForTimeout(1000);
      }
      return last;
    };
    await openWorldWorkspace(workspaces.world.name).catch(() => {});
    await robustClick(page.getByRole('button', { name: 'Conversation' }).first(), 'Conversation tab').catch(() => {});
    await page.waitForTimeout(1500);
    let established = await statusText();
    let seeded = false;
    if (!/^connected/.test(String(established))) {
      // Nothing in the app opens the event channel until a conversation is
      // actually used (ChatView's mount effect, or the forum's own ask), so
      // establish it the way the forum does.
      await sendBrief('Query', `${TAG} PING — reply with the single word PONG`);
      seeded = true;
      established = await waitForStatus(/^connected/, 120000);
      await waitForStatus(/^connected$/, 120000);
    }

    // Phase A — offline emulation. Recorded for completeness: an EventSource
    // that is already open is not re-issued, so this does not by itself prove
    // the stream dropped.
    await context.setOffline(true);
    const offlineProof = await page.evaluate(() => fetch('/status').then(r => `ok ${r.status}`).catch(e => `ERR ${e.message}`));
    const statusOffline = await waitForStatus(/^(disconnected|connecting)/, 30000);
    await context.setOffline(false);

    // Phase B — the real drop: the dev server proxies /sessions/*/events to the
    // daemon, so stopping it closes the live stream socket.
    const before = await statusText();
    const pids = execSync(`lsof -ti tcp:${port} || true`).toString().trim().split(/\s+/).filter(Boolean);
    for (const pid of pids) { try { process.kill(Number(pid), 'SIGKILL'); } catch { /* already gone */ } }
    await page.waitForTimeout(2000);
    const serverDown = !(await portListening(port));
    const statusAfterStop = await waitForStatus(/^(disconnected|connecting)/, 60000);
    const canvasWhileDown = await forumVisible();
    const downShot = await shot('in-app-reconnect-server-stopped');

    const child = spawn('npm', ['run', 'dev', '--', '--host', '127.0.0.1', '--port', String(port)], {
      cwd: path.join(repo, 'ui/command-center'), detached: true, stdio: 'ignore',
    });
    child.unref();
    let up = false;
    for (let i = 0; i < 60 && !up; i++) { await page.waitForTimeout(1000); up = await portListening(port); }
    // Vite's own HMR client reloads the page once its server is back, so the
    // app restarts here; recovery means the forum route comes back and the
    // conversation reconnects when it is used again.
    await page.waitForTimeout(12000);
    const reopened = await openWorldWorkspace(workspaces.world.name).catch(e => ({ error: String(e.message) }));
    await robustClick(page.getByRole('button', { name: 'Conversation' }).first(), 'Conversation tab').catch(() => {});
    await page.waitForTimeout(1500);
    const statusAfterReload = await statusText();
    if (!/^connected/.test(String(statusAfterReload))) {
      await sendBrief('Query', `${TAG} PING after reconnect — reply with the single word PONG`);
    }
    const statusRecovered = await waitForStatus(/^connected/, 180000);
    gateResult('reconnect', {
      description: 'event channel dropped (dev-server proxy killed) and restored; offline emulation recorded separately',
      connectionSeededBySendingAQuery: seeded,
      statusBefore: established,
      offlineEmulation: { pageFetchWhileOffline: offlineProof, statusWhileOffline: statusOffline },
      statusBeforeServerStop: before,
      devServerPidsKilled: pids.length,
      devServerPortClosed: serverDown,
      statusAfterServerStop: statusAfterStop,
      forumStillRenderingWhileDisconnected: canvasWhileDown,
      devServerBackUp: up,
      forumAfterServerRestart: reopened,
      statusAfterViteReload: statusAfterReload,
      statusAfterServerRestart: statusRecovered,
      recovered: /^connected/.test(String(statusRecovered)),
      screenshots: [downShot, await shot('in-app-reconnect-recovered')],
      sseResponses: recent(/\/events/, 8),
    });
  } catch (e) { gateResult('reconnect', { failure: String(e && e.message || e) }); }
  await flush();
}

// ------------------------------------------------------- gate: remount + keys
if (want('remount')) {
  try {
    await openWorldWorkspace(workspaces.world.name).catch(() => {});
    await page.waitForTimeout(2000);
    // enter walk mode, so a leaked key listener would be observable
    await robustClick(page.locator('.forum-play button').first(), 'Walk the world');
    await page.waitForTimeout(1500);
    const controlsWalking = await page.locator('.forum-controls').first().innerText();
    await page.evaluate(() => {
      window.__gateTicks = 0;
      let last = window.__worldPerf;
      clearInterval(window.__gateTimer);
      window.__gateTimer = setInterval(() => { if (window.__worldPerf !== last) { last = window.__worldPerf; window.__gateTicks++; } }, 100);
    });
    await page.waitForTimeout(3500);
    const ticksVisible = await page.evaluate(() => window.__gateTicks);

    const otherName = workspaces.other?.name;
    let switched = null;
    if (otherName) {
      await robustClick(page.getByRole('button', { name: otherName, exact: false }).first(), `workspace ${otherName}`);
      await page.waitForTimeout(2500);
      switched = otherName;
    }
    const hiddenState = await page.evaluate(() => ({
      forumBox: [...document.querySelectorAll('.forum-shell')].map(s => { const b = s.getBoundingClientRect(); return [Math.round(b.width), Math.round(b.height)]; }),
      pointerLock: document.pointerLockElement ? document.pointerLockElement.tagName : null,
      controls: [...document.querySelectorAll('.forum-controls')].map(c => c.textContent),
    }));
    await page.evaluate(() => { window.__gateTicks = 0; });
    // type WASD on the other workspace
    for (const key of ['w', 'a', 's', 'd', 'w', 'w']) {
      await page.keyboard.press(key);
      await page.waitForTimeout(120);
    }
    await page.waitForTimeout(3000);
    const ticksHidden = await page.evaluate(() => window.__gateTicks);
    const perfAfterHidden = await page.evaluate(() => window.__worldPerf || null);
    const awayShot = await shot('in-app-other-workspace');

    await robustClick(page.getByRole('button', { name: workspaces.world.name, exact: false }).first(), 'World tab');
    await page.waitForTimeout(4000);
    await page.evaluate(() => { window.__gateTicks = 0; });
    await page.waitForTimeout(3500);
    const ticksBack = await page.evaluate(() => window.__gateTicks);
    const backVisible = await forumVisible();
    const canvasBox = await page.locator('.forum-shell canvas').first().boundingBox();
    const sampling = canvasBox ? pngRgbVariance(await page.screenshot({ clip: canvasBox })) : { supported: false, reason: 'no-canvas-box' };
    gateResult('remount', {
      description: 'switch away and back: hidden panel stops rendering, keys do not leak, forum keeps rendering',
      controlsWhileWalking: controlsWalking,
      perfTicksWhileVisible: ticksVisible,
      switchedTo: switched,
      hiddenState,
      perfTicksWhileHiddenAfterWASD: ticksHidden,
      perfSnapshotWhileHidden: perfAfterHidden,
      perfTicksAfterReturn: ticksBack,
      canvasAfterReturn: backVisible,
      samplingAfterReturn: sampling,
      screenshots: [awayShot, await shot('in-app-world-return')],
    });
  } catch (e) { gateResult('remount', { failure: String(e && e.message || e), screenshot: await shot('in-app-remount-failure').catch(() => null) }); }
  await flush();
}

report.obstructions = obstructions;
report.console = console_.slice(0, 120);
report.pageErrors = pageErrors;
report.requestFailures = requestFailures.slice(0, 40);
report.daemonHttpFailures = responses.filter(r => r.status >= 400);
report.finishedAt = new Date().toISOString();
await flush();
await page.close().catch(() => {});
await context.close().catch(() => {});
await browser.close().catch(() => {});
console.log(JSON.stringify({ report: path.relative(repo, reportPath), gates: Object.fromEntries(Object.entries(report.gates).map(([k, v]) => [k, v.failure ? `FAILURE: ${v.failure}` : 'recorded'])) }, null, 2));
