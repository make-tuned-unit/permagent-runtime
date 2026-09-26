// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { planReplay, utf8Length, writeReplay, type GridMark, type ReplayTerminal } from './replayGeometry';

const ESC = '\x1b';

// ── Pure seam: which bytes were painted for which grid ───────────────────────

describe('planReplay', () => {
  it('without marks replays everything at the current size (legacy daemon)', () => {
    expect(planReplay('abc', 3, undefined)).toEqual([{ data: 'abc', grid: null }]);
    expect(planReplay('abc', 3, [])).toEqual([{ data: 'abc', grid: null }]);
  });

  it('an empty replay plans nothing', () => {
    expect(planReplay('', 0, [{ seq: 0, cols: 80, rows: 24 }])).toEqual([]);
  });

  it('parses the head at the grid in force where the (truncated) window starts', () => {
    // Stream positions 0..100 were trimmed away; the window is 100..106.
    const marks: GridMark[] = [
      { seq: 0, cols: 46, rows: 30 },
      { seq: 40, cols: 90, rows: 30 },
      { seq: 150, cols: 160, rows: 45 }, // after the replay: not ours
    ];
    expect(planReplay('abcdef', 106, marks)).toEqual([
      { data: 'abcdef', grid: { cols: 90, rows: 30 } },
    ]);
  });

  it('cuts at every mark inside the window, in UTF-8 byte offsets', () => {
    // "é" is 2 bytes and "❯" is 3, as Rust's String::len() counts them.
    const data = 'aé' + '❯b' + 'c';
    expect(utf8Length(data)).toBe(1 + 2 + 3 + 1 + 1);
    const start = 1000;
    const end = start + utf8Length(data);
    const marks: GridMark[] = [
      { seq: 10, cols: 46, rows: 30 },
      { seq: start + 3, cols: 90, rows: 30 }, // after "aé"
      { seq: start + 7, cols: 90, rows: 24 }, // after "❯b"
    ];
    expect(planReplay(data, end, marks)).toEqual([
      { data: 'aé', grid: { cols: 46, rows: 30 } },
      { data: '❯b', grid: { cols: 90, rows: 30 } },
      { data: 'c', grid: { cols: 90, rows: 24 } },
    ]);
  });

  it('counts astral characters as four bytes and never splits a pair', () => {
    const data = 'x🙂y';
    expect(utf8Length(data)).toBe(6);
    const plan = planReplay(data, 6, [
      { seq: 0, cols: 80, rows: 24 },
      { seq: 5, cols: 100, rows: 24 },
    ]);
    expect(plan.map((s) => s.data)).toEqual(['x🙂', 'y']);
  });

  it('merges runs whose grid did not actually change and drops bogus marks', () => {
    const plan = planReplay('abcd', 4, [
      { seq: 0, cols: 80, rows: 24 },
      { seq: 2, cols: 80, rows: 24 },
      { seq: 3, cols: 1, rows: 0 }, // FitAddon's collapsed-box floor — never real
    ]);
    expect(plan).toEqual([{ data: 'abcd', grid: { cols: 80, rows: 24 } }]);
  });

  it('a mark exactly at the window end applies to nothing in it', () => {
    expect(
      planReplay('ab', 12, [
        { seq: 0, cols: 80, rows: 24 },
        { seq: 12, cols: 120, rows: 40 },
      ]),
    ).toEqual([{ data: 'ab', grid: { cols: 80, rows: 24 } }]);
  });
});

