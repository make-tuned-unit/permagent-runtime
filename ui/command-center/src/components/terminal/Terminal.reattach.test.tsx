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
const { COLS, ROWS } = vi.hoisted(() => ({
  COLS: 217, // a full-width Build pane — deliberately not a default 80
  ROWS: 48,
}));

vi.mock('@xterm/xterm', () => {
  const disposable = () => ({ dispose: () => {} });
  class FakeXTerm {
    cols = COLS;
    rows = ROWS;
    options: Record<string, unknown> = {};
    parser = { registerOscHandler: () => disposable() };
    open() {}
    loadAddon() {}
    write(_d: string) {}
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
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));

// ── Tauri API stand-in ───────────────────────────────────────────────────────
type Invocation = { cmd: string; args?: Record<string, unknown> };
const invocations: Invocation[] = [];
const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  invocations.push({ cmd, args });
  if (cmd === 'get_pty_output') return { data: 'replayed scrollback', seq: 7 };
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
  invoke.mockClear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
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
