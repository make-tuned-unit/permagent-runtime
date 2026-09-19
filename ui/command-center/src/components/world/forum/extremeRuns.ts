// The rectangle detector shared by `scripts/probe-black-rect.mjs` and the
// system-WKWebView probe's frames (`--analyze`). It is here, not in scripts/,
// because vitest only runs `src/**`, and an instrument whose threshold nobody
// tests is how a black night sky got reported as a black rectangle at DPR 2.

/** A decoded 8-bit RGB(A) frame: `channels` is 3 or 4, `stride` is bytes per row. */
export interface DecodedImage { width: number; height: number; channels: number; stride: number; pixels: Uint8Array }
export type Box = [number, number, number, number] | null;
export interface Polarity {
  /** Every pixel in a long run, anywhere in the frame. */
  count: number;
  /**
   * The box of the one connected block this polarity is judged by: the
   * largest solid block if there is one, otherwise the largest cluster.
   */
  box: Box;
  /** That block's count / its box area: 1 is a solid block, a crushed textured surface is far below. */
  fill: number;
  /** Some block is solid: at least MIN_BLOCK_ROWS tall and filling at least SOLID_FILL of its own box. */
  solid: boolean;
  /** How many separate clusters of vertically touching runs there are. */
  blocks: number;
  /** The union box of every run: where the pixels are, NOT a shape. */
  extent: Box;
}
export interface ExtremeReport { count: number; black: Polarity; white: Polarity; box: Box; /** Either polarity is a solid block: the defect fingerprint. */ solid: boolean }
/**
 * How full a block's box must be to count as a rectangle.
 *
 * Why: on 2026-09-19 the probe was pointed at the World in the layout the
 * user's app actually has (desk and rail closed, `--app-state`), and it
 * reported a "black rectangle" on 4 of 4 night frames: 9,876 px in a
 * 514x161 box. It was the far tower facade on the left, a dark texture the
 * grade takes to exact 0 in long broken runs: 12% of its box. The real
 * rectangle the driver drew (job 19, 71,302 px at [437,0]-[776,250]) filled
 * 84% of its box. Half separates them with room either side.
 *
 * The fill is measured PER BLOCK (runs that touch on adjacent rows), never
 * over one box around every run in the frame. A frame-wide box let that same
 * crushed facade, 660 px below the job-19 rectangle, dilute the rectangle to
 * fill 0.221 and report solid:false in exactly the user's layout, which is
 * the instrument going blind to the defect it exists to catch.
 */
export const SOLID_FILL = 0.5;
/**
 * The fewest rows (CSS px) a block needs before it can be a rectangle. A lone
 * run fills its own 1-row box completely, so without a height floor every
 * isolated crushed facade row would read as a solid block. Measured on the
 * four --app-state night frames of 2026-09-19 (DPR 2): the facade breaks into
 * 7 clusters, the tallest 7 device rows (3.5 CSS px) at fill 0.859, so fill
 * alone cannot tell it from a block. 24 CSS px is ~7x that and ~10x under
 * the smallest block ever reported (job 19: 251 rows).
 */
export const MIN_BLOCK_ROWS = 24;
interface Run { y: number; x0: number; x1: number }
interface Block { count: number; minX: number; minY: number; maxX: number; maxY: number }

/**
 * Group runs into blocks: two runs join when they sit on adjacent rows and
 * overlap in x. `runs` must be in row order, left to right within a row.
 */