describe('writeReplay', () => {
  it('drains each run before resizing for the next, so no run is parsed at the wrong grid', async () => {
    const log: string[] = [];
    let pending: (() => void) | null = null;
    const term: ReplayTerminal = {
      cols: 160,
      rows: 45,
      resize(c, r) {
        // A resize while a write is still queued would re-grid those bytes.
        expect(pending).toBeNull();
        this.cols = c;
        this.rows = r;
        log.push(`resize ${c}x${r}`);
      },
      write(d, cb) {
        log.push(`write ${d} @${this.cols}x${this.rows}`);
        pending = () => { pending = null; cb?.(); };
        setTimeout(() => pending?.(), 0);
      },
    };
    await writeReplay(term, [
      { data: 'A', grid: { cols: 46, rows: 30 } },
      { data: 'B', grid: { cols: 90, rows: 30 } },
    ]);
    expect(pending).toBeNull();
    expect(log).toEqual(['resize 46x30', 'write A @46x30', 'resize 90x30', 'write B @90x30']);
  });

  it('leaves the size alone for an unmarked (legacy) replay', async () => {
    const calls: string[] = [];
    const term: ReplayTerminal = {
      cols: 160,
      rows: 45,
      resize: () => calls.push('resize'),
      write: (_d, cb) => { calls.push('write'); cb?.(); },
    };
    await writeReplay(term, [{ data: 'x', grid: null }]);
    expect(calls).toEqual(['write']);
  });
});

// ── End-to-end in the real xterm.js: the pop-out repro ───────────────────────
//
// A harness-like painter that emits the same byte shapes as composer.rs
// (`paint`, `prepare_output`, `resync_region`, `stale_strip_start`,
// `reflowed_rows`), driving an ORIGINAL docked xterm through 46x30 → 90x30 →
// 90x24 → 90x30 while recording `resize_pty` marks exactly as terminal.rs
// does. A fresh xterm then "pops out" at 160x45 and replays that stream.

function stubMatchMedia() {
  if (!window.matchMedia) {
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        onchange: null,
        dispatchEvent: () => false,
      }),
    });
  }
}

type XTerm = import('@xterm/xterm').Terminal;

async function makeTerm(cols: number, rows: number): Promise<XTerm> {
  stubMatchMedia();
  const { Terminal } = await import('@xterm/xterm');
  const term = new Terminal({ cols, rows, scrollback: 10000, allowProposedApi: true });
  term.open(document.createElement('div'));
  return term;
}

const drain = (term: XTerm, data = '') =>
  new Promise<void>((resolve) => term.write(data, () => resolve()));

function allLines(term: XTerm): string[] {
  const buf = term.buffer.active;
  const out: string[] = [];
  for (let i = 0; i < buf.length; i++) out.push(buf.getLine(i)?.translateToString(true) ?? '');
  return out;
}

function viewport(term: XTerm): string[] {
  const buf = term.buffer.active;
  const out: string[] = [];
  for (let i = 0; i < term.rows; i++) out.push(buf.getLine(buf.viewportY + i)?.translateToString(true) ?? '');
  return out;
}

const PROMPT = 'Ask Permagent to do anything';
const count = (lines: string[], needle: string) =>
  lines.reduce((n, l) => n + l.split(needle).length - 1, 0);

function wrap(s: string, c: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < s.length; i += c) out.push(s.slice(i, i + c));
  return out;
}

/** composer.rs, reduced to the byte shapes that matter here. */
class HarnessPainter {
  cols: number;
  rows: number;
  private lastRows = 0;
  private lastCols = 0;
  private lastHeight = 0;
  private lastWidths: number[] = [];
  private prepared = false;
  private hasOutputCursor = false;
  private secs = 0;
  /** composer_rows: the status line wraps (harness-side) on a narrow pane,
   *  so the strip is taller at 46 columns than at 90. */
  get height() { return this.frameLines().length; }

  constructor(cols: number, rows: number) {
    this.cols = cols;
    this.rows = rows;
  }

  private frameLines(): string[] {
    const c = Math.max(this.cols, 24);
    const fit = (s: string) => (s.length > c ? s.slice(0, c) : s);
    const input = `│ ❯ ${PROMPT}`;
    return [
      ...wrap(`• orchestrator__decompose_roadmap (${this.secs}s · turn ${this.secs + 11}s …) · esc to interrupt · enter queues a follow-up`, c),
      `╭${'─'.repeat(c - 2)}╮`,
      `${fit(input).padEnd(c - 1, ' ')}│`,
      `╰${'─'.repeat(c - 2)}╯`,
      fit('gpt-5.4-mini ~/Documents/dev/CityCircle'),
    ];
  }

