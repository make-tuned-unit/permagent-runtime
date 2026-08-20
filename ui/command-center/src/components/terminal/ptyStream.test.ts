// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { handlePtyData, type PtyStreamSink } from './ptyStream';

// ── #573 regression pins ────────────────────────────────────────────────────
//
// The doubled-"Forging…" / status-line ghosting came from a second writer:
// the #239 local-echo path injected keystrokes into the xterm buffer and then
// stripped "matching" leading bytes from later PTY chunks, corrupting TUI
// repaint sequences. The fixed contract is: PTY bytes reach the terminal
// VERBATIM, always, and nothing else writes. These tests pin that contract
// at two levels: the pure handler (byte identity) and a real xterm buffer
// fed a synthetic spinner-overwrite stream (rendered-line list).

const SID = 'sess-1';
const ESC = '\x1b';

function collectingSink() {
  const writes: string[] = [];
  const cwds: string[] = [];
  const sink: PtyStreamSink = {
    write: (d) => writes.push(d),
    onCwd: (p) => cwds.push(p),
  };
  return { writes, cwds, sink };
}

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

async function renderStream(chunks: string[], size: { cols?: number; rows?: number } = {}): Promise<string[]> {
  stubMatchMedia();
  const { Terminal } = await import('@xterm/xterm');
  const term = new Terminal({
    cols: size.cols ?? 80,
    rows: size.rows ?? 24,
    allowProposedApi: true,
  });
  term.open(document.createElement('div'));
  const sink: PtyStreamSink = { write: (d) => term.write(d) };
  for (const data of chunks) {
    handlePtyData({ session_id: SID, data }, SID, sink);
  }
  // term.write is async; wait for the parser to drain.
  await new Promise<void>((resolve) => term.write('', () => resolve()));
  const lines: string[] = [];
  const buf = term.buffer.active;
  for (let i = 0; i < buf.length; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) ?? '');
  }
  term.dispose();
  return lines;
}

describe('handlePtyData — verbatim stream contract', () => {
  it('forwards every chunk byte-identically, including chunks that start with printable chars', () => {
    const { writes, sink } = collectingSink();
    // Chunk boundaries fall arbitrarily mid-frame. The second chunk starts
    // with a printable "F" — under the old pending-echo strip, a recently
    // typed "F" would have eaten this byte and shifted the whole repaint.
    const chunks = [
      `${ESC}[1A\r${ESC}[K⠙ `,
      'Forging… (3m 11s · ↓ 6.6k tokens)',
      `\r${ESC}[`,
      'K⠋ Forging… (3m 12s · ↓ 6.7k tokens)',
    ];
    for (const data of chunks) {
      handlePtyData({ session_id: SID, data }, SID, sink);
    }
    expect(writes).toEqual(chunks); // byte-identical, chunk-for-chunk
  });

  it('never writes for a different session', () => {
    const { writes, cwds, sink } = collectingSink();
    handlePtyData({ session_id: 'other', data: 'ghost bytes' }, SID, sink);
    handlePtyData({ session_id: SID, data: 'ghost bytes' }, null, sink);
    expect(writes).toEqual([]);
    expect(cwds).toEqual([]);
  });

  it('does not call write for an empty payload', () => {
    const { writes, sink } = collectingSink();
    handlePtyData({ session_id: SID, data: '' }, SID, sink);
    expect(writes).toEqual([]);
  });

  it('surfaces OSC 7 CWD reports while still writing the chunk verbatim', () => {
    const { writes, cwds, sink } = collectingSink();
    const data = `${ESC}]7;file://mac.local/Users/jesse/dev%20box\x07prompt$ `;
    handlePtyData({ session_id: SID, data }, SID, sink);
    expect(cwds).toEqual(['/Users/jesse/dev box']);
    expect(writes).toEqual([data]);
  });

  it('ignores malformed OSC 7 percent-encoding without dropping the write', () => {
    const { writes, cwds, sink } = collectingSink();
    const data = `${ESC}]7;file://host/bad%zz\x07`;
    handlePtyData({ session_id: SID, data }, SID, sink);
    expect(cwds).toEqual([]);
    expect(writes).toEqual([data]);
  });

  it('tolerates a missing onCwd sink', () => {
    const write = vi.fn();
    const data = `${ESC}]7;file://host/ok\x07`;
    expect(() => handlePtyData({ session_id: SID, data }, SID, { write })).not.toThrow();
    expect(write).toHaveBeenCalledWith(data);
  });
});

// ── Rendered-buffer pin: spinner overwrite leaves exactly one status line ───

