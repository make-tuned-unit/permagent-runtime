/** Port of the CLI infinity banner (`crates/goose-cli/src/session/output.rs`).
 *  Lemniscate of Bernoulli at braille resolution, cyan→violet ribbon, comet sweep. */

export const MOBIUS_W = 26;
export const MOBIUS_H = 5;
export const MOBIUS_SAMPLES = 360;
export const MOBIUS_INTRO_FRAMES = 45; // 0..=44 in the CLI
export const MOBIUS_INTERVAL_MS = 26;

/** Brand ribbon: cyan #00D5FF → purple #A855CC (tokens.ts), not the 256-color
 *  approximation. Truecolor hex so Ink/xterm paint the same colors as the GUI. */
const RIBBON = [
  "#00D5FF",
  "#00B8F5",
  "#3D8AE8",
  "#5B6ED4",
  "#7A55C4",
  "#8D44AE",
  "#A855CC",
  "#C893E0",
];
const HOT = "#FFFFFF";
const WAIT = "#444444";

const BRAILLE_BITS: number[][] = [
  [0x01, 0x08],
  [0x02, 0x10],
  [0x04, 0x20],
  [0x40, 0x80],
];

type Cell = { mask: number; tsum: number; n: number };
type Sample = { row: number; col: number; t: number };

export type ColorRun = { text: string; color: string };

interface Band {
  cells: Map<string, Cell>;
  samples: Sample[];
}

function key(row: number, col: number): string {
  return `${row},${col}`;
}

function infinityBand(): Band {
  const dw = MOBIUS_W * 2;
  const dh = MOBIUS_H * 4;
  const cells = new Map<string, Cell>();
  const samples: Sample[] = [];
  for (let i = 0; i < MOBIUS_SAMPLES; i++) {
    const t = i / MOBIUS_SAMPLES;
    const th = t * Math.PI * 2;
    const d = 1.0 + Math.sin(th) * Math.sin(th);
    const x = Math.cos(th) / d;
    const y = (Math.sin(th) * Math.cos(th)) / d;
    const px = Math.round((x * 0.48 + 0.5) * (dw - 1));
    const py = Math.round((y * 1.3 + 0.5) * (dh - 1));
    const col = Math.floor(px / 2);
    const row = Math.floor(py / 4);
    const bit = BRAILLE_BITS[py % 4]![px % 2]!;
    const k = key(row, col);
    const e = cells.get(k) ?? { mask: 0, tsum: 0, n: 0 };
    e.mask |= bit;
    e.tsum += t;
    e.n += 1;
    cells.set(k, e);
    samples.push({ row, col, t });
  }
  return { cells, samples };
}

function ribbonColor(t: number): string {
  const m = 1.0 - Math.abs(2.0 * t - 1.0);
  const idx = Math.min(
    RIBBON.length - 1,
    Math.round(m * (RIBBON.length - 1)),
  );
  return RIBBON[idx]!;
}

function infinityFrame(band: Band, comet: number | null): ColorRun[][] {
  const TAIL = 34;
  const hot = new Map<string, number>();
  if (comet !== null) {
    for (let k = 0; k < TAIL; k++) {
      const idx = (comet + MOBIUS_SAMPLES - k) % MOBIUS_SAMPLES;
      const s = band.samples[idx]!;
      const heat = 1.0 - k / TAIL;
      const ck = key(s.row, s.col);
      const prev = hot.get(ck) ?? 0;
      if (heat > prev) hot.set(ck, heat);
    }
  }

  const lines: ColorRun[][] = [];
  for (let r = 0; r < MOBIUS_H; r++) {
    const runs: ColorRun[] = [{ text: "  ", color: WAIT }];
    const push = (ch: string, color: string) => {
      const last = runs[runs.length - 1]!;
      if (last.color === color) last.text += ch;
      else runs.push({ text: ch, color });
    };
    for (let c = 0; c < MOBIUS_W; c++) {
      const cell = band.cells.get(key(r, c));
      if (!cell) {
        push(" ", WAIT);
        continue;
      }
      const ch = String.fromCodePoint(0x2800 + cell.mask);
      const t = cell.tsum / cell.n;
      const heat = hot.get(key(r, c)) ?? 0;
      if (heat > 0.85) push(ch, HOT);
      else if (heat > 0) push(ch, ribbonColor(t));
      else if (comet !== null) push(ch, WAIT);
      else push(ch, ribbonColor(t));
    }
    lines.push(runs);
  }
  return lines;
}

function buildIntro(): ColorRun[][][] {
  const band = infinityBand();
  const last = MOBIUS_INTRO_FRAMES - 1;
  const frames: ColorRun[][][] = [];
  for (let f = 0; f <= last; f++) {
    if (f === last) {
      frames.push(infinityFrame(band, null));
      continue;
    }
    const u = f / last;
    const eased = u * u * (3.0 - 2.0 * u);
    const head = Math.floor(eased * 2.0 * MOBIUS_SAMPLES) % MOBIUS_SAMPLES;
    frames.push(infinityFrame(band, head));
  }
  return frames;
}

const INTRO = buildIntro();

export function getMobiusIntroFrame(i: number): ColorRun[][] {
  return INTRO[Math.min(Math.max(i, 0), INTRO.length - 1)]!;
}
