import { useEffect, useRef } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { useEventBus } from '../../lib/eventBus';
import { useTheme } from '../../styles/useTheme';
import { getXtermTheme } from './xtermTheme';
import { onRepaintRegain } from '../../lib/repaintOnRegain';

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

interface TerminalProps {
  sessionId: string | null;
  onSessionSpawned?: (sessionId: string) => void;
  onTitleChange?: (title: string) => void;
  onCwdChange?: (cwd: string) => void;
  cwd?: string;
  initialCommand?: string;
  /** S2 (#428): supervised loop session id (`sup-<uuid>`) — passed to
   *  `spawn_pty_session` so the Rust PTY reader tees output to the daemon. */
  supervisedSessionId?: string;
  isVisible: boolean;
}

export function Terminal({ sessionId, onSessionSpawned, onTitleChange, onCwdChange, cwd, initialCommand, supervisedSessionId, isVisible }: TerminalProps) {
  const { theme, colors } = useTheme();
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);
  const sessionIdRef = useRef<string | null>(sessionId);
  // Stable refs for values that change but shouldn't re-trigger the effect
  const onSessionSpawnedRef = useRef(onSessionSpawned);
  const onTitleChangeRef = useRef(onTitleChange);
  const onCwdChangeRef = useRef(onCwdChange);
  const cwdRef = useRef(cwd);
  const initialCommandRef = useRef(initialCommand);
  const supervisedSessionIdRef = useRef(supervisedSessionId);

  sessionIdRef.current = sessionId;
  onSessionSpawnedRef.current = onSessionSpawned;
  onTitleChangeRef.current = onTitleChange;
  onCwdChangeRef.current = onCwdChange;
  cwdRef.current = cwd;
  initialCommandRef.current = initialCommand;
  supervisedSessionIdRef.current = supervisedSessionId;

  // ── Single setup effect — runs once on mount, cleans up on unmount ──
  useEffect(() => {
    let cancelled = false;

    (async () => {
      if (!containerRef.current) return;

      const api = await getTauriApi();
      if (cancelled) return;

      const term = new XTerm({
        theme: getXtermTheme(theme, colors),
        fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, "DejaVu Sans Mono", monospace',
        fontSize: 13,
        lineHeight: 1.15,
        cursorBlink: true,
        cursorStyle: 'bar',
        allowProposedApi: true,
        scrollback: 10000,
        customGlyphs: true,
        rescaleOverlappingGlyphs: true,
        drawBoldTextInBrightColors: false,
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.loadAddon(new WebLinksAddon());

      term.open(containerRef.current!);

      // Use default DOM renderer — WebGL addon causes banding and Unicode
      // rendering issues (box-drawing chars show as ??). DOM renderer
      // handles Unicode, 256-color, and true-color correctly.

      fitAddon.fit();

      xtermRef.current = term;
      fitAddonRef.current = fitAddon;

      // Spawn PTY if we don't have a session yet
      if (!sessionIdRef.current && api) {
        try {
          const cols = term.cols;
          const rows = term.rows;
          const result = (await api.invoke('spawn_pty_session', {
            shell: null,
            cwd: cwdRef.current || null,
            cols,
            rows,
            supervisedSessionId: supervisedSessionIdRef.current ?? null,
          })) as { session_id: string; cwd: string };
          if (cancelled) return;
          sessionIdRef.current = result.session_id;
          onSessionSpawnedRef.current?.(result.session_id);
          // Always report the resolved CWD so the tab shows a folder name
          // immediately. Later OSC 7 or title changes will override it
          // if the shell navigates elsewhere.
          if (result.cwd) {
            onCwdChangeRef.current?.(result.cwd);
          }
          // Activity: terminal session started
          api.invoke('emit_activity', {
            event_type: 'terminal_session_started',
            source_surface: 'terminal',
            payload: { session_id: result.session_id, working_directory: result.cwd || null },
            session_id: null,
            project_id: null,
          }).catch((err: unknown) => console.debug('[activity] terminal_session_started emission failed:', err));

          // Send initial command (e.g. "claude" or "codex") after shell is ready
          if (initialCommandRef.current) {
            const cmd = initialCommandRef.current;
            initialCommandRef.current = undefined; // fire once
            setTimeout(() => {
              api.invoke('write_to_pty', {
                sessionId: result.session_id,
                data: cmd + '\n',
              }).catch(() => {});
            }, 300);
          }
        } catch (err) {
          term.writeln('\r\n\x1b[31mFailed to spawn terminal: ' + err + '\x1b[0m\r\n');
          return;
        }
      }

      // Track whether the PTY has echo enabled. When echo is off (password
      // prompts, ssh), we must NOT local-echo to avoid flashing plaintext.
      let echoEnabled = true;

      // Pending local echo queue: characters we've already rendered locally.
      // When the authoritative PTY echo arrives, we strip matching leading
      // characters to avoid rendering them twice.
      const pendingEchos: { char: string; time: number }[] = [];

      // Coalesced force-flush of the DOM renderer after PTY output. xterm's
      // write schedules a render via requestAnimationFrame; when the main
      // webview is occlusion-throttled (the native browser sibling is forward),
      // that frame can be dropped, leaving the rendered view stale after the
      // shell already advanced — e.g. an answered CC prompt that stays on
      // screen (#555). A short post-write refresh re-queues the render so the
      // view self-heals as soon as a frame lands.
      let flushTimer: ReturnType<typeof setTimeout> | null = null;
      const scheduleFlush = () => {
        if (flushTimer) return; // coalesce a burst into one refresh
        flushTimer = setTimeout(() => {
          flushTimer = null;
          const t = xtermRef.current;
          if (t) {
            try {
              t.refresh(0, t.rows - 1);
            } catch {
              /* ignore refresh during teardown/layout transitions */
            }
          }
        }, 80);
      };

      // Set up PTY data/exit listeners
      let unlistenData: (() => void) | null = null;
      let unlistenExit: (() => void) | null = null;
      let unlistenEcho: (() => void) | null = null;

      if (api) {
        unlistenData =
          (await api.listen('pty_data', (e) => {
            const payload = e.payload as { session_id: string; data: string };
            if (payload.session_id === sessionIdRef.current) {
              // Strip leading characters that match pending local echoes
              // to prevent duplicate rendering. Expire stale entries (>2s).
              const now = performance.now();
              while (pendingEchos.length > 0 && now - pendingEchos[0].time > 2000) {
                pendingEchos.shift();
              }
              let output = payload.data;
              while (pendingEchos.length > 0 && output.length > 0 && output[0] === pendingEchos[0].char) {
                pendingEchos.shift();
                output = output.substring(1);
              }
              if (output.length > 0) {
                term.write(output);
                scheduleFlush();
              }

              // Parse OSC 7 (CWD reporting) from the full data, not the stripped output
              const osc7 = payload.data.match(/\x1b\]7;file:\/\/[^/]*([^\x07\x1b]+)/);
              if (osc7) {
                try {
                  const decoded = decodeURIComponent(osc7[1]);
                  onCwdChangeRef.current?.(decoded);
                } catch { /* ignore decode errors */ }
              }
            }
          })) ?? null;

        unlistenExit =
          (await api.listen('pty_exit', (e) => {
            const payload = e.payload as { session_id: string; code?: number };
            if (payload.session_id === sessionIdRef.current) {
              term.writeln('\r\n\x1b[33m[Process exited]\x1b[0m');
              // L2-observe (#400): forward the process-exit signal to the bus.
              if (api) {
                api.invoke('emit_activity', {
                  event_type: 'terminal_process_exited',
                  source_surface: 'terminal',
                  payload: { session_id: sessionIdRef.current, exit_code: payload.code ?? null },
                  session_id: null,
                  project_id: null,
                }).catch((err: unknown) => console.debug('[activity] terminal_process_exited emission failed:', err));
              }
            }
          })) ?? null;

        unlistenEcho =
          (await api.listen('pty_echo_state', (e) => {
            const payload = e.payload as { session_id: string; echo_enabled: boolean };
            if (payload.session_id === sessionIdRef.current) {
              echoEnabled = payload.echo_enabled;
            }
          })) ?? null;
      }

      // Forward keystrokes to PTY
      let inputBuffer = '';
      // Last Enter-sniffed command awaiting its OSC 133;D completion mark
      // (the zsh precmd hook injected by terminal.rs). Null when no command
      // is in flight — the mark also fires on bare prompts, which are ignored.
      let pendingCommand: string | null = null;

      // OSC 133;D;<exit> — pair the completion mark with the sniffed command
      // and emit terminal_command_completed (the initiative layer's #360
      // signal). Same transitional frontend-emission caveat as onData below.
      const oscHandlerDisposable = term.parser.registerOscHandler(133, (data) => {
        if (!data.startsWith('D')) return false;
        const command = pendingCommand;
        pendingCommand = null;
        if (command && api) {
          const exitCode = Number(data.split(';')[1]);
          api.invoke('emit_activity', {
            event_type: 'terminal_command_completed',
            source_surface: 'terminal',
            payload: {
              command,
              exit_code: Number.isFinite(exitCode) ? exitCode : null,
              working_directory: cwdRef.current || null,
            },
            session_id: null,
            project_id: null,
          }).catch((err: unknown) => console.debug('[activity] terminal_command_completed emission failed:', err));
        }
        return true;
      });

      const onDataDisposable = term.onData((data) => {
        // Local echo: render printable ASCII immediately for instant feedback.
        // Gated on PTY ECHO flag — suppressed during password prompts and
        // other echo-off modes to avoid flashing plaintext.
        if (echoEnabled && data.length === 1 && data >= ' ' && data <= '~') {
          term.write(data);
          pendingEchos.push({ char: data, time: performance.now() });
        }

        if (api && sessionIdRef.current) {
          api.invoke('write_to_pty', {
            sessionId: sessionIdRef.current,
            data,
          }).catch(() => {});
        }

        // Detect Enter to emit terminal_command event
        // TODO: Frontend-driven emission is transitional. Lifecycle hooks should move
        // to Rust-owned surfaces in Phase 2.5 (see docs/architecture/PHASE_2_5_TAURI_REFACTOR.md).
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
            // Activity: terminal command started
            if (api) {
              api.invoke('emit_activity', {
                event_type: 'terminal_command_started',
                source_surface: 'terminal',
                payload: { command, working_directory: cwdRef.current || null },
                session_id: null,
                project_id: null,
              }).catch((err: unknown) => console.debug('[activity] terminal_command_started emission failed:', err));
            }
            pendingCommand = command;
          }
          inputBuffer = '';
        } else if (data === '\x7f') {
          inputBuffer = inputBuffer.slice(0, -1);
        } else if (data.length === 1 && data >= ' ') {
          inputBuffer += data;
        }
      });

      const onTitleDisposable = term.onTitleChange((title) => {
        onTitleChangeRef.current?.(title);
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

      // Debounce ResizeObserver to avoid sending rapid intermediate
      // dimensions to the PTY. Without this, TUI apps (Claude Code, vim)
      // can render their status bar at a stale row position, causing
      // duplicate/ghost status lines during panel resizes.
      let resizeTimer: ReturnType<typeof setTimeout> | null = null;
      const resizeObserver = new ResizeObserver(() => {
        if (resizeTimer) clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          // Gate on REAL rendered size, not `isVisible`. When an ancestor
          // (the workspace div in App.tsx, or the settings overlay) is hidden
          // with display:none, this terminal's tab can still be "active"
          // (isVisible === true), so an isVisible gate would let the observer's
          // collapse-fire run fit() on a 0-box. FitAddon then reads a stale
          // computed width (~100px) and reflows the terminal to a ~10-col
          // sliver — the reflow-on-tab-return bug. offsetWidth/Height are 0
          // whenever any ancestor is display:none, so this guard skips the
          // corrupting fit AND, because the show transition (0 -> full) also
          // fires this observer, re-fits to full width on return.
          const el = containerRef.current;
          if (fitAddonRef.current && el && el.offsetWidth > 0 && el.offsetHeight > 0) {
            try {
              fitAddonRef.current.fit();
            } catch {
              // ignore fit errors during layout transitions
            }
          }
        }, 100);
      });
      resizeObserver.observe(containerRef.current!);

      cleanupRef.current = () => {
        // Activity: terminal session ended
        if (api && sessionIdRef.current) {
          api.invoke('emit_activity', {
            event_type: 'terminal_session_ended',
            source_surface: 'terminal',
            payload: { session_id: sessionIdRef.current },
            session_id: null,
            project_id: null,
          }).catch((err: unknown) => console.debug('[activity] terminal_session_ended emission failed:', err));
        }
        oscHandlerDisposable.dispose();
        onDataDisposable.dispose();
        onTitleDisposable.dispose();
        onResizeDisposable.dispose();
        unlistenData?.();
        unlistenExit?.();
        unlistenEcho?.();
        if (resizeTimer) clearTimeout(resizeTimer);
        if (flushTimer) clearTimeout(flushTimer);
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

  // Update xterm theme when app theme changes
  // (colors has stable per-theme identity, so this fires exactly on theme change)
  useEffect(() => {
    if (xtermRef.current) {
      xtermRef.current.options.theme = getXtermTheme(theme, colors);
    }
  }, [theme, colors]);

  // Re-fit when this tab becomes the active tab (fast path for the in-manager
  // tab switch). Guarded on real size for the same reason as the ResizeObserver:
  // never fit a collapsed (display:none-ancestor) container, or it reflows to a
  // ~10-col sliver.
  useEffect(() => {
    if (isVisible && fitAddonRef.current) {
      const timer = setTimeout(() => {
        const el = containerRef.current;
        if (fitAddonRef.current && el && el.offsetWidth > 0 && el.offsetHeight > 0) {
          try {
            fitAddonRef.current.fit();
          } catch {
            // ignore
          }
        }
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [isVisible]);

  // Force-flush the DOM renderer when the webview regains focus/visibility.
  // After macOS occlusion throttling freezes this view (terminal vanishes on
  // focus-loss in fullscreen #517; CC pane blank #551), the buffer is current
  // but the painted frame is stale — re-fit (guarded) and refresh so it
  // self-heals immediately on regain instead of waiting for an interaction.
  useEffect(() => {
    return onRepaintRegain(() => {
      const el = containerRef.current;
      const term = xtermRef.current;
      if (!term || !el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      try {
        fitAddonRef.current?.fit();
        term.refresh(0, term.rows - 1);
      } catch {
        /* ignore during layout transitions */
      }
    });
  }, []);

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

  const xtermBg = getXtermTheme(theme, colors).background;

  return (
    <div
      ref={containerRef}
      className="h-full w-full"
      style={{ backgroundColor: xtermBg }}
    />
  );
}