describe('spinner-overwrite rendering through a real xterm buffer', () => {
  it('in-place \\r + EL updates yield exactly one Forging line with the final text', async () => {
    const lines = await renderStream([
      '$ claude\r\n',
      '⠙ Forging… (3m 10s · ↓ 6.5k tokens)',
      // Longer → shorter → longer rewrites; \x1b[K must fully clear each frame.
      `\r${ESC}[K⠋ Forging… (3s)`,
      `\r${ESC}[K⠹ Forging… (3m 11s · ↓ 6.6k tokens)`,
    ]);
    const forgingLines = lines.filter((l) => l.includes('Forging'));
    expect(forgingLines).toEqual(['⠹ Forging… (3m 11s · ↓ 6.6k tokens)']);
  });

  it('multi-line frame repaint (cursor-up + erase-down) leaves no ghost frame', async () => {
    const lines = await renderStream([
      'output line A\r\n',
      '⠙ Forging… (1s)',
      // Ink-style repaint: cursor to column 0, up one line, erase to end of
      // screen, then redraw the two-line frame with new content.
      `\r${ESC}[1A${ESC}[J`,
      'output line A\r\noutput line B\r\n⠋ Forging… (2s)',
    ]);
    const forgingLines = lines.filter((l) => l.includes('Forging'));
    expect(forgingLines).toEqual(['⠋ Forging… (2s)']);
    expect(lines.filter((l) => l === 'output line B')).toHaveLength(1);
  });

  it('erase sequences split across chunk boundaries still clear the old frame', async () => {
    const lines = await renderStream([
      '⠙ Forging… (3m 10s · ↓ 6.5k tokens)',
      `\r${ESC}[`, // split mid-CSI — the old strip loop ran per-chunk and could desync this
      'K⠋ Forging… (3m 11s)',
    ]);
    const forgingLines = lines.filter((l) => l.includes('Forging'));
    expect(forgingLines).toEqual(['⠋ Forging… (3m 11s)']);
  });
});

// ── Replay/live overlap ─────────────────────────────────────────────────────
//
// The PTY reader appends each chunk to the bounded replay buffer AND emits it
// as `pty_data`, so both carry the same bytes. A terminal that reattaches
// subscribes first (or it loses whatever arrives during the round trip), which
// means the live stream and the replay overlap. Writing both wrote that overlap
// twice — stale output landing on top of the line the TUI was drawing, which is
// how harness text ended up spliced into Claude Code's input box.
//
// `seq` is a stream POSITION, not a length, so it stays correct even after the
// replay buffer truncates at 2 MB.
describe('replay/live handoff', () => {
  /** The rule Terminal.tsx applies when releasing held chunks. */
  function release(held: Array<{ data: string; seq?: number }>, upTo: number | null): string[] {
    return held
      .filter(c => !(c.seq !== undefined && upTo !== null && c.seq <= upTo))
      .map(c => c.data);
  }

  it('drops chunks the replay already contains', () => {
    const held = [{ data: 'a', seq: 10 }, { data: 'b', seq: 20 }, { data: 'c', seq: 30 }];
    expect(release(held, 20)).toEqual(['c']);
  });

  it('keeps everything when the replay covered nothing', () => {
    const held = [{ data: 'a', seq: 10 }, { data: 'b', seq: 20 }];
    expect(release(held, 0)).toEqual(['a', 'b']);
  });

  it('keeps chunks with no seq — a duplicate is recoverable, a gap is not', () => {
    const held = [{ data: 'a' }, { data: 'b', seq: 5 }];
    expect(release(held, 10)).toEqual(['a']);
  });

  it('releases everything when the replay call failed', () => {
    const held = [{ data: 'a', seq: 1 }, { data: 'b', seq: 2 }];
    expect(release(held, null)).toEqual(['a', 'b']);
  });

  it('is exact at the boundary — seq equal to the replay position is covered', () => {
    const held = [{ data: 'boundary', seq: 100 }, { data: 'after', seq: 101 }];
    expect(release(held, 100)).toEqual(['after']);
  });
});

// ── TUI status-line vs prompt (2026-08-19) ─────────────────────────────────
//
// Claude Code paints "Press up to edit queued messages" with CUP onto the
// last row, and the input on the row above. When the PTY grid matches xterm,
// those stay distinct. When it does not (xterm wrapped a line the TUI thought
// was one row), the hint is written onto the wrapped remainder — the
// concatenated "messagesit. That seems like a pri" screenshot.
//
// These tests feed a real xterm buffer through handlePtyData (verbatim, #573).
// They do not inject spaces or strip ANSI. The grid-sync that prevents the
// mismatch lives in ptyGrid.ts.
describe('queued-message hint vs prompt row', () => {
  const HINT = 'Press up to edit queued messages';
  const BODY = 'it. That seems like a pri';

  it('CUP to the last row leaves the hint off the prompt when the grid matches', async () => {
    const lines = await renderStream(
      [
        `${ESC}[2J${ESC}[H`,
        `${ESC}[23;1H> ${BODY}`,
        `${ESC}[24;1H${HINT}`,
      ],
      { cols: 80, rows: 24 },
    );
    const joined = lines.join('\n');
    expect(joined).not.toContain('messagesit');
    const hintRow = lines.findIndex(l => l.includes(HINT));
    const bodyRow = lines.findIndex(l => l.includes(BODY));
    expect(hintRow).toBeGreaterThanOrEqual(0);
    expect(bodyRow).toBeGreaterThanOrEqual(0);
    expect(hintRow).not.toBe(bodyRow);
  });

  it('a cols mismatch splices the hint into the wrapped prompt — that is the bug class', async () => {
    // 40-col xterm. 40 A's fill row 1. Row 2 holds `HINT.length` placeholder
    // cells plus a leftover that starts with "it" — the same join as the
    // screenshot once a TUI that still thinks it has 80 cols CUP-paints the
    // hint onto row 2 without EL.
    const prefix = 'A'.repeat(40);
    const leftover = 'it. That'.slice(0, 40 - HINT.length);
    const row2 = 'X'.repeat(HINT.length) + leftover;
    expect(row2).toHaveLength(40);
    const lines = await renderStream(
      [
        prefix + row2,
        `${ESC}[2;1H${HINT}`,
      ],
      { cols: 40, rows: 24 },
    );
    const spliced = lines.find(l => l.includes('messagesit'));
    expect(spliced).toBeTruthy();
    expect(spliced).toContain(`${HINT}${leftover}`);
  });
});
