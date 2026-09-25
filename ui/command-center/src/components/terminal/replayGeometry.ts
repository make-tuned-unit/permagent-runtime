/**
 * Replaying a PTY tail at the geometry it was painted for.
 *
 * WHY (reported 2026-09-24, Build terminal popped out into its own window):
 * the pop-out showed several stale copies of the harness's pinned prompt —
 * purple borders, "gpt-5.4-mini ~/Documents/d", "• orchestrator__decompose_…",
 * "❯ Ask Permagent to do anything" — some cut at ~46 columns, some at the wide
 * width, above the one live frame at the bottom.
 *
 * The harness (crates/goose-cli/src/session/composer.rs) pins its prompt box
 * to the bottom rows with DECSTBM and paints every row with ABSOLUTE cursor
 * addressing (`CSI row;1H`). Those bytes only mean what they meant for the
 * grid the PTY had when they were written. A reattaching xterm (a pop-out, a
 * redock, a Build pane remount) replays up to 2 MB of that stream — frames
 * painted at every size the docked pane has had — and it used to replay all of
 * it into a terminal already fitted to the NEW container. `CSI 26;1H` then
 * lands in the middle of a 45-row screen, the scroll region no longer sits
 * above the frame, and each later streamed line scrolls another stale frame
 * into the scrollback. The harness's own resize cleanup (`stale_strip_start`,
 * `reflowed_rows`) assumes the terminal REFLOWED the old frame the way xterm
 * does on a resize; a replay at the wrong size never reflowed anything, so
 * that cleanup erased the wrong rows.
 *
 * THE FIX: the backend records the grid at every `resize_pty` as a mark at a
 * stream position (`GridMark`, terminal.rs), and returns the marks covering
 * the replay window with it. The reattaching xterm is resized to each mark's
 * grid before the bytes painted for it are parsed — so the replay goes through
 * exactly the geometry changes (and xterm reflows) the original terminal went
 * through — and only then fitted to its own container. That last resize is an
 * ordinary resize: xterm reflows, SIGWINCH reaches the harness, and the
 * harness clears its reflowed strip and repaints one clean frame, which is the
 * path its cleanup was built for.
 *
 * Nothing here touches the bytes (#573: PTY output reaches xterm verbatim);
 * only the grid they are parsed at changes.
 */

/** A `resize_pty` (or the spawn size) at a stream position — terminal.rs. */
export interface GridMark {
  /** Stream position (UTF-8 bytes produced so far) at which the grid applied. */
  seq: number;
  cols: number;
  rows: number;
}

export interface Grid {
  cols: number;
  rows: number;
}

/** One run of replay bytes and the grid they were painted for. */
export interface ReplaySegment {
  data: string;
  /** Null when the backend reported no geometry: parse at whatever size the
   *  terminal already has (the pre-2026-09-24 behaviour). */
  grid: Grid | null;
}

function validGrid(m: { cols: number; rows: number }): boolean {
  return (
    Number.isInteger(m.cols) && Number.isInteger(m.rows) && m.cols >= 2 && m.rows >= 2
  );
}

/** UTF-8 length of one UTF-16 code unit sequence position, matching Rust's
 *  `String::len()` (which is what `seq` counts). */
function utf8Width(code: number): number {
  if (code < 0x80) return 1;
  if (code < 0x800) return 2;
  return 3; // BMP (a surrogate PAIR is handled by the caller as 4)
}

/**
 * String index for each byte offset in `offsets` (ascending), measured in
 * UTF-8 bytes from the start of `data`. An offset inside a multibyte char —
 * which a real mark never is, marks sit on chunk boundaries — rounds up to
 * the next char boundary so a segment never splits a character.
 */
function utf8OffsetsToIndices(data: string, offsets: number[]): number[] {
  const out: number[] = [];
  let bytes = 0;
  let i = 0;
  for (const target of offsets) {
    while (i < data.length && bytes < target) {
      const code = data.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < data.length) {
        const next = data.charCodeAt(i + 1);
        if (next >= 0xdc00 && next <= 0xdfff) {
          bytes += 4;
          i += 2;
          continue;
        }
      }
      bytes += utf8Width(code);
      i += 1;
    }
    out.push(i);
  }
  return out;
}

