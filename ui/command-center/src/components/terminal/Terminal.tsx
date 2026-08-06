import { useEffect, useRef } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { useEventBus } from '../../lib/eventBus';
import { useTheme } from '../../styles/useTheme';
import { getXtermTheme } from './xtermTheme';
import { onRepaintRegain } from '../../lib/repaintOnRegain';
import { handlePtyData, type PtyDataPayload, type PtyStreamSink } from './ptyStream';

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

export function scheduleInitialCommand(
  invoke: TauriApi['invoke'],
  sessionId: string,
  command: string,
): () => void {
  const timer = setTimeout(() => {
    invoke('write_to_pty', { sessionId, data: `${command}\n` }).catch(() => {});
  }, 300);
  return () => clearTimeout(timer);
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
    let cancelInitialCommand: (() => void) | null = null;

    (async () => {
      if (!containerRef.current) return;

      const api = await getTauriApi();
      if (cancelled) return;

      const term = new XTerm({
        theme: getXtermTheme(theme, colors),
        fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, "DejaVu Sans Mono", monospace',
        fontSize: 13,
        // MUST stay 1.0. Extra leading inserts a gap between rows that the
        // glyph cannot bridge, so every vertical box-drawing rule (│ ┃ ║) is
        // sliced into dashes and long horizontal rules (─ ━) drift off the
        // text baseline — which is what made a coding harness's boxed output
        // look broken and struck through. The DOM renderer cannot compensate:
        // `customGlyphs` (which stretches box glyphs to fill the cell) only
        // applies to the canvas/WebGL renderers.
        lineHeight: 1,
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

      // Suppress macOS AutoFill on xterm's hidden input.
      //
      // xterm sets autocorrect/autocapitalize/spellcheck on its helper
      // textarea but NOT `autocomplete`, which is the attribute WebKit's
      // AutoFill actually consults. Without it, focusing the terminal could pop
      // the native "code from Messages" suggestion — and while that popup is up
      // it owns the arrow keys, so Up/Down never reach the PTY and TUI prompts
      // (a coding harness's approval gate) become unanswerable. Killing the
      // popup is what restores arrow-key navigation.
      const helper = containerRef.current!.querySelector<HTMLTextAreaElement>(
        'textarea.xterm-helper-textarea',
      );
      if (helper) {
        helper.setAttribute('autocomplete', 'off');
        helper.setAttribute('aria-autocomplete', 'none');
        // A neutral name/id keeps field-name heuristics from reading this as a
        // verification-code input.
        helper.setAttribute('name', 'permagent-terminal-input');
        helper.setAttribute('id', 'permagent-terminal-input');
      }

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
            cancelInitialCommand = scheduleInitialCommand(api.invoke, result.session_id, cmd);
          }
        } catch (err) {
          term.writeln('\r\n\x1b[31mFailed to spawn terminal: ' + err + '\x1b[0m\r\n');
          return;
        }
      }

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

      // Set up PTY data/exit listeners.
      // Render discipline (#573): PTY bytes go to xterm VERBATIM via
      // handlePtyData — the frontend never injects or strips characters.
      // See ptyStream.ts for why the #239 local-echo path was removed.
      // ── Replay/live handoff ────────────────────────────────────────────
      //
      // The Rust side appends every chunk to a bounded replay buffer AND emits
      // it as `pty_data`, so the two carry the SAME bytes. Subscribing first
      // and then writing the replay wrote the overlap twice — a fragment of
      // earlier output landing on top of whatever the TUI was drawing, which
      // is how harness text ended up spliced into Claude Code's input line
      // (reported 2026-08-04). Subscribing AFTER the replay instead would lose
      // whatever arrived during the round trip.
      //
      // So: subscribe first and HOLD live chunks, then write the replay, then
      // release only the chunks the replay did not already cover. `seq` is a
      // stream position, so the comparison is exact in both directions.
      let replayUpTo: number | null = null;
      const held: Array<{ data: string; seq?: number }> = [];

      const liveSink: PtyStreamSink = {
        write: (data) => {
          if (replayUpTo === null) { held.push({ data, seq: pendingSeq }); return; }
          term.write(data);
          scheduleFlush();
        },
        onCwd: (path) => onCwdChangeRef.current?.(path),
      };
      // handlePtyData hands the sink only the string, so the chunk's seq rides
      // alongside it rather than through the sink signature.
      let pendingSeq: number | undefined;

      const releaseHeld = (upTo: number | null) => {
        replayUpTo = upTo ?? 0;
        for (const chunk of held) {
          // No seq (older daemon) → keep it: a duplicate is recoverable, a
          // missing chunk is not.
          if (chunk.seq !== undefined && upTo !== null && chunk.seq <= upTo) continue;
          term.write(chunk.data);
        }
        held.length = 0;
        scheduleFlush();
      };

      let unlistenData: (() => void) | null = null;
      let unlistenExit: (() => void) | null = null;

      if (api) {
        unlistenData =
          (await api.listen('pty_data', (e) => {
            const payload = e.payload as PtyDataPayload;
            pendingSeq = payload.seq;
            handlePtyData(payload, sessionIdRef.current, liveSink);
            pendingSeq = undefined;
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
        if (cancelled) {
          unlistenData?.();
          unlistenExit?.();
          term.dispose();
          return;
        }

        // A detached window owns a fresh xterm renderer but reconnects to the
        // same live PTY. Rehydrate its scrollback from the bounded Rust buffer.
        if (sessionIdRef.current) {
          try {
            const replay = await api.invoke('get_pty_output', {
              sessionId: sessionIdRef.current,
            }) as { data: string; seq: number };
            if (!cancelled && replay?.data) term.write(replay.data);
            if (!cancelled) releaseHeld(replay?.seq ?? null);
          } catch {
            // Session may have exited during the handoff — release anyway, or
            // the terminal would sit mute with live bytes stuck in the queue.
            if (!cancelled) releaseHeld(null);
          }
        } else {
          // Fresh session: nothing to replay, so nothing to hold back.
          releaseHeld(null);
        }
        // The (re)attached PTY still has the PREVIOUS mount's grid: the
        // initial fit() above ran before onResize is registered below, so
        // this xterm's dimensions never reach resize_pty on their own —
        // and a TUI keeps painting for the old width, which is the garbled
        // approval-gate bug after a Build pane toggle. Sync the grid
        // explicitly; for a freshly spawned PTY this is a same-size no-op.
        if (sessionIdRef.current) {
          api.invoke('resize_pty', {
            sessionId: sessionIdRef.current,
            cols: term.cols,
            rows: term.rows,
          }).catch(() => {});
        }
        if (cancelled) {
          unlistenData?.();
          unlistenExit?.();
          term.dispose();
          return;
        }
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
        // No local echo: keystrokes are forwarded to the PTY only, and the
        // authoritative echo comes back through the stream (#573 — a second
        // writer here corrupted TUI status-line repaints; see ptyStream.ts).
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
        if (resizeTimer) clearTimeout(resizeTimer);
        if (flushTimer) clearTimeout(flushTimer);
        cancelInitialCommand?.();
        resizeObserver.disconnect();
        term.dispose();
        xtermRef.current = null;
        fitAddonRef.current = null;
      };
    })();

    return () => {
      cancelled = true;
      cancelInitialCommand?.();
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
