import { useEffect, useRef, useCallback } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { useEventBus } from '../../lib/eventBus';

let invoke: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null = null;
let listen: ((event: string, handler: (e: { payload: unknown }) => void) => Promise<() => void>) | null = null;

async function loadTauriApi() {
  try {
    const core = await import('@tauri-apps/api/core');
    const event = await import('@tauri-apps/api/event');
    invoke = core.invoke;
    listen = event.listen;
  } catch {
    // Running in browser
  }
}

const THEME = {
  background: '#0A0E17',
  foreground: '#e2e8f0',
  cursor: '#00ffb4',
  cursorAccent: '#0A0E17',
  selectionBackground: 'rgba(0, 255, 180, 0.2)',
  selectionForeground: '#e2e8f0',
  black: '#1e293b',
  red: '#ef4444',
  green: '#22c55e',
  yellow: '#f59e0b',
  blue: '#3b82f6',
  magenta: '#a855f7',
  cyan: '#06b6d4',
  white: '#e2e8f0',
  brightBlack: '#64748b',
  brightRed: '#f87171',
  brightGreen: '#4ade80',
  brightYellow: '#fbbf24',
  brightBlue: '#60a5fa',
  brightMagenta: '#c084fc',
  brightCyan: '#22d3ee',
  brightWhite: '#f8fafc',
};

interface TerminalProps {
  sessionId: string | null;
  onSessionSpawned?: (sessionId: string) => void;
  cwd?: string;
  isVisible: boolean;
}

export function Terminal({ sessionId, onSessionSpawned, cwd, isVisible }: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);
  const sessionIdRef = useRef<string | null>(sessionId);

  sessionIdRef.current = sessionId;

  const initTerminal = useCallback(async () => {
    if (!containerRef.current) return;

    await loadTauriApi();

    const term = new XTerm({
      theme: THEME,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace',
      fontSize: 13,
      lineHeight: 1.3,
      cursorBlink: true,
      cursorStyle: 'bar',
      allowProposedApi: true,
      scrollback: 10000,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(new WebLinksAddon());

    term.open(containerRef.current);

    try {
      term.loadAddon(new WebglAddon());
    } catch {
      // WebGL not available
    }

    fitAddon.fit();

    xtermRef.current = term;
    fitAddonRef.current = fitAddon;

    if (!sessionIdRef.current && invoke) {
      try {
        const cols = term.cols;
        const rows = term.rows;
        const sid = await invoke('spawn_pty_session', {
          shell: null,
          cwd: cwd || null,
          cols,
          rows,
        }) as string;
        sessionIdRef.current = sid;
        onSessionSpawned?.(sid);
      } catch (err) {
        term.writeln('\r\n\x1b[31mFailed to spawn terminal: ' + err + '\x1b[0m\r\n');
        return;
      }
    }

    let unlistenData: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;

    if (listen) {
      unlistenData = await listen('pty_data', (e) => {
        const payload = e.payload as { session_id: string; data: string };
        if (payload.session_id === sessionIdRef.current) {
          term.write(payload.data);
        }
      }) ?? null;

      unlistenExit = await listen('pty_exit', (e) => {
        const payload = e.payload as { session_id: string; code?: number };
        if (payload.session_id === sessionIdRef.current) {
          term.writeln('\r\n\x1b[33m[Process exited]\x1b[0m');
        }
      }) ?? null;
    }

    let inputBuffer = '';

    const onDataDisposable = term.onData((data) => {
      if (invoke && sessionIdRef.current) {
        invoke('write_to_pty', {
          sessionId: sessionIdRef.current,
          data,
        }).catch(() => {});
      }

      // Detect Enter to emit terminal_command event to daemon event bus
      if (data === '\r' || data === '\n') {
        const command = inputBuffer.trim();
        if (command.length > 0) {
          useEventBus.getState().addEvent({
            id: 'term-' + Date.now(),
            type: 'task_created',
            timestamp: new Date().toISOString(),
            payload: {
              source: 'terminal',
              command,
              session_id: sessionIdRef.current,
            },
          });
        }
        inputBuffer = '';
      } else if (data === '\x7f') {
        inputBuffer = inputBuffer.slice(0, -1);
      } else if (data.length === 1 && data >= ' ') {
        inputBuffer += data;
      }
    });

    const onResizeDisposable = term.onResize(({ cols, rows }) => {
      if (invoke && sessionIdRef.current) {
        invoke('resize_pty', {
          sessionId: sessionIdRef.current,
          cols,
          rows,
        }).catch(() => {});
      }
    });

    const resizeObserver = new ResizeObserver(() => {
      if (fitAddonRef.current && isVisible) {
        try {
          fitAddonRef.current.fit();
        } catch {
          // ignore fit errors during layout transitions
        }
      }
    });
    resizeObserver.observe(containerRef.current);

    cleanupRef.current = () => {
      onDataDisposable.dispose();
      onResizeDisposable.dispose();
      unlistenData?.();
      unlistenExit?.();
      resizeObserver.disconnect();
      term.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
    };
  }, [cwd, onSessionSpawned, isVisible]);

  useEffect(() => {
    initTerminal();
    return () => {
      cleanupRef.current?.();
    };
  }, [initTerminal]);

  useEffect(() => {
    if (isVisible && fitAddonRef.current) {
      const timer = setTimeout(() => {
        try {
          fitAddonRef.current?.fit();
        } catch {
          // ignore
        }
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [isVisible]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k' && isVisible) {
        e.preventDefault();
        xtermRef.current?.clear();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [isVisible]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full"
      style={{ backgroundColor: '#0A0E17' }}
    />
  );
}
