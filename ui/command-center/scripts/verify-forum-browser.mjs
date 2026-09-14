#!/usr/bin/env node
/*
 * Browser-only Solar Forum smoke evidence.
 *
 * This intentionally does not mock the daemon, create sessions, send briefs,
 * or claim that disconnected feeds are successful work. The page is allowed
 * to render with unavailable authenticated feeds; that state is recorded apart
 * from render, asset, WebGL, and interaction failures.
 */
import { chromium } from 'playwright';
import { mkdir, writeFile } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const url = process.env.FORUM_URL || process.argv[2] || 'http://127.0.0.1:5285/ui/forum.html';
const browserDir = path.join(repo, 'assets/world/forum/browser');
const reportPath = path.join(repo, 'docs/design/solar-forum/browser-report.json');
const viewport = { width: 1440, height: 1000 };
const report = {
  url,
  viewport,
  startedAt: new Date().toISOString(),
  notes: ['No daemon state was mocked and no task/job/capability request was sent.'],
  themes: [],
};

await mkdir(browserDir, { recursive: true });

async function writePartialReport(phase) {
  report.phase = phase;
  report.updatedAt = new Date().toISOString();
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ phase, themes: report.themes.map(theme => ({ theme: theme.theme, phase: theme.phase || null, failure: theme.failure || null })) }));
}

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms); });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function classifyNetworkFailure(failure) {
  const requestUrl = failure.url || '';
  return /(?:\/api\/|\/events|\/permagent\/)/.test(requestUrl) ? 'daemon' : 'render-assets';
}

function isShaderFailure(text) {
  return /shader\s+error|fragment shader is not compiled|vertex shader is not compiled|program not valid|invalid_operation|context lost|compile(?:d|r)?\s+error|link(?:ed|er)?\s+error|validate_status false/i.test(text);
}

async function inspectCanvas(page) {
  return page.evaluate(() => {
    const canvas = document.querySelector('canvas');
    if (!canvas) return { present: false, visible: false, width: 0, height: 0, rgbVariance: 0, framebufferSampling: 'unsupported' };
    const box = canvas.getBoundingClientRect();
    const gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
    let rgbVariance = 0;
    let framebufferSampling = 'unsupported';
    if (gl && canvas.width > 0 && canvas.height > 0) {
      // PreserveDrawingBuffer is not guaranteed. This is only a best-effort
      // sample of the current frame; the locator PNG below is authoritative.
      const values = [];
      for (let row = 0; row < 8; row++) {
        for (let col = 0; col < 8; col++) {
          const pixel = new Uint8Array(4);
          const x = Math.min(canvas.width - 1, Math.floor((col + .5) * canvas.width / 8));
          const y = Math.min(canvas.height - 1, Math.floor((row + .5) * canvas.height / 8));
          gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
          values.push(pixel[0], pixel[1], pixel[2]);
        }
      }
      const mean = values.reduce((sum, value) => sum + value, 0) / values.length;
      rgbVariance = values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / values.length;
      framebufferSampling = 'rgb-distributed-current-frame';
    }
    return {
      present: true,
      visible: box.width > 0 && box.height > 0,
      width: box.width,
      height: box.height,
      drawingBuffer: [canvas.width, canvas.height],
      webgl: Boolean(gl),
      rgbVariance,
      framebufferSampling,
      glError: gl ? gl.getError() : null,
    };
  });
}