export function runBlocks(runs: Run[]): Block[] {
  const parent = runs.map((_, i) => i);
  const find = (i: number): number => { while (parent[i] !== i) { parent[i] = parent[parent[i]]; i = parent[i]; } return i; };
  let prevStart = 0, prevEnd = 0; // [prevStart, prevEnd): runs on row y-1
  let i = 0;
  while (i < runs.length) {
    const y = runs[i].y;
    let end = i;
    while (end < runs.length && runs[end].y === y) end++;
    const prevY = prevEnd > prevStart ? runs[prevStart].y : -2;
    if (prevY === y - 1) {
      // Two pointers over two x-sorted rows.
      let a = prevStart, b = i;
      while (a < prevEnd && b < end) {
        if (runs[a].x0 < runs[b].x1 && runs[b].x0 < runs[a].x1) parent[find(b)] = find(a);
        if (runs[a].x1 <= runs[b].x1) a++; else b++;
      }
    }
    prevStart = i; prevEnd = end; i = end;
  }
  const byRoot = new Map<number, Block>();
  runs.forEach((r, k) => {
    const root = find(k);
    const blk = byRoot.get(root) ?? { count: 0, minX: r.x0, minY: r.y, maxX: r.x1 - 1, maxY: r.y };
    blk.count += r.x1 - r.x0;
    if (r.x0 < blk.minX) blk.minX = r.x0; if (r.x1 - 1 > blk.maxX) blk.maxX = r.x1 - 1;
    if (r.y < blk.minY) blk.minY = r.y; if (r.y > blk.maxY) blk.maxY = r.y;
    byRoot.set(root, blk);
  });
  return [...byRoot.values()];
}

/**
 * Blown-out pixels of EITHER polarity, with the block they form.
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
  const minRows = Math.round(MIN_BLOCK_ROWS * scale);
  const scan = (isHit: (at: number) => boolean): Run[] => {
    const runs: Run[] = [];
    for (let y = 0; y < img.height; y++) {
      let runStart = -1;
      for (let x = 0; x <= img.width; x++) {
        const hit = x < img.width && isHit(y * img.stride + x * img.channels);
        if (hit && runStart < 0) runStart = x;
        if (!hit && runStart >= 0) {
          if (x - runStart >= MIN_RUN) runs.push({ y, x0: runStart, x1: x });
          runStart = -1;
        }
      }
    }
    return runs;
  };
  const px = img.pixels;
  const polarity = (runs: Run[]): Polarity => {
    const blocks = runBlocks(runs).map((b) => {
      const fill = b.count / ((b.maxX - b.minX + 1) * (b.maxY - b.minY + 1));
      return { ...b, fill, solid: fill >= SOLID_FILL && b.maxY - b.minY + 1 >= minRows };
    });
    const count = blocks.reduce((n, b) => n + b.count, 0);
    // Judge by the biggest solid block, else the biggest cluster, so a real
    // rectangle is reported with its own box however much texture surrounds it.
    const pick = blocks.reduce<(typeof blocks)[number] | null>(
      (best, b) => (!best || (b.solid && !best.solid) || (b.solid === best.solid && b.count > best.count) ? b : best), null);
    const extent: Box = blocks.length
      ? [Math.min(...blocks.map((b) => b.minX)), Math.min(...blocks.map((b) => b.minY)), Math.max(...blocks.map((b) => b.maxX)), Math.max(...blocks.map((b) => b.maxY))]
      : null;
    return {
      count,
      box: pick ? [pick.minX, pick.minY, pick.maxX, pick.maxY] : null,
      fill: pick ? Math.round(pick.fill * 1000) / 1000 : 0,
      solid: !!pick?.solid,
      blocks: blocks.length,
      extent,
    };
  };
  const b = polarity(scan((at: number) => px[at] === 0 && px[at + 1] === 0 && px[at + 2] === 0));
  const w = polarity(scan((at: number) => px[at] === 255 && px[at + 1] === 255 && px[at + 2] === 255));
  return {
    count: b.count + w.count,
    black: b,
    white: w,
    box: b.count >= w.count ? b.box : w.box,
    solid: b.solid || w.solid,
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

/**
 * `ps -o etime` ("[[dd-]hh:]mm:ss") in seconds, or NaN if it is not that shape.
 *
 * Why: `--shipped` proved the fix is in the binary ON DISK, but a running
 * Permagent.app keeps serving the assets embedded in the binary it was
 * launched from. Replacing the bundle under a running app changes nothing on
 * screen until the next launch, so "the fix shipped" is only true for the
 * user if the running process started after the binary was written. macOS
 * `ps` has no `etimes`, and `lstart` is locale-formatted, so elapsed time is
 * the portable way to get the start.
 */
