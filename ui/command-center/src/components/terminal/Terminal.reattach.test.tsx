/**
 * @vitest-environment jsdom
 *
 * Terminal reattach — the PTY grid sync.
 *
 * The regression this pins (2026-08-06): a Build pane toggle remounts the
 * Terminal, whose new xterm fits to the NEW pane width — but the initial
 * fit() runs before the onResize listener is registered, so the reattached
 * PTY was never told the new dimensions. Claude Code (any TUI) kept painting
 * its approval gates for the OLD width: overlapped, garbled status rows.
 * After (re)attach, the component must explicitly resize_pty to this xterm's
 * grid, and must do it AFTER the scrollback replay is written.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

// vi.mock factories are hoisted above module code, so the grid constants ride
// vi.hoisted and the stand-in class is defined inside the factory itself.
const { COLS, ROWS, termLog } = vi.hoisted(() => ({
  COLS: 217, // a full-width Build pane — deliberately not a default 80
  ROWS: 48,
  // What the fake xterm was asked to do, in order, with the grid in force.
  termLog: [] as string[],
}));

vi.mock('@xterm/xterm', () => {
  const disposable = () => ({ dispose: () => {} });
  class FakeXTerm {
    cols = COLS;
    rows = ROWS;
    options: Record<string, unknown> = {};
    parser = { registerOscHandler: () => disposable() };
    open() {}
    loadAddon(addon: { activate?: (t: unknown) => void }) { addon.activate?.(this); }
    resize(cols: number, rows: number) {
      this.cols = cols;
      this.rows = rows;
      termLog.push(`resize ${cols}x${rows}`);
    }
    write(d: string, cb?: () => void) {
      termLog.push(`write ${JSON.stringify(d)} @${this.cols}x${this.rows}`);
      cb?.();
    }
    writeln(_d: string) {}
    refresh() {}
    clear() {}
    dispose() {}
    onData() { return disposable(); }
    onTitleChange() { return disposable(); }
    onResize() { return disposable(); }
    // Real xterm exposes this; the component activates Unicode 11 width
    // tables through it. Without it the fake throws on assignment and every
    // test in this file fails for a reason unrelated to what it asserts.
    unicode = { activeVersion: '6' };
  }
  return { Terminal: FakeXTerm };
});
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    private term: { resize: (c: number, r: number) => void; cols: number; rows: number } | null = null;
    activate(t: { resize: (c: number, r: number) => void; cols: number; rows: number }) { this.term = t; }
    // A real fit sizes the grid to the container; this pane is COLS x ROWS.
    fit() {
      if (this.term && (this.term.cols !== COLS || this.term.rows !== ROWS)) this.term.resize(COLS, ROWS);
    }
  },
}));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));

// ── Tauri API stand-in ───────────────────────────────────────────────────────
type Invocation = { cmd: string; args?: Record<string, unknown> };
let replayResponse: unknown = { data: 'replayed scrollback', seq: 7 };
const invocations: Invocation[] = [];
const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  invocations.push({ cmd, args });
  if (cmd === 'get_pty_output') return replayResponse;
  if (cmd === 'spawn_pty_session') return { session_id: 'pty-fresh', cwd: '/tmp' };
  return undefined;
});
vi.mock('@tauri-apps/api/core', () => ({ invoke: (c: string, a?: Record<string, unknown>) => invoke(c, a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

import { Terminal } from './Terminal';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom has no ResizeObserver; the component observes its container for
// pane-resize refits, which is out of scope here.
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  invocations.length = 0;
  termLog.length = 0;
  replayResponse = { data: 'replayed scrollback', seq: 7 };
  invoke.mockClear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  // jsdom reports 0×0; a 0-box must NOT reach resize_pty (see the test
  // below). The happy-path tests stub a laid-out pane so fit/advertise run.
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
    configurable: true,
    get() { return 800; },
  });
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
    configurable: true,
    get() { return 600; },
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flushAsyncSetup() {
  // The setup effect awaits the Tauri api, the replay, and listener wiring —
  // drain the microtask queue until the invocation list stops growing.
  for (let i = 0; i < 12; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

describe('Terminal reattach grid sync', () => {
  it('resizes the reattached PTY to this xterm grid, after the replay', async () => {
    act(() => {
      root.render(
        <Terminal sessionId="pty-live" isVisible />,
      );
    });
    await flushAsyncSetup();

    const resize = invocations.find(i => i.cmd === 'resize_pty');
    expect(resize, 'reattach must resize_pty or TUIs paint for the old width').toBeTruthy();
    expect(resize!.args).toMatchObject({ sessionId: 'pty-live', cols: COLS, rows: ROWS });

    // Ordering: the replay is written for the old grid; the resize (and the
    // TUI repaint it provokes) must come after it, not race it.
    const replayIdx = invocations.findIndex(i => i.cmd === 'get_pty_output');
    const resizeIdx = invocations.findIndex(i => i.cmd === 'resize_pty');
    expect(replayIdx).toBeGreaterThanOrEqual(0);
    expect(resizeIdx).toBeGreaterThan(replayIdx);

    // A reattach must never spawn a second shell over the live one.
    expect(invocations.some(i => i.cmd === 'spawn_pty_session')).toBe(false);
  });

  // 2026-09-24 pop-out report: the replay was parsed at the NEW window's
  // size, scattering harness frames painted for older sizes. With the
  // backend's grid marks the replay is parsed at each painted grid, and only
  // then is the xterm fitted to its container and the PTY told.
  it('replays each run at the grid it was painted for, then fits and advertises', async () => {
    // "AAAA" painted at 46x30, "BB" after a resize to 90x30 (seq 1004).
    replayResponse = {
      data: 'AAAABB',
      seq: 1006,
      grid: [
        { seq: 0, cols: 46, rows: 30 },
        { seq: 1004, cols: 90, rows: 30 },
      ],
    };
    act(() => {
      root.render(<Terminal sessionId="pty-live" isVisible />);
    });
    await flushAsyncSetup();

    const fromReplay = termLog.slice(termLog.indexOf('resize 46x30'));
    expect(fromReplay).toEqual([
      'resize 46x30',
      'write "AAAA" @46x30',
      'resize 90x30',
      'write "BB" @90x30',
      `resize ${COLS}x${ROWS}`,
    ]);
    const resize = invocations.find(i => i.cmd === 'resize_pty');
    expect(resize!.args).toMatchObject({ sessionId: 'pty-live', cols: COLS, rows: ROWS });
  });

  it('without grid marks replays at the fitted size, as before', async () => {
    act(() => {
      root.render(<Terminal sessionId="pty-live" isVisible />);
    });
    await flushAsyncSetup();
    expect(termLog).toContain(`write "replayed scrollback" @${COLS}x${ROWS}`);
    expect(termLog.some(l => l.startsWith('resize') && l !== `resize ${COLS}x${ROWS}`)).toBe(false);
  });

  it('does not advertise a collapsed 0-box to a live PTY', async () => {
    Object.defineProperty(HTMLElement.prototype, 'offsetWidth', {
      configurable: true,
      get() { return 0; },
    });
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get() { return 0; },
    });
    act(() => {
      root.render(
        <Terminal sessionId="pty-live" isVisible />,
      );
    });
    await flushAsyncSetup();

    // The old unguarded fit() sent 2×1 (or this fake's stale 217×48) to a
    // live Claude Code session. A TUI then painted its status line on the
    // prompt. Hold the last good PTY size until the pane has a real box.
    expect(invocations.some(i => i.cmd === 'resize_pty')).toBe(false);
  });

  it('a fresh terminal spawns with this xterm grid (no stale defaults)', async () => {
    act(() => {
      root.render(
        <Terminal sessionId={null} isVisible />,
      );
    });
    await flushAsyncSetup();

    const spawn = invocations.find(i => i.cmd === 'spawn_pty_session');
    expect(spawn).toBeTruthy();
    expect(spawn!.args).toMatchObject({ cols: COLS, rows: ROWS });
  });
});
