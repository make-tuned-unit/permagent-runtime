// The rectangle detector shared by `scripts/probe-black-rect.mjs` and the
// system-WKWebView probe's frames (`--analyze`). It is here, not in scripts/,
// because vitest only runs `src/**`, and an instrument whose threshold nobody
// tests is how a black night sky got reported as a black rectangle at DPR 2.

/** A decoded 8-bit RGB(A) frame: `channels` is 3 or 4, `stride` is bytes per row. */
export interface DecodedImage { width: number; height: number; channels: number; stride: number; pixels: Uint8Array }
export type Box = [number, number, number, number] | null;
export interface ExtremeReport { count: number; black: { count: number; box: Box }; white: { count: number; box: Box }; box: Box }
interface Track { count: number; minX: number; minY: number; maxX: number; maxY: number }

/**
 * Blown-out pixels of EITHER polarity, with the bounding box they occupy.
 *
 * This counted pure black only, and that was a hole in the instrument rather
 * than in the theory. On 2026-09-18 the user photographed a solid WHITE rectangle
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
export function extremeReport(img: DecodedImage, scale = 1): ExtremeReport {
  const track = (): Track => ({ count: 0, minX: img.width, minY: img.height, maxX: -1, maxY: -1 });
  const black = track();
  const white = track();
  const note = (t: Track, x: number, y: number) => {
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
  //
  // The run is 200 CSS pixels, so it scales with the device pixel ratio: at
  // DPR 2 the pure-black night sky between two stars is easily 200 DEVICE
  // pixels wide, and an unscaled threshold reported that as a black rectangle.
  const MIN_RUN = Math.round(200 * scale);
  const scan = (isHit: (at: number) => boolean, track: Track) => {
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
  scan((at: number) => px[at] === 0 && px[at + 1] === 0 && px[at + 2] === 0, black);
  scan((at: number) => px[at] === 255 && px[at + 1] === 255 && px[at + 2] === 255, white);
  const box = (t: Track): Box => (t.maxX < 0 ? null : [t.minX, t.minY, t.maxX, t.maxY]);
  return {
    count: black.count + white.count,
    black: { count: black.count, box: box(black) },
    white: { count: white.count, box: box(white) },
    box: black.count >= white.count ? box(black) : box(white),
  };
}

export interface ClipReport {
  /** Pixels at exact 255,255,255: every channel clipped. */
  white: number;
  /** Pixels with at least one channel at 255: partially clipped hue. */
  anyChannel: number;
  /** The largest 4-connected blob of pure white, and its box. */
  largestWhite: { count: number; box: Box };
}

/**
 * How much of the frame is clipped, and how big the biggest flat-white shape is.
 *
 * The run detector above only fires on a block at least 200 CSS px wide, and
 * the white block the user saw over the night sky on 2026-09-18 never reproduced
 * to that size in a probe. What DID show on every night frame was the cause
 * underneath it: the gas giant and the moon clipping to flat 255 because the
 * composer drew linear HDR with no view transform (AgX is dropped as soon as
 * EffectComposer mounts). Any surface that crosses 1.0 in every channel turns
 * into a hard-edged white polygon, and the size of the largest such blob is the
 * measurable proxy for "a white block beside the gas giant". This is what the
 * A/B table in ForumPostProcessing.tsx is counted in.
 */
export function clipReport(img: DecodedImage, region?: [number, number, number, number]): ClipReport {
  const { width, height, stride, channels, pixels: px } = img;
  // `region` is [x0, y0, x1, y1] exclusive, in image pixels: the probe passes
  // the canvas rect so the Agent Desk's white text is not counted as sky.
  const [rx0, ry0, rx1, ry1] = region ?? [0, 0, width, height];
  const isWhite = new Uint8Array(width * height);
  let white = 0;
  let anyChannel = 0;
  for (let y = Math.max(0, ry0); y < Math.min(height, ry1); y++) {
    for (let x = Math.max(0, rx0); x < Math.min(width, rx1); x++) {
      const at = y * stride + x * channels;
      const r = px[at] === 255, g = px[at + 1] === 255, b = px[at + 2] === 255;
      if (r || g || b) anyChannel++;
      if (r && g && b) { white++; isWhite[y * width + x] = 1; }
    }
  }
  // Iterative flood fill: a recursive one blows the stack on a real block.
  let best = { count: 0, box: null as Box };
  const stack: number[] = [];
  for (let start = 0; start < isWhite.length; start++) {
    if (isWhite[start] !== 1) continue;
    isWhite[start] = 2;
    stack.push(start);
    let count = 0, minX = width, minY = height, maxX = -1, maxY = -1;
    while (stack.length) {
      const i = stack.pop() as number;
      const x = i % width, y = (i - x) / width;
      count++;
      if (x < minX) minX = x; if (x > maxX) maxX = x;
      if (y < minY) minY = y; if (y > maxY) maxY = y;
      if (x > 0 && isWhite[i - 1] === 1) { isWhite[i - 1] = 2; stack.push(i - 1); }
      if (x < width - 1 && isWhite[i + 1] === 1) { isWhite[i + 1] = 2; stack.push(i + 1); }
      if (y > 0 && isWhite[i - width] === 1) { isWhite[i - width] = 2; stack.push(i - width); }
      if (y < height - 1 && isWhite[i + width] === 1) { isWhite[i + width] = 2; stack.push(i + width); }
    }
    if (count > best.count) best = { count, box: [minX, minY, maxX, maxY] };
  }
  return { white, anyChannel, largestWhite: best };
}

