import { useEffect, useRef } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { useEventBus } from '../../lib/eventBus';

// ── Tauri API loader (cached, no module-level mutation) ──

interface TauriApi {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (event: string, handler: (e: { payload: unknown }) => void) => Promise<() => void>;
}

let cachedApi: TauriApi | null = null;
let apiPromise: Promise<TauriApi | null> | null = null;

function getTauriApi(): Promise<TauriApi | null> {
  if (cachedApi) return Promise.resolve(cachedApi);
  if (!apiPromise) {
    apiPromise = (async () => {
      try {
        const core = await import('@tauri-apps/api/core');
        const event = await import('@tauri-apps/api/event');
        cachedApi = { invoke: core.invoke, listen: event.listen };
        return cachedApi;
      } catch {
        return null;
      }
    })();
  }
  return apiPromise;
}

const THEME = {
  background: '#0A0E17',
  foreground: '#e2e8f0',
  cursor: '#00D5FF',
  cursorAccent: '#0A0E17',
  selectionBackground: 'rgba(0, 213, 255, 0.2)',
  selectionForeground: '#e2e8f0',
  black: '#1e293b',
  red: '#ef4444',
  green: '#5BD17F',
  yellow: '#f59e0b',
  blue: '#3b82f6',
  magenta: '#A855CC',
  cyan: '#00D5FF',
  white: '#e2e8f0',
  brightBlack: '#64748b',
  brightRed: '#f87171',
  brightGreen: '#5BD17F',
  brightYellow: '#fbbf24',
  brightBlue: '#60a5fa',
  brightMagenta: '#C893E0',
  brightCyan: '#00D5FF',
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
  // Stable refs for values that change but shouldn't re-trigger the effect
  const onSessionSpawnedRef = useRef(onSessionSpawned);
  const cwdRef = useRef(cwd);
  const isVisibleRef = useRef(isVisible);

  sessionIdRef.current = sessionId;
  onSessionSpawnedRef.current = onSessionSpawned;
  cwdRef.current = cwd;
  isVisibleRef.current = isVisible;

  // ── Single setup effect — runs once on mount, cleans up on unmount ──
  useEffect(() => {
    let cancelled = false;

    (async () => {
      if (!containerRef.current) return;

      const api = await getTauriApi();
      if (cancelled) return;

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

      term.open(containerRef.current!);

      try {
        term.loadAddon(new WebglAddon());
      } catch {
        // WebGL not available
      }

      fitAddon.fit();

      xtermRef.current = term;
      fitAddonRef.current = fitAddon;

      // Spawn PTY if we don't have a session yet
      if (!sessionIdRef.current && api) {
        try {
          const cols = term.cols;
          const rows = term.rows;
          const sid = (await api.invoke('spawn_pty_session', {
            shell: null,
            cwd: cwdRef.current || null,
            cols,
            rows,
          })) as string;
          if (cancelled) return;
          sessionIdRef.current = sid;
          onSessionSpawnedRef.current?.(sid);
        } catch (err) {
          term.writeln('\r\n\x1b[31mFailed to spawn terminal: ' + err + '\x1b[0m\r\n');
          return;
        }
      }

      // Set up PTY data/exit listeners
      let unlistenData: (() => void) | null = null;
      let unlistenExit: (() => void) | null = null;

      if (api) {
        unlistenData =
          (await api.listen('pty_data', (e) => {
            const payload = e.payload as { session_id: string; data: string };
            if (payload.session_id === sessionIdRef.current) {
              term.write(payload.data);
            }
          })) ?? null;

        unlistenExit =
          (await api.listen('pty_exit', (e) => {
            const payload = e.payload as { session_id: string; code?: number };
            if (payload.session_id === sessionIdRef.current) {
              term.writeln('\r\n\x1b[33m[Process exited]\x1b[0m');
            }
          })) ?? null;
      }

      // Forward keystrokes to PTY
      let inputBuffer = '';

      const onDataDisposable = term.onData((data) => {
        if (api && sessionIdRef.current) {
          api.invoke('write_to_pty', {
            sessionId: sessionIdRef.current,
            data,
          }).catch(() => {});
        }

        // Detect Enter to emit terminal_command event
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
        if (api && sessionIdRef.current) {
          api.invoke('resize_pty', {
            sessionId: sessionIdRef.current,
            cols,
            rows,
          }).catch(() => {});
        }
      });

      const resizeObserver = new ResizeObserver(() => {
        if (fitAddonRef.current && isVisibleRef.current) {
          try {
            fitAddonRef.current.fit();
          } catch {
            // ignore fit errors during layout transitions
          }
        }
      });
      resizeObserver.observe(containerRef.current!);

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
    })();

    return () => {
      cancelled = true;
      cleanupRef.current?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Mount once — props accessed through stable refs

  // Re-fit when visibility changes
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

  // Cmd+K to clear
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
