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
 *   node scripts/probe-black-rect.mjs [url] [label]
 *
 * Env:
 *   ANGLE_BACKEND  metal (default here — the whole point) | gl
 *   SHOTS          how many frames to sample (default 6)
 *   SETTLE_MS      wait before the first frame (default 6000)
 */
import { chromium } from 'playwright';
import { mkdir, writeFile } from 'node:fs/promises';
import { inflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const url = process.env.APP_URL || process.argv[2] || 'http://localhost:5173/ui/forum.html';
const label = process.argv[3] || 'probe';
const backend = process.env.ANGLE_BACKEND || 'metal';
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

/**
 * Blown-out pixels of EITHER polarity, with the bounding box they occupy.
 *
 * This counted pure black only, and that was a hole in the instrument rather
 * than in the theory. On 2026-09-18 Jesse photographed a solid WHITE rectangle
 * over the night world — the same hard-edged, axis-aligned block as the black
 * one — and this probe had reported zero black pixels minutes earlier, which
 * read as "clean" when it only ever meant "not black". A driver that hands
 * back an uninitialised or wrongly-cleared render target can do it in either
 * colour, so the fingerprint is a large run of IDENTICAL extreme pixels, not
 * a particular value.
 *
 * Both polarities are counted separately so a regression says which one it is,
 * and each keeps its own box: reporting one merged box across both would
 * describe a rectangle that does not exist.
 */
function extremeReport(img) {
  const track = () => ({ count: 0, minX: img.width, minY: img.height, maxX: -1, maxY: -1 });
  const black = track();
  const white = track();
  const note = (t, x, y) => {
    t.count++;
    if (x < t.minX) t.minX = x; if (x > t.maxX) t.maxX = x;
    if (y < t.minY) t.minY = y; if (y > t.maxY) t.maxY = y;
  };
  // Only count pixels inside a LONG UNBROKEN HORIZONTAL RUN.
  //
  // Counting every extreme pixel made this useless on a real frame: white card
  // text, button fills and the day world's pale stone put 28,333 white pixels
  // on screen with no defect present at all, and a bounding box spanning the
  // window. The artifact is a solid block, so the thing that distinguishes it
  // from legitimate UI is contiguity — a rectangle produces runs hundreds of
  // pixels wide, and glyphs and gradients do not.
  const MIN_RUN = 200;
  const scan = (isHit, track) => {
    for (let y = 0; y < img.height; y++) {
      let runStart = -1;
      for (let x = 0; x <= img.width; x++) {
        const hit = x < img.width && isHit(y * img.stride + x * img.channels);
        if (hit && runStart < 0) runStart = x;
        if (!hit && runStart >= 0) {
          if (x - runStart >= MIN_RUN) {
            for (let k = runStart; k < x; k++) note(track, k, y);
          }
          runStart = -1;
        }
      }
    }
  };
  const px = img.pixels;
  scan((at) => px[at] === 0 && px[at + 1] === 0 && px[at + 2] === 0, black);
  scan((at) => px[at] === 255 && px[at + 1] === 255 && px[at + 2] === 255, white);
  const box = (t) => (t.maxX < 0 ? null : [t.minX, t.minY, t.maxX, t.maxY]);
  return {
    count: black.count + white.count,
    black: { count: black.count, box: box(black) },
    white: { count: white.count, box: box(white) },
    box: black.count >= white.count ? box(black) : box(white),
  };
}

const browser = await chromium.launch({
  headless: true,
  channel: 'chrome',
  args: [`--use-angle=${backend}`, '--enable-gpu', '--ignore-gpu-blocklist', '--disable-frame-rate-limit', '--disable-gpu-vsync'],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 1000 }, deviceScaleFactor: 1 });
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
await page.waitForTimeout(settleMs);

await mkdir(outDir, { recursive: true });
const results = [];
for (let i = 0; i < shots; i++) {
  // Nudge the camera between frames: the defect sticks to a pose, so a single
  // frame proves nothing either way.
  await page.mouse.move(640, 500);
  await page.mouse.down();
  await page.mouse.move(640 + i * 40, 500 + i * 12, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(700);
  const buffer = await page.screenshot({ type: 'png' });
  const report = extremeReport(decodePng(buffer));
  results.push(report);
  if (report.count > 0) await writeFile(path.join(outDir, `${label}-${backend}-${i}.png`), buffer);
}

await browser.close();

const worst = results.reduce((a, b) => (b.count > a.count ? b : a), results[0]);
const total = results.reduce((sum, r) => sum + r.count, 0);
console.log(JSON.stringify({
  label,
  backend,
  frames: results.length,
  framesWithArtifact: results.filter(r => r.count > 0).length,
  totalExtremePixels: total,
  blackPixels: results.reduce((sum, r) => sum + r.black.count, 0),
  whitePixels: results.reduce((sum, r) => sum + r.white.count, 0),
  worstFrame: worst,
}, null, 2));