  private bottom() { return Math.max(this.rows - this.lastHeight, 1); }
  private region() { return `${ESC}[1;${Math.max(this.rows - this.lastHeight, 1)}r`; }

  install(): string {
    this.lastHeight = this.height;
    this.lastRows = this.rows;
    this.lastCols = this.cols;
    return `${this.region()}${ESC}[${this.bottom()};1H${ESC}[?2004h`;
  }

  private resync(): string {
    const h = this.height;
    if (h === this.lastHeight && this.rows === this.lastRows && this.cols === this.lastCols) return '';
    const cols = Math.max(this.cols, 1);
    const reflowed = this.lastWidths.reduce((n, w) => n + Math.max(Math.ceil(w / cols), 1), 0);
    const newStart = this.rows - h + 1;
    const oldStart = this.lastRows - this.lastHeight + 1;
    const reflowedStart = this.rows - Math.max(reflowed, this.lastHeight) + 1;
    const clearFrom = Math.max(Math.min(newStart, oldStart, reflowedStart), 1);
    let ansi = `${ESC}[?25l`;
    for (let r = clearFrom; r <= this.rows; r++) ansi += `${ESC}[${r};1H${ESC}[K`;
    this.lastHeight = h;
    this.lastRows = this.rows;
    this.lastCols = this.cols;
    ansi += this.region();
    this.hasOutputCursor = false;
    return ansi;
  }

  paint(): string {
    let out = '';
    const prepared = this.prepared;
    this.prepared = false;
    if (prepared) out += `${ESC}[s`;
    const resync = this.resync();
    if (prepared && resync === '') this.hasOutputCursor = true;
    out += resync + `${ESC}[?2004h` + this.region();
    const lines = this.frameLines();
    const start = this.rows - this.height + 1;
    out += `${ESC}[?25l`;
    lines.forEach((l, i) => { out += `${ESC}[${start + i};1H${ESC}[K${l}`; });
    out += `${ESC}[${start + lines.length - 3};${5 + PROMPT.length}H${ESC}[?25h`;
    this.lastWidths = lines.map((l) => [...l].length);
    return out;
  }

  prepareOutput(): string {
    let out = this.resync();
    out += `${ESC}[?2004h` + this.region();
    out += this.hasOutputCursor ? `${ESC}[?25l${ESC}[u` : `${ESC}[?25l${ESC}[${this.bottom()};1H`;
    this.prepared = true;
    return out;
  }

  tick(): string { this.secs += 1; return this.paint(); }
}

/** The docked terminal plus the PTY's replay buffer and resize marks. */
class DockedSession {
  stream = '';
  marks: GridMark[] = [];
  constructor(public term: XTerm) {
    this.marks.push({ seq: 0, cols: term.cols, rows: term.rows }); // spawn size
  }
  async emit(data: string) {
    this.stream += data;
    await drain(this.term, data);
  }
  /** `resize_pty`: the UI resizes xterm, terminal.rs records the mark. */
  resize(cols: number, rows: number) {
    this.term.resize(cols, rows);
    this.marks.push({ seq: utf8Length(this.stream), cols, rows });
  }
}

async function runHarnessHistory(session: DockedSession, painter: HarnessPainter) {
  let n = 0;
  const stream = async (k: number) => {
    for (let i = 0; i < k; i++) {
      await session.emit(painter.prepareOutput());
      await session.emit(`real output line ${++n}: the agent wrote something worth keeping\r\n`);
      await session.emit(painter.paint());
      if (i % 2 === 0) await session.emit(painter.tick());
    }
  };
  await session.emit(painter.install());
  await session.emit(painter.paint());
  await stream(12);
  // The Build pane is widened (the 46-col frames in the report).
  session.resize(90, 30); painter.cols = 90; painter.rows = 30;
  await session.emit(painter.paint());
  await stream(12);
  // …and shortened, then restored (a split opened and closed).
  session.resize(90, 24); painter.rows = 24;
  await session.emit(painter.paint());
  await stream(6);
  session.resize(90, 30); painter.rows = 30;
  await session.emit(painter.paint());
  await stream(8);
  return n;
}