/** UTF-8 byte length of a string, as Rust's `String::len()` reports it. */
export function utf8Length(data: string): number {
  let bytes = 0;
  for (let i = 0; i < data.length; i++) {
    const code = data.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff && i + 1 < data.length) {
      const next = data.charCodeAt(i + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        i += 1;
        continue;
      }
    }
    bytes += utf8Width(code);
  }
  return bytes;
}

/**
 * Split a replay into runs, each tagged with the grid its bytes were painted
 * for.
 *
 * `endSeq` is the stream position at the END of `data` (the backend reads
 * both under one lock, so it is exact); the replay therefore starts at
 * `endSeq - utf8Length(data)`. The grid in force at that start is the last
 * mark at or before it. A mark after `endSeq` belongs to bytes not in this
 * replay and is ignored. Without any usable mark the whole replay is one
 * run with `grid: null`.
 */
export function planReplay(
  data: string,
  endSeq: number,
  marks: readonly GridMark[] | null | undefined,
): ReplaySegment[] {
  if (!data) return [];
  const usable = (marks ?? [])
    .filter((m) => m && Number.isFinite(m.seq) && validGrid(m))
    .slice()
    .sort((a, b) => a.seq - b.seq);
  if (usable.length === 0) return [{ data, grid: null }];

  const start = endSeq - utf8Length(data);
  // Grid in force at the replay's first byte.
  let initial: GridMark | null = null;
  const inside: GridMark[] = [];
  for (const m of usable) {
    if (m.seq <= start) initial = m;
    else if (m.seq < endSeq) inside.push(m);
  }
  // A window that starts before the first recorded mark (should not happen:
  // the spawn size is mark zero) parses its head at the first mark's grid —
  // the closest thing to the truth that is known.
  const headGrid: Grid = initial
    ? { cols: initial.cols, rows: initial.rows }
    : { cols: usable[0].cols, rows: usable[0].rows };

  const cuts = utf8OffsetsToIndices(
    data,
    inside.map((m) => m.seq - start),
  );
  const segments: ReplaySegment[] = [];
  let from = 0;
  let grid: Grid = headGrid;
  inside.forEach((m, k) => {
    const to = cuts[k];
    if (to > from) segments.push({ data: data.slice(from, to), grid });
    from = to;
    grid = { cols: m.cols, rows: m.rows };
  });
  if (from < data.length) segments.push({ data: data.slice(from), grid });

  // Merge neighbours that share a grid (a resize to the same size is not a
  // geometry change) so the writer does not await for nothing.
  const merged: ReplaySegment[] = [];
  for (const seg of segments) {
    const last = merged[merged.length - 1];
    if (last && last.grid && seg.grid && sameGrid(last.grid, seg.grid)) {
      last.data += seg.data;
    } else {
      merged.push({ ...seg });
    }
  }
  return merged;
}

export function sameGrid(a: Grid, b: Grid): boolean {
  return a.cols === b.cols && a.rows === b.rows;
}

/** The subset of xterm's API the replay writer uses. */
export interface ReplayTerminal {
  cols: number;
  rows: number;
  resize(cols: number, rows: number): void;
  write(data: string, callback?: () => void): void;
}

/**
 * Write a planned replay, resizing the terminal to each run's grid before its
 * bytes are parsed.
 *
 * xterm parses `write()` asynchronously but applies `resize()` at once, so a
 * resize issued while earlier bytes are still queued would re-grid those
 * bytes too. Each run is therefore drained (write callback) before the next
 * resize — and the promise resolves only once the LAST run is parsed, so the
 * caller's fit-to-container resize cannot overtake the replay either.
 *
 * Resolves true when the terminal was re-gridded (the caller must fit it back
 * to its container), false for an unmarked replay written as-is.
 */
export async function writeReplay(
  term: ReplayTerminal,
  segments: readonly ReplaySegment[],
): Promise<boolean> {
  // No geometry from the backend: write as the terminal stands, exactly as
  // before marks existed, and tell the caller nothing was re-gridded.
  if (segments.every((seg) => seg.grid === null)) {
    for (const seg of segments) term.write(seg.data);
    return false;
  }
  for (const seg of segments) {
    if (seg.grid && !sameGrid(seg.grid, term)) {
      term.resize(seg.grid.cols, seg.grid.rows);
    }
    await new Promise<void>((resolve) => term.write(seg.data, resolve));
  }
  return true;
}