function pngRgbVariance(buffer) {
  // Decode the locator PNG without an image package. Playwright emits an
  // 8-bit, non-interlaced RGB/RGBA PNG for this screenshot.
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

async function captureTheme({ name, colorScheme }) {
  const channel = process.env.PLAYWRIGHT_CHANNEL;
  const browser = await chromium.launch(channel ? { channel, headless: true } : { headless: true });
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  await page.emulateMedia({ colorScheme, reducedMotion: 'no-preference' });
  const errors = [];
  const daemonFailures = [];
  const assetFailures = [];
  const shaderErrors = [];
  page.on('pageerror', error => errors.push({ kind: 'pageerror', message: error.message }));
  page.on('console', message => {
    if (message.type() !== 'error' && message.type() !== 'warning') return;
    const text = message.text();
    const item = { kind: `console-${message.type()}`, message: text };
    errors.push(item);
    if (isShaderFailure(text)) shaderErrors.push(item);
  });
  page.on('requestfailed', request => {
    const failure = { url: request.url(), error: request.failure()?.errorText || 'request failed' };
    (classifyNetworkFailure(failure) === 'daemon' ? daemonFailures : assetFailures).push(failure);
  });
  page.on('response', response => {
    const failure = { url: response.url(), status: response.status() };
    if (response.status() >= 400 && classifyNetworkFailure(failure) === 'daemon') daemonFailures.push(failure);
  });

  const result = { theme: name, colorScheme, screenshot: path.join('assets/world/forum/browser', `solar-forum-${name}.png`), errors, shaderErrors, daemonFailures, assetFailures };
  report.themes.push(result);
  await writePartialReport(`${name}:browser-start`);
  try {
    const assetResponse = page.waitForResponse(response => response.url().includes('solar-forum.glb') && response.ok(), { timeout: 30000 }).catch(() => null);
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    result.phase = 'page-loaded'; await writePartialReport(`${name}:page-loaded`);
    await page.waitForSelector('canvas', { state: 'attached', timeout: 30000 });
    await page.waitForFunction(() => {
      const canvas = document.querySelector('canvas');
      const box = canvas?.getBoundingClientRect();
      return Boolean(canvas && box && box.width > 100 && box.height > 100);
    }, undefined, { timeout: 30000 });
    await page.waitForTimeout(2500);
    result.assetLoaded = Boolean(await assetResponse);
    if (!result.assetLoaded) throw new Error('Solar Forum GLB did not produce a successful browser response');
    result.phase = 'asset-ready'; await writePartialReport(`${name}:asset-ready`);
    result.appearance = await page.locator('[data-testid="forum-appearance"]').textContent();
    result.canvas = await inspectCanvas(page);
    if (!result.canvas.webgl || !result.canvas.visible || result.canvas.width < 100 || result.canvas.height < 100 || result.canvas.glError) {
      throw new Error(`WebGL canvas is not render-ready: ${JSON.stringify(result.canvas)}`);
    }
    const canvasBox = await page.locator('canvas').boundingBox();
    if (!canvasBox) throw new Error('WebGL canvas lost its layout box before screenshot evidence');
    const canvasPng = await page.screenshot({ clip: canvasBox });
    result.screenshotSampling = pngRgbVariance(canvasPng);
    if (!result.screenshotSampling.supported) result.samplingNote = 'PNG RGB sampling unsupported; framebuffer result is best effort';
    else if (result.screenshotSampling.rgbVariance === 0) throw new Error('Canvas PNG RGB samples are uniformly blank');
    result.phase = 'canvas-ready'; await writePartialReport(`${name}:canvas-ready`);

    // Safe UI smoke only: these controls do not send a request or create work.
    await page.getByRole('navigation', { name: 'World places' }).getByRole('button', { name: 'Gallery', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.forum-location span')?.textContent === 'GALLERY');
    result.district = await page.locator('.forum-location span').textContent();
    const findButton = page.getByRole('button', { name: /^Find / }).first();
    await findButton.click();
    result.sovereign = {
      button: await findButton.textContent(),
      selected: await page.locator('#forum-agent').inputValue(),
    };
    await withTimeout(page.screenshot({ path: path.join(repo, result.screenshot), fullPage: true }), 30000, `${name} page screenshot`);
    result.screenshotCaptured = true;
    result.phase = 'interactions-and-screenshot'; await writePartialReport(`${name}:screenshot-ready`);
    result.feed = await page.locator('.forum-live').textContent().catch(() => null);
    result.disconnectedFeed = Boolean(result.feed && /unavailable|connecting/i.test(result.feed));
  } catch (error) {
    result.failure = error instanceof Error ? error.message : String(error);
  } finally {
    if (!result.screenshotCaptured) {
      try {
        const canvas = page.locator('canvas');
        const box = await canvas.boundingBox();
        if (box) {
          const png = await withTimeout(page.screenshot({ clip: box }), 15000, `${name} failure canvas screenshot`);
          result.failureScreenshotSampling = pngRgbVariance(png);
        }
        await withTimeout(page.screenshot({ path: path.join(repo, result.screenshot), fullPage: true }), 30000, `${name} failure page screenshot`);
        result.screenshotCaptured = true;
      } catch (screenshotError) {
        result.screenshotCaptureFailure = screenshotError instanceof Error ? screenshotError.message : String(screenshotError);
      }
    }
    try { await withTimeout(page.close(), 5000, `${name} page close`); } catch (error) { result.pageCloseFailure = error.message; }
    try { await withTimeout(browser.close(), 5000, `${name} browser close`); } catch (error) {
      result.browserCloseFailure = error.message;
      // Playwright's Browser normally owns the child process internally. Some
      // versions expose it for emergency cleanup; kill only that owned child.
      const ownedProcess = typeof browser.process === 'function' ? browser.process() : browser._initializer?.process;
      if (ownedProcess && typeof ownedProcess.kill === 'function') ownedProcess.kill('SIGTERM');
    }
  }
  await writePartialReport(`${name}:complete`);
  return result;
}

for (const theme of [{ name: 'day', colorScheme: 'light' }, { name: 'night', colorScheme: 'dark' }]) await captureTheme(theme);
report.finishedAt = new Date().toISOString();
report.renderFailures = report.themes.filter(theme => theme.failure || theme.assetFailures.length || theme.shaderErrors.length || theme.errors.some(error => error.kind === 'pageerror'));
report.daemonUnavailable = report.themes.filter(theme => theme.disconnectedFeed || theme.daemonFailures.length);
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify({ report: path.relative(repo, reportPath), themes: report.themes.map(({ theme, screenshot, failure, disconnectedFeed }) => ({ theme, screenshot, failure: failure || null, disconnectedFeed: Boolean(disconnectedFeed) })) }, null, 2));
if (report.renderFailures.length) process.exitCode = 1;
