/**
 * @vitest-environment jsdom
 *
 * Terminal + followUpDelivery, wired together.
 *
 * Regression this pins: "Send to Claude" (Grow → Actions) opened a terminal,
 * started the harness, and never delivered the directive — the old
 * `scheduleFollowUpInput` pasted on a blind 2200ms timer that the harness
 * routinely missed (Keychain prompt, workspace-trust dialog, MCP startup).
 * The fix pastes only once the PTY stream shows the TUI took the tty
 * (bracketed paste plus a full-screen surface — see followUpDelivery.ts for
 * why 2004 alone is not enough), and surfaces a
 * "not delivered" chip if a ceiling elapses first.
 *
 * Mocks mirror Terminal.reattach.test.tsx, extended so the `pty_data` Tauri
 * listener handler is captured and callable — that's how this test injects
 * PTY chunks the readiness machine reads.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('@xterm/xterm', () => {
  const disposable = () => ({ dispose: () => {} });
  class FakeXTerm {
    cols = 80;
    rows = 24;
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
    unicode = { activeVersion: '6' };
  }
  return { Terminal: FakeXTerm };
});
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));
vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));

// ── Tauri API stand-in, with a capturable pty_data handler ─────────────────
type Invocation = { cmd: string; args?: Record<string, unknown> };
const invocations: Invocation[] = [];
const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
  invocations.push({ cmd, args });
  if (cmd === 'get_pty_output') return { data: '', seq: 0 };
  if (cmd === 'spawn_pty_session') return { session_id: 'pty-fresh', cwd: '/tmp' };
  return undefined;
});

let ptyDataHandler: ((e: { payload: unknown }) => void) | null = null;
const listen = vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
  if (event === 'pty_data') ptyDataHandler = handler;
  return () => {};
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (c: string, a?: Record<string, unknown>) => invoke(c, a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: (e: string, h: (e: { payload: unknown }) => void) => listen(e, h) }));

import { Terminal } from './Terminal';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  invocations.length = 0;
  invoke.mockClear();
  listen.mockClear();
  ptyDataHandler = null;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
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
  vi.useRealTimers();
});

async function flushAsyncSetup() {
  for (let i = 0; i < 12; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

function emitPtyData(data: string) {
  act(() => {
    ptyDataHandler?.({ payload: { session_id: 'pty-fresh', data } });
  });
}

function writeToPtyCalls() {
  return invocations.filter(i => i.cmd === 'write_to_pty');
}

function followUpWrites() {
  return writeToPtyCalls().filter(i => typeof i.args?.data === 'string' && (i.args!.data as string).includes('\x1b[200~'));
}

describe('Terminal follow-up delivery (wired)', () => {
  it('does not deliver the follow-up before a readiness marker arrives', async () => {
    act(() => {
      root.render(
        <Terminal sessionId={null} isVisible initialCommand="claude" followUpInput="do the thing" />,
      );
    });
    await flushAsyncSetup();

    // scheduleInitialCommand's own 300ms timer firing "claude\n" is fine —
    // only the bracketed-paste follow-up must be withheld.
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });

    expect(followUpWrites()).toEqual([]);
  });

  it('delivers exactly once after a readiness marker + settle', async () => {
    act(() => {
      root.render(
        <Terminal sessionId={null} isVisible initialCommand="claude" followUpInput="do the thing" />,
      );
    });
    await flushAsyncSetup();

    // Bracketed paste PLUS the alternate screen: 2004 on its own is raw mode,
    // which Claude Code also sets for its workspace-trust dialog. See
    // followUpDelivery.ts and harnessStartupFixtures.ts.
    emitPtyData('\x1b[?2004h\x1b[?1049h');
    await act(async () => { await vi.advanceTimersByTimeAsync(250); });

    const delivered = followUpWrites();
    expect(delivered).toHaveLength(1);
    expect(delivered[0].args?.data).toBe('\x1b[200~do the thing\x1b[201~\r');
    expect(delivered[0].args?.sessionId).toBe('pty-fresh');

    // A later marker must not deliver again.
    // Bracketed paste PLUS the alternate screen: 2004 on its own is raw mode,
    // which Claude Code also sets for its workspace-trust dialog. See
    // followUpDelivery.ts and harnessStartupFixtures.ts.
    emitPtyData('\x1b[?2004h\x1b[?1049h');
    await act(async () => { await vi.advanceTimersByTimeAsync(250); });
    expect(followUpWrites()).toHaveLength(1);
  });

  it('shows the pending chip on the ceiling and "Send now" delivers it', async () => {
    act(() => {
      root.render(
        <Terminal sessionId={null} isVisible initialCommand="claude" followUpInput="do the thing" />,
      );
    });
    await flushAsyncSetup();

    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });

    expect(followUpWrites()).toEqual([]);
    const status = container.querySelector('[role="status"]');
    expect(status, 'pending chip should be in the DOM after the ceiling').toBeTruthy();
    expect(status?.textContent).toMatch(/not delivered/i);

    const sendNowBtn = Array.from(container.querySelectorAll('button'))
      .find(b => /send now/i.test(b.textContent || ''));
    expect(sendNowBtn, 'Send now button should be present').toBeTruthy();

    act(() => {
      sendNowBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await flushAsyncSetup();

    const delivered = followUpWrites();
    expect(delivered).toHaveLength(1);
    expect(delivered[0].args?.data).toBe('\x1b[200~do the thing\x1b[201~\r');

    // The chip clears once delivery happens.
    expect(container.querySelector('[role="status"]')).toBeNull();
  });
});
