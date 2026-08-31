import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import '@xterm/xterm/css/xterm.css';
import { useEventBus } from '../../lib/eventBus';
import { useTheme } from '../../styles/useTheme';
import { font, radius, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { getXtermTheme } from './xtermTheme';
import { onRepaintRegain } from '../../lib/repaintOnRegain';
import { handlePtyData, type PtyDataPayload, type PtyStreamSink } from './ptyStream';
import {
  FALLBACK_PTY_GRID,
  advertisedGrid,
  containerCanFit,
  fitVisibleTerminal,
  remeasureXterm,
  subscribeTerminalFonts,
} from './ptyGrid';
import { api as daemonApi } from '../../lib/api';
import { buildCodingSessionPayload, isHarnessCommand } from './codingSession';
import { createFollowUpDelivery, type FollowUpDelivery } from './followUpDelivery';
import { FiCopy, FiSend, FiX } from 'react-icons/fi';

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
  /** Pasted after `initialCommand` has had time to start a coding TUI. */
  followUpInput?: string;
  /** Growth action to mark implemented when this harness session ends. */
  growthAction?: { projectId: string; actionId: string };
  isVisible: boolean;
}

export function Terminal({ sessionId, onSessionSpawned, onTitleChange, onCwdChange, cwd, initialCommand, supervisedSessionId, followUpInput, growthAction, isVisible }: TerminalProps) {
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
  const followUpInputRef = useRef(followUpInput);
  const growthActionRef = useRef(growthAction);
  // The readiness-gated follow-up delivery for this mount (see
  // followUpDelivery.ts) — `sendNow()` backs the pending chip's button.
  const followUpDeliveryRef = useRef<FollowUpDelivery | null>(null);
  // Surfaced when the ceiling elapses without the TUI taking the tty; cleared
  // once delivery actually happens (including a late, post-ceiling delivery).
  const [pendingPrompt, setPendingPrompt] = useState<string | null>(null);

  sessionIdRef.current = sessionId;
  onSessionSpawnedRef.current = onSessionSpawned;
  onTitleChangeRef.current = onTitleChange;
  onCwdChangeRef.current = onCwdChange;
  cwdRef.current = cwd;
  initialCommandRef.current = initialCommand;
  supervisedSessionIdRef.current = supervisedSessionId;
  followUpInputRef.current = followUpInput;
  growthActionRef.current = growthAction;

  // ── Single setup effect — runs once on mount, cleans up on unmount ──
  useEffect(() => {
    let cancelled = false;
    let cancelInitialCommand: (() => void) | null = null;

    (async () => {
      if (!containerRef.current) return;

      // Captured BEFORE the spawn block consumes initialCommandRef (it clears
      // the ref to fire-once) — the coding-session capture below needs to
      // know a harness was launched here.
      const launchCommand = initialCommandRef.current ?? null;

      const api = await getTauriApi();
      if (cancelled) return;

      const term = new XTerm({
        theme: getXtermTheme(theme, colors),
        fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", Menlo, "DejaVu Sans Mono", monospace',
        fontSize: textSize.small,
        // MUST stay 1.0. Extra leading inserts a gap between rows that the
        // glyph cannot bridge, so every vertical box-drawing rule (│ ┃ ║) is
        // sliced into dashes and long horizontal rules (─ ━) drift off the
        // text baseline — which is what made a coding harness's boxed output
        // look broken and struck through. The DOM renderer cannot compensate:
        // `customGlyphs` (which stretches box glyphs to fill the cell) only
        // applies to the canvas/WebGL renderers.
        // Overlap is a GRID bug (ptyGrid.ts), not a reason to raise this.
        lineHeight: 1,
        cursorBlink: true,
        cursorStyle: 'bar',
        allowProposedApi: true,
        scrollback: 10000,
        customGlyphs: true,
        rescaleOverlappingGlyphs: true,
        drawBoldTextInBrightColors: false,
        // 1 = off. Anything higher rewrites 256/truecolor toward the default
        // fg so Claude's orange ✻, Cursor's palette, and the Permagent ribbon
        // all collapse to white-on-dark / gray-on-light.
        minimumContrastRatio: 1,
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.loadAddon(new WebLinksAddon());

      // Unicode 11 width tables. xterm.js ships Unicode 6 by default, which
      // gets the width of modern symbols wrong — and a coding harness's UI is
      // built almost entirely from them (✻ ⏵⏵ ⎿ ✱ ⚠).
      //
      // A width disagreement is not a cosmetic problem. The program counts a
      // glyph as one cell, the renderer draws two (or the reverse), and from
      // that point the two disagree about which column the cursor is in. The
      // next redraw that moves the cursor up to rewrite a line then lands on
      // the WRONG ROW: a second input line prints on top of the first instead
      // of below it, interleaving both into gibberish. Reported 2026-08-12
      // with the prompt rendered as "drivacyhcommissionerlicy analytics …" —
      // two lines occupying one row.
      //
      // Must be loaded AND activated: loading only registers the provider.
      const unicode11 = new Unicode11Addon();
      term.loadAddon(unicode11);
      term.unicode.activeVersion = '11';

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

      // Never fit a collapsed box: FitAddon floors 0×0 to 2×1, and a TUI
      // that paints for one row puts its status line on the prompt
      // (reported 2026-08-19). Tabs stay mounted behind display:none.
      fitVisibleTerminal(fitAddon, containerRef.current);

      xtermRef.current = term;
      fitAddonRef.current = fitAddon;

      const advertiseGrid = () => {
        if (!api || !sessionIdRef.current) return;
        if (!containerCanFit(containerRef.current)) return;
        const grid = advertisedGrid(term);
        if (!grid) return;
        api.invoke('resize_pty', {
          sessionId: sessionIdRef.current,
          cols: grid.cols,
          rows: grid.rows,
        }).catch(() => {});
      };

      // Spawn PTY if we don't have a session yet
      if (!sessionIdRef.current && api) {
        try {
          const grid = advertisedGrid(term) ?? FALLBACK_PTY_GRID;
          const result = (await api.invoke('spawn_pty_session', {
            shell: null,
            cwd: cwdRef.current || null,
            cols: grid.cols,
            rows: grid.rows,
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
            const follow = followUpInputRef.current;
            followUpInputRef.current = undefined;
            if (follow) {
              const spawnedSessionId = result.session_id;
              followUpDeliveryRef.current = createFollowUpDelivery({
                text: follow,
                write: (d) => {
                  void api.invoke('write_to_pty', { sessionId: spawnedSessionId, data: d }).catch(() => {});
                },
                onPending: () => setPendingPrompt(follow),
                onSent: () => setPendingPrompt(null),
              });
            }
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
          // Readiness for the follow-up paste (createFollowUpDelivery) is
          // read off these same bytes — feed the identical verbatim chunk.
          followUpDeliveryRef.current?.onData(data);
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
          followUpDeliveryRef.current?.onData(chunk.data);
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
            if (!cancelled && replay?.data) {
              term.write(replay.data);
              followUpDeliveryRef.current?.onData(replay.data);
            }
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
        // Skip a collapsed box: advertising 2×1 is the 2026-08-19 overlap.
        advertiseGrid();
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

      // ── Coding-session memory (#reported 2026-08-06) ──────────────────────
      // When a harness (claude/codex/permagent) finishes, ship the transcript
      // tail to the daemon → summarized → remembered in the Brain. Typed
      // launches are caught by the Enter sniffer; project-tab launches are
      // injected (never typed), so the pending launch command stands in for
      // the first completion mark that arrives after a real session length.
      const spawnedAt = Date.now();
      let pendingLaunchHarness: string | null =
        isHarnessCommand(launchCommand) ? launchCommand : null;
      const captureCodingSession = (command: string) => {
        void (async () => {
          try {
            if (!api || !sessionIdRef.current) return;
            const replay = (await api.invoke('get_pty_output', {
              sessionId: sessionIdRef.current,
            })) as { data: string } | null;
            const payload = buildCodingSessionPayload({
              rawTranscript: replay?.data ?? '',
              cwd: cwdRef.current,
              command,
              spawnedAtMs: spawnedAt,
              nowMs: Date.now(),
            });
            if (payload) {
              await daemonApi.codingSessionSummary(payload);
            }
          } catch (err) {
            // Memory is best-effort; the terminal itself must never care.
            console.debug('[terminal] coding-session summary failed:', err);
          }
        })();
      };

      const reportGrowthAction = () => {
        const ga = growthActionRef.current;
        if (!ga) return;
        growthActionRef.current = undefined;
        daemonApi.completeGrowthActionFromHarness(ga.projectId, ga.actionId)
          .catch((err: unknown) => {
            console.debug('[terminal] growth-action complete failed:', err);
          });
      };

      // OSC 133;D;<exit> — pair the completion mark with the sniffed command
      // and emit terminal_command_completed (the initiative layer's #360
      // signal). Same transitional frontend-emission caveat as onData below.
      const oscHandlerDisposable = term.parser.registerOscHandler(133, (data) => {
        if (!data.startsWith('D')) return false;
        const command = pendingCommand;
        pendingCommand = null;
        // Coding-session memory: a completed harness command ends a session.
        // Typed commands arrive via the sniffer; an injected project-tab
        // launch was never typed, so the first completion mark after a real
        // session length (>5s — the instant at-spawn prompt mark is not a
        // session) stands in for it.
        let sessionCommand = command;
        if (!sessionCommand && pendingLaunchHarness && Date.now() - spawnedAt > 5_000) {
          sessionCommand = pendingLaunchHarness;
          pendingLaunchHarness = null;
        }
        if (sessionCommand && isHarnessCommand(sessionCommand)) {
          captureCodingSession(sessionCommand);
          reportGrowthAction();
        }
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

      const onResizeDisposable = term.onResize(() => {
        advertiseGrid();
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
          fitVisibleTerminal(fitAddonRef.current, el);
        }, 100);
      });
      resizeObserver.observe(containerRef.current!);

      const unsubFonts = subscribeTerminalFonts(() => {
        if (cancelled) return;
        const t = xtermRef.current;
        if (!t) return;
        // JetBrains Mono arrives with display:swap after the first measure.
        // Re-measure, then fit, then tell the PTY — never rewrite the buffer.
        remeasureXterm(t);
        if (!fitVisibleTerminal(fitAddonRef.current, containerRef.current)) return;
        advertiseGrid();
      });

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
        unsubFonts();
        if (resizeTimer) clearTimeout(resizeTimer);
        if (flushTimer) clearTimeout(flushTimer);
        cancelInitialCommand?.();
        followUpDeliveryRef.current?.cancel();
        resizeObserver.disconnect();
        term.dispose();
        xtermRef.current = null;
        fitAddonRef.current = null;
      };
    })();

    return () => {
      cancelled = true;
      cancelInitialCommand?.();
      followUpDeliveryRef.current?.cancel();
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
        fitVisibleTerminal(fitAddonRef.current, el);
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
      if (!term || !containerCanFit(el)) return;
      try {
        fitVisibleTerminal(fitAddonRef.current, el);
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

  const dismissPending = () => setPendingPrompt(null);
  const sendPendingNow = () => followUpDeliveryRef.current?.sendNow();
  const copyPending = () => {
    if (!pendingPrompt) return;
    try {
      void navigator.clipboard?.writeText(pendingPrompt);
    } catch {
      /* clipboard access can be denied — the chip stays up either way */
    }
  };

  return (
    // The wrapper must NOT change the box `fitVisibleTerminal` and the
    // ResizeObserver measure below — it exists only to position the chip,
    // so the xterm container div stays byte-for-byte the same as before.
    <div className="relative h-full w-full">
      <div
        ref={containerRef}
        className="pty-terminal h-full w-full"
        style={{ backgroundColor: xtermBg }}
      />
      {pendingPrompt && (
        <div
          role="status"
          className="absolute bottom-2 right-2 z-10 max-w-xs rounded-lg px-3 py-2 text-xs"
          style={{
            backgroundColor: colors.surfaceHi,
            border: `1px solid ${colors.border}`,
            boxShadow: colors.elevationFloating,
            color: colors.text,
            fontFamily: font.body,
          }}
        >
          <div className="flex items-start justify-between gap-2">
            <span style={{ color: colors.textMuted }}>
              Prompt not delivered — the agent didn&apos;t take the terminal in time.
            </span>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={dismissPending}
              aria-label="Dismiss"
              className="shrink-0"
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontSize: textSize.caption,
              } as CSSProperties}
            >
              <FiX size={12} />
            </Button>
          </div>
          <div className="mt-2 flex items-center gap-2">
            <Button
              colors={colors}
              variant="primary"
              type="button"
              onClick={sendPendingNow}
              // `gap: 4` is the old `gap-1` between the icon and the word.
              style={{
                '--pa-btn-pad': '4px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                fontFamily: font.body,
                fontSize: textSize.caption,
                gap: 4,
              } as CSSProperties}
            >
              <FiSend size={11} /> Send now
            </Button>
            <Button
              colors={colors}
              type="button"
              onClick={copyPending}
              style={{
                '--pa-btn-pad': '4px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                fontFamily: font.body,
                fontSize: textSize.caption,
                gap: 4,
              } as CSSProperties}
            >
              <FiCopy size={11} /> Copy
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