export type ChainEffect = 'Bloom' | 'ToneMapping' | 'HueSaturation' | 'BrightnessContrast' | 'unknown';
export interface ComposerChain {
  /** The `multisampling` prop as written in the bundle, e.g. "0". */
  multisampling: string;
  effects: ChainEffect[];
  /** True when the ToneMapping effect's mode is AgX. */
  agx: boolean;
}

/**
 * Every EffectComposer chain written into a BUILT bundle chunk, read off the
 * minified source.
 *
 * Why this is here: on 2026-09-19 the white-block fix could not be confirmed in
 * Permagent.app (the screen was locked), and every pixel probe so far had
 * rendered `ui/forum.html` from the vite DEV server — a page that is not in
 * `dist/` at all. So "is the fix in the app the user launches?" had only ever been
 * answered by file dates, which proved nothing once before (see the app
 * install provenance note). The Tauri binary names each embedded asset in
 * plain text, and vite names each chunk by a hash of its content, so a chunk
 * name found in the binary AND in `dist/assets` is the same bytes the app
 * serves. This reads the composer chain out of those bytes.
 *
 * Minified identifiers change every build; prop names do not. Each effect is
 * recognised by the props only it takes: Bloom by `luminanceThreshold`,
 * ToneMapping by `mode`, HueSaturation by `saturation`, BrightnessContrast by
 * `contrast`/`brightness`. AgX is `mode:<ns>.AGX`, or the enum's number if a
 * future bundler inlines it (`agxValue`, postprocessing's ToneMappingMode.AGX).
 */
export function composerChains(source: string, agxValue = 7): ComposerChain[] {
  const chains: ComposerChain[] = [];
  const head = /multisampling:\s*([\w.!$]+)\s*,\s*children:\s*\[/g;
  for (let m = head.exec(source); m; m = head.exec(source)) {
    // The children array, up to its matching bracket.
    let depth = 1, end = head.lastIndex;
    while (end < source.length && depth > 0) {
      const ch = source[end++];
      if (ch === '[') depth++;
      else if (ch === ']') depth--;
    }
    const children = source.slice(head.lastIndex, end - 1);
    let agx = false;
    const effects: ChainEffect[] = [];
    // Effect props here are flat objects: no nested braces to balance.
    for (const [, props] of children.matchAll(/\{([^{}]*)\}/g)) {
      if (/\bluminanceThreshold\s*:/.test(props)) effects.push('Bloom');
      else if (/\bmode\s*:/.test(props)) {
        effects.push('ToneMapping');
        if (new RegExp(`\\bmode\\s*:\\s*(?:[\\w$]+\\.AGX\\b|${agxValue}\\b)`).test(props)) agx = true;
      } else if (/\bsaturation\s*:/.test(props)) effects.push('HueSaturation');
      else if (/\b(?:contrast|brightness)\s*:/.test(props)) effects.push('BrightnessContrast');
      else effects.push('unknown');
    }
    chains.push({ multisampling: m[1], effects, agx });
  }
  return chains;
}

/**
 * The forum chain as ForumPostProcessing.tsx ships it: single-sampled (the
 * ANGLE/Metal black rectangle), and AgX after Bloom and before the grade (the
 * white block). Anything else in a built chunk means the fix is not in it.
 */
export function isShippedForumChain(chain: ComposerChain): boolean {
  return chain.multisampling === '0'
    && chain.agx
    && chain.effects.join('>') === 'Bloom>ToneMapping>HueSaturation>BrightnessContrast';
}

/**
 * The `assets/*.js` names a Tauri binary embeds. They sit in the binary as
 * plain text (the asset BODIES are compressed, which is why `strings` on the
 * binary cannot find code, but the keys are not).
 */
export function embeddedAssetNames(binary: string): string[] {
  return [...new Set(binary.match(/assets\/[\w.-]+\.js/g) ?? [])].sort();
}