export function parseElapsed(etime: string): number {
  const m = /^\s*(?:(\d+)-)?(?:(\d+):)?(\d+):(\d+)\s*$/.exec(etime);
  if (!m) return NaN;
  const [, d, h, mm, ss] = m;
  return ((Number(d || 0) * 24 + Number(h || 0)) * 60 + Number(mm)) * 60 + Number(ss);
}

export interface RunningApp { pid: number; startedAt: string; stale: boolean }

/**
 * The processes in `ps -axo pid=,etime=,comm=` output that are this app
 * binary, and whether each one was started BEFORE the binary was last
 * written (so it is still showing the previous build's UI). One second of
 * slack covers `etime`'s whole-second resolution.
 */
export function runningAppProcesses(psOutput: string, binaryPath: string, binaryMtimeMs: number, nowMs: number): RunningApp[] {
  const out: RunningApp[] = [];
  for (const line of psOutput.split('\n')) {
    const m = /^\s*(\d+)\s+(\S+)\s+(.+?)\s*$/.exec(line);
    if (!m || m[3] !== binaryPath) continue;
    const elapsed = parseElapsed(m[2]);
    if (!Number.isFinite(elapsed)) continue;
    const started = nowMs - elapsed * 1000;
    out.push({ pid: Number(m[1]), startedAt: new Date(started).toISOString(), stale: started + 1000 < binaryMtimeMs });
  }
  return out;
}

/**
 * The World's own localStorage keys out of the app's WebKit store, as
 * `sqlite3 -readonly <db> "select key, hex(value) from ItemTable"` prints
 * them ("key|HEX" per line). WebKit keeps each value as UTF-16LE bytes.
 *
 * Why: every pixel probe rendered the forum with EMPTY storage, which is the
 * Agent Desk open and the place rail open. The user's app, read on
 * 2026-09-19, has `permagent.forum.desk = closed` and the rail closed, so the
 * canvas he looks at is the full window width: a different camera aspect,
 * a different place for the gas giant in the frame, and different render
 * target sizes in the composer. A probe that renders a layout the user never
 * sees cannot vouch for the one he does. Only the World's keys are kept (no
 * theme, no chat drafts): the probe needs the layout, not the user's data.
 */
export function forumStorageFromRows(rows: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of rows.split('\n')) {
    const bar = line.indexOf('|');
    if (bar < 0) continue;
    const key = line.slice(0, bar), hex = line.slice(bar + 1).trim();
    if (!/^permagent\.(forum|world)\./.test(key) || !/^(?:[0-9A-Fa-f]{4})*$/.test(hex)) continue;
    let value = '';
    for (let i = 0; i < hex.length; i += 4) value += String.fromCharCode(parseInt(hex.slice(i + 2, i + 4) + hex.slice(i, i + 2), 16));
    out[key] = value;
  }
  return out;
}

/**
 * Whether macOS reports the login session's screen as locked, from
 * `ioreg -n Root -d1 -a` (a plist). On 2026-09-19 every attempt at the in-app
 * night look failed on this: a capture of a locked session is the lock screen,
 * and judging THAT as "no white block" would be a clean verdict about the
 * wrong picture. `--app-window` refuses instead.
 */
export function screenIsLocked(ioregPlist: string): boolean {
  return /<key>CGSSessionScreenIsLocked<\/key>\s*<true\s*\/>/.test(ioregPlist);
}

export interface WindowRow { id: number; pid: number; layer: number; x: number; y: number; w: number; h: number }

/**
 * The app's main window among on-screen windows: layer 0 (a normal window,
 * not a menu-bar item or tooltip), owned by one of the app's processes, the
 * largest by area. Owner NAMES are not used: a window's owner is matched by
 * the pid of the binary `--shipped` checked, so a dev build or another app
 * called Permagent cannot stand in for it.
 */