/** The live bytes after the pop-out sends resize_pty (SIGWINCH → repaint). */
async function liveAfterAttach(painter: HarnessPainter, write: (d: string) => Promise<void>, cols: number, rows: number) {
  painter.cols = cols;
  painter.rows = rows;
  await write(painter.paint());
  for (let i = 0; i < 6; i++) {
    await write(painter.prepareOutput());
    await write(`live line ${i}\r\n`);
    await write(painter.tick());
  }
}

async function dockedHistory() {
  const docked = await makeTerm(46, 30);
  const session = new DockedSession(docked);
  const painter = new HarnessPainter(46, 30);
  const lines = await runHarnessHistory(session, painter);
  return { docked, session, painter, lines };
}

type Size = { cols: number; rows: number };

/** The old attach: xterm fitted to the pop-out BEFORE the replay, replay
 *  parsed at that size, later fits (font swap, window settle) resize it. */
async function popOutLegacy(first: Size, final: Size) {
  const { docked, session, painter } = await dockedHistory();
  const popout = await makeTerm(first.cols, first.rows);
  await drain(popout, session.stream);
  if (first.cols !== final.cols || first.rows !== final.rows) popout.resize(final.cols, final.rows);
  await liveAfterAttach(painter, (d) => drain(popout, d), final.cols, final.rows);
  docked.dispose();
  return popout;
}

/** The new attach, as Terminal.tsx runs it: plan, replay at each painted
 *  grid, THEN fit to the container and advertise it. */
async function popOutNew(first: Size, final: Size) {
  const { docked, session, painter, lines } = await dockedHistory();
  const popout = await makeTerm(first.cols, first.rows);
  await writeReplay(popout, planReplay(session.stream, utf8Length(session.stream), session.marks));
  popout.resize(final.cols, final.rows); // fitVisibleTerminal
  await liveAfterAttach(painter, (d) => drain(popout, d), final.cols, final.rows);
  docked.dispose();
  return { popout, lines };
}

/** What the ORIGINAL docked terminal would show had it simply been resized
 *  to the pop-out's size and fed the same live bytes. */
async function reference(final: Size) {
  const { docked, painter } = await dockedHistory();
  docked.resize(final.cols, final.rows);
  await liveAfterAttach(painter, (d) => drain(docked, d), final.cols, final.rows);
  return docked;
}

const dump = (label: string, t: XTerm) => {
  if (process.env.DUMP) console.log(`${label}\n` + allLines(t).map((l, i) => `${i}|${l}`).filter((l) => !/^\d+\|$/.test(l)).join('\n'));
};

// The pop-out window opens at 860x560 (~100x33) and the first fit can run on
// fallback font metrics before JetBrains Mono swaps in; the user may then
// maximise it. Each pair is (grid at replay time, grid it settles at).
const POP_OUTS: Array<[Size, Size]> = [
  [{ cols: 160, rows: 45 }, { cols: 160, rows: 45 }],
  [{ cols: 100, rows: 33 }, { cols: 100, rows: 33 }],
  [{ cols: 100, rows: 33 }, { cols: 160, rows: 45 }],
  [{ cols: 66, rows: 20 }, { cols: 160, rows: 45 }],
  [{ cols: 40, rows: 12 }, { cols: 100, rows: 33 }],
];

