// @vitest-environment jsdom
import { describe, it, expect, vi, beforeAll } from 'vitest';
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
  // jsdom lacks matchMedia; xterm's CoreBrowserService needs a minimal stub
  // (DPR change tracking only — irrelevant to buffer semantics under test).
  beforeAll(() => {
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
  });

  async function renderStream(chunks: string[]): Promise<string[]> {
    const { Terminal } = await import('@xterm/xterm');
    const term = new Terminal({ cols: 80, rows: 24, allowProposedApi: true });
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