export function pickAppWindow(rows: WindowRow[], pids: number[]): WindowRow | null {
  let best: WindowRow | null = null;
  for (const r of rows) {
    if (r.layer !== 0 || !pids.includes(r.pid) || r.w < 200 || r.h < 200) continue;
    if (!best || r.w * r.h > best.w * best.h) best = r;
  }
  return best;
}

/**
 * The World's day/night, as the running app will see it, read from
 * `defaults read -g AppleInterfaceStyle` without touching the GUI. The World
 * takes night from `(prefers-color-scheme: dark)` (useSystemAppearance.ts),
 * which WebKit answers from this system setting; the desktop shell sets no
 * window theme of its own. The key holds "Dark" in dark mode (and in Auto
 * mode while it is dark) and does not exist in light mode, where `defaults`
 * exits non-zero. The user's white block was over the NIGHT sky
 * (2026-09-18): a clean `--app-window` capture taken in light mode would be a
 * clean verdict about the day scene, so the probe refuses unless this says
 * 'night'. `ok` is whether the `defaults` command succeeded.
 */
export function worldAppearanceFromDefaults(stdout: string, ok: boolean): 'night' | 'day' {
  return ok && stdout.trim().toLowerCase() === 'dark' ? 'night' : 'day';
}

export interface WorldInCapture {
  /** At least two of the World's own HUD labels were read off the capture. */
  world: boolean;
  /** What the World's title card says it resolved, or null if that line was not read. */
  appearance: 'night' | 'day' | null;
  /** The labels that matched, so a verdict can be checked against the capture. */
  anchors: string[];
}

/**
 * Whether a capture of the app window shows the World, from the text macOS
 * Vision reads off it (`VNRecognizeTextRequest`, on-device, one line per
 * observation).
 *
 * Why: node r2 of the white-block work could not be confirmed in the real app
 * on 2026-09-19. `--shipped` had proved the fix is in the binary the running
 * app was launched from, and `--app-window` could capture that window, but it
 * printed "clean" for ANY view: the Dashboard or Chat has no gas giant to clip,
 * so a clean capture of it would have closed the node on the wrong picture.
 * That was left as an operator step ("open the capture and check"), which is
 * exactly the step nobody can take while the screen is locked. Reading the
 * World's own labels makes the probe refuse on a capture that is not the World.
 *
 * Two labels, not one: the title card can be dismissed and the play buttons
 * change while walking or flying, and a single word ("Forum") could appear in
 * a chat. The labels are the ones visible in the user's layout (desk and rail
 * closed): Vision reads them even at the HUD's resting opacity (0.32 on the
 * title card), measured on the --app-state night frames. Matching folds case,
 * accents ("FÖRUM" is a real Vision read of that title) and spacing.
 *
 * `appearance` comes from the title card's "Night / follows system appearance"
 * line, which is what the World itself resolved from prefers-color-scheme: a
 * stronger witness than the system setting, which only says what it SHOULD
 * resolve.
 */
export function worldFromOcr(lines: string[]): WorldInCapture {
  const fold = (s: string) => s.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/\s+/g, ' ').trim();
  const text = lines.map(fold);
  const labels: Array<[string, RegExp]> = [
    ['The Solar Forum', /\bthe solar forum\b/],
    ['follows system appearance', /\bfollows system appearance\b/],
    ['Walk the world', /\b(?:walk the world|return to overview)\b/],
    ['Fly the skiff', /\b(?:fly|leave) the skiff\b/],
    ['Show the desk', /\b(?:show|hide) the desk\b/],
  ];
  const anchors = labels.filter(([, re]) => text.some((t) => re.test(t))).map(([name]) => name);
  let appearance: WorldInCapture['appearance'] = null;
  for (const t of text) {
    const m = /\b(night|day)\s*\/?\s*follows system appearance\b/.exec(t);
    if (m) { appearance = m[1] as 'night' | 'day'; break; }
  }
  return { world: anchors.length >= 2, appearance, anchors };
}