// These parse megabytes of real PTY bytes through xterm.js; alone they take
// ~0.5–2 s each, but under a loaded machine (a parallel cargo build) they hit
// vitest's 5 s default. A generous per-test budget keeps the gate honest
// without hiding a real hang (each still fails if it never finishes).
describe('pop-out replay in real xterm.js', { timeout: 30_000 }, () => {
  it('reproduces the stale frames with the old attach', async () => {
    const stale: number[] = [];
    for (const [first, final] of POP_OUTS) {
      const popout = await popOutLegacy(first, final);
      dump(`LEGACY ${first.cols}x${first.rows}->${final.cols}x${final.rows}`, popout);
      stale.push(count(allLines(popout), PROMPT));
      popout.dispose();
    }
    if (process.env.DUMP) console.log('legacy prompt-frame counts', stale);
    // At least the smaller-than-painted pop-outs leave stale frames behind.
    expect(Math.max(...stale)).toBeGreaterThan(1);
  });

  it.each(POP_OUTS)('geometry-true replay %o -> %o shows exactly one prompt frame and loses no output', async (first, final) => {
    const { popout, lines } = await popOutNew(first, final);
    const ref = await reference(final);
    dump(`NEW ${first.cols}x${first.rows}->${final.cols}x${final.rows}`, popout);
    const all = allLines(popout);
    expect(count(all, PROMPT)).toBe(1);
    expect(count(viewport(popout), PROMPT)).toBe(1);
    // The one frame is the live one, pinned to the bottom rows.
    const view = viewport(popout);
    expect(view[final.rows - 3]).toContain(PROMPT);
    expect(view[final.rows - 1]).toBe('gpt-5.4-mini ~/Documents/dev/CityCircle');
    // Cell-for-cell what the original terminal would show at that size —
    // including whatever real output the harness's own resizes cost it.
    expect(all).toEqual(allLines(ref));
    // No real output line appears twice.
    for (let i = 1; i <= lines; i++) {
      expect(count(all, `real output line ${i}:`)).toBeLessThanOrEqual(1);
    }
    popout.dispose();
    ref.dispose();
  });

  it('a trimmed window (the 2 MB cap) still starts at the grid in force', async () => {
    const { docked, session, painter } = await dockedHistory();
    // The window now starts mid-way through the 90x30 era, as a capped
    // buffer would.
    const cut = session.stream.indexOf('real output line 20:');
    const tail = session.stream.slice(cut);
    const popout = await makeTerm(160, 45);
    const plan = planReplay(tail, utf8Length(session.stream), session.marks);
    expect(plan[0].grid).toEqual({ cols: 90, rows: 30 });
    await writeReplay(popout, plan);
    popout.resize(160, 45);
    await liveAfterAttach(painter, (d) => drain(popout, d), 160, 45);
    expect(count(allLines(popout), PROMPT)).toBe(1);
    docked.dispose();
    popout.dispose();
  });

  it('a plain shell keeps all of its output through a pop-out', async () => {
    const docked = await makeTerm(46, 30);
    const session = new DockedSession(docked);
    const long = (i: number) => `row ${i} ${'x'.repeat(70)} end-${i}`; // wraps at 46 and at 90
    for (let i = 0; i < 40; i++) await session.emit(`j@mac % echo ${i}\r\n${long(i)}\r\n`);
    session.resize(90, 30);
    for (let i = 40; i < 60; i++) await session.emit(`j@mac % echo ${i}\r\n${long(i)}\r\n`);
    await session.emit('j@mac % ');

    const reference = await makeTerm(46, 30);
    const refSession = new DockedSession(reference);
    for (let i = 0; i < 40; i++) await refSession.emit(`j@mac % echo ${i}\r\n${long(i)}\r\n`);
    refSession.resize(90, 30);
    for (let i = 40; i < 60; i++) await refSession.emit(`j@mac % echo ${i}\r\n${long(i)}\r\n`);
    await refSession.emit('j@mac % ');
    reference.resize(160, 45);

    const popout = await makeTerm(160, 45);
    await writeReplay(popout, planReplay(session.stream, utf8Length(session.stream), session.marks));
    popout.resize(160, 45);

    const all = allLines(popout);
    // Wrapped rows rejoin into one logical line per output row.
    const text = all.join('\n');
    for (let i = 0; i < 60; i++) {
      expect(all.filter((l) => l.endsWith(`end-${i}`)).length).toBe(1);
      expect(text).toContain(`echo ${i}`);
    }
    expect(all.filter((l) => l.trim()).pop()?.trimEnd()).toBe('j@mac %');
    expect(all).toEqual(allLines(reference));
    for (const t of [docked, reference, popout]) t.dispose();
  });
});
