/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Connection model: persistent socket, opened once and kept alive.
 * Mic: acquired at activation, released at deactivation. Recording start/stop
 *       is synchronous (no getUserMedia race on quick press-and-release).
 * State machine: idle → connecting → ready → recording → processing → playing → ready
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { getApiBaseUrl } from '../lib/api';
import { getStreamToken } from '../lib/streamToken';

export type VoiceState =
  | 'idle'          // Voice off — no socket
  | 'connecting'    // WebSocket connecting, waiting for server ready
  | 'ready'         // Socket open + server ready, waiting for push-to-talk
  | 'recording'     // Push-to-talk held, capturing audio
  | 'processing'    // Audio sent, waiting for STT + reply + TTS
  | 'playing'       // Playing TTS response ("Speaking...")
  | 'error';        // Recoverable error (auto-recovers to ready/idle)

/**
 * Watchdog ceiling: max time a mic-locking state may sit with NO daemon
 * activity and NO local playback before we treat it as wedged and force a
 * recovery. Generously above the worst observed legitimate pre-stream latency
 * (~53s) so a slow-but-live reply is never cut off, while a genuine hang
 * (daemon emits no reply_end/error, socket stays open) self-heals.
 */
export const VOICE_WATCHDOG_MS = 90_000;

/**
 * Pure predicate: is a busy voice state wedged and in need of a forced clear?
 *
 * Wedged = in a mic-locking state (`processing`/`playing`), with NO audio
 * actively playing or queued (so it isn't making local progress), and no
 * daemon activity for longer than the threshold. Extracted so the watchdog
 * decision is unit-testable without a live audio session.
 */
export function isVoiceWedged(opts: {
  state: VoiceState;
  isPlaying: boolean;
  queuedChunks: number;
  msSinceActivity: number;
  thresholdMs?: number;
}): boolean {
  const {
    state,
    isPlaying,
    queuedChunks,
    msSinceActivity,
    thresholdMs = VOICE_WATCHDOG_MS,
  } = opts;
  if (state !== 'processing' && state !== 'playing') return false;
  if (isPlaying || queuedChunks > 0) return false;
  return msSinceActivity > thresholdMs;
}

/**
 * Pure predicate: is the voice loop in a state the user can barge in on?
 *
 * True while a reply is being produced or spoken (`processing`/`playing`) — the
 * states where "stop and let me talk" (#398) is meaningful. In `ready` a Space
 * press means start recording, not interrupt; in `idle`/`connecting`/`error`
 * there's nothing to stop. Extracted so the key/click routing is unit-testable
 * without a live audio session.
 */
export function isInterruptibleState(state: VoiceState): boolean {
  return state === 'processing' || state === 'playing';
}

export interface VoiceEvent {
  type: 'transcript' | 'reply_text' | 'reply_audio' | 'error' | 'state_change';
  text?: string;
  audio?: { samples: Float32Array; sampleRate: number };
  state?: VoiceState;
  error?: string;
}

/** A deferred navigation forwarded over the voice socket (speak-then-act). */
interface NavPayload {
  tab?: string;
  tool_type?: string;
  panel_type?: string;
  section?: string | null;
  state?: unknown;
  reason?: string;
}

interface UseVoiceOptions {
  sessionId?: string;
  sampleRate?: number;
  onEvent?: (event: VoiceEvent) => void;
}

export function useVoice(options: UseVoiceOptions = {}) {
  const { sessionId, sampleRate = 16000, onEvent } = options;
  const [state, setState] = useState<VoiceState>('idle');
  const [lastTranscript, setLastTranscript] = useState('');
  const [lastReply, setLastReply] = useState('');
  const [error, setError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  // Persistent mic stream — acquired at activation, released at deactivation.
  // Eliminates the getUserMedia async gap that caused the press-release race.
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const pendingAudioRef = useRef(false);
  const audioQueueRef = useRef<Float32Array[]>([]);
  const playingRef = useRef(false);
  const playbackCtxRef = useRef<AudioContext | null>(null);
  // Frequency analyser tapping the TTS output, so the chat can draw a live
  // waveform while the agent speaks. Lives on the playback context; recreated
  // with it, nulled when it closes.
  const analyserRef = useRef<AnalyserNode | null>(null);
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const activeRef = useRef(false);
  const connectingRef = useRef(false);
  const readyResolveRef = useRef<(() => void) | null>(null);
  const readyRejectRef = useRef<((err: Error) => void) | null>(null);
  const stateRef = useRef<VoiceState>('idle');
  // Last time the daemon showed life (any inbound message) or we entered a
  // busy state — drives the stuck-state watchdog below.
  const lastActivityRef = useRef(0);
  // Frame counter for diagnostics
  const frameCountRef = useRef(0);
  // Speak-then-act: a navigation forwarded by the backend AFTER the turn's
  // narration. Held until audio playback fully drains, then fired — so the view
  // switches when the agent stops speaking, not the moment the tool resolves.
  const pendingNavRef = useRef<NavPayload | null>(null);

  const emit = useCallback((event: VoiceEvent) => {
    onEventRef.current?.(event);
  }, []);

  // Apply a deferred navigation by re-broadcasting it as the cross-window Tauri
  // 'app_navigate' event that the main window's useAppNavigate already honors.
  const fireNav = useCallback((payload: NavPayload) => {
    (async () => {
      try {
        if ('__TAURI_INTERNALS__' in window) {
          const { emit: tauriEmit } = await import('@tauri-apps/api/event');
          await tauriEmit('app_navigate', payload);
        }
      } catch (e) {
        console.error('[useVoice] deferred nav emit failed:', e);
      }
    })();
  }, []);

  // Fire the pending nav only once nothing is left to play — the real
  // "narration finished" signal. Safe to call redundantly (idle-guarded).
  const flushNavIfIdle = useCallback(() => {
    if (
      pendingNavRef.current &&
      !playingRef.current &&
      audioQueueRef.current.length === 0 &&
      !pendingAudioRef.current
    ) {
      const nav = pendingNavRef.current;
      pendingNavRef.current = null;
      fireNav(nav);
    }
  }, [fireNav]);

  const setStateAndEmit = useCallback((newState: VoiceState) => {
    stateRef.current = newState;
    setState(newState);
    emit({ type: 'state_change', state: newState });
  }, [emit]);

  // --- Audio playback queue ---
  const playNextChunk = useCallback(() => {
    if (playingRef.current) return;
    const next = audioQueueRef.current.shift();
    if (!next || next.length === 0) {
      if (!pendingAudioRef.current) {
        playbackCtxRef.current?.close().catch(() => {});
        playbackCtxRef.current = null;
        analyserRef.current = null;
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          setStateAndEmit('ready');
          // Narration finished playing — release any deferred navigation now.
          flushNavIfIdle();
        }
      }
      return;
    }
    playingRef.current = true;
    try {
      if (!playbackCtxRef.current || playbackCtxRef.current.state === 'closed') {
        playbackCtxRef.current = new AudioContext({ sampleRate: 24000 });
        // A small FFT — we only need a handful of bars, and low bins carry the
        // voice energy. Smoothing keeps the waveform fluid, not jittery.
        const analyser = playbackCtxRef.current.createAnalyser();
        analyser.fftSize = 64;
        analyser.smoothingTimeConstant = 0.75;
        analyser.connect(playbackCtxRef.current.destination);
        analyserRef.current = analyser;
      }
      const ctx = playbackCtxRef.current;
      const buffer = ctx.createBuffer(1, next.length, 24000);
      buffer.getChannelData(0).set(next);
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      // Route through the analyser (a pass-through) so playback is unchanged
      // but we can read live frequency data off it.
      source.connect(analyserRef.current ?? ctx.destination);
      source.onended = () => {
        playingRef.current = false;
        playNextChunk();
      };
      source.start();
    } catch {
      playingRef.current = false;
      pendingAudioRef.current = false;
    }
  }, [setStateAndEmit, flushNavIfIdle]);

  // --- WebSocket message handler ---
  const handleWsMessage = useCallback((event: MessageEvent) => {
    // Any inbound message proves the daemon is alive — reset the watchdog.
    lastActivityRef.current = Date.now();
    if (typeof event.data === 'string') {
      try {
        const msg = JSON.parse(event.data);
        switch (msg.type) {
          case 'ready':
            setStateAndEmit('ready');
            readyResolveRef.current?.();
            readyResolveRef.current = null;
            readyRejectRef.current = null;
            break;
          case 'transcript':
            // A transcript proves STT is working this session — clear any stale
            // transient error (e.g. a prior "No speech detected") so it doesn't
            // stick in the UI while the conversation is in fact succeeding.
            setError(null);
            setLastTranscript(msg.text ?? '');
            emit({ type: 'transcript', text: msg.text ?? '' });
            break;
          case 'reply_text':
            setLastReply(msg.text ?? '');
            emit({ type: 'reply_text', text: msg.text ?? '' });
            break;
          case 'reply_start':
            setStateAndEmit('playing');
            break;
          case 'reply_end':
            pendingAudioRef.current = false;
            if (!playingRef.current && audioQueueRef.current.length === 0) {
              setStateAndEmit('ready');
            }
            // Covers a reply whose audio already drained before reply_end.
            flushNavIfIdle();
            break;
          case 'navigate':
            // Deferred navigation: hold it behind the audio queue, then fire
            // once playback drains. If nothing is playing it fires immediately.
            pendingNavRef.current = {
              tab: msg.tab,
              tool_type: msg.tool_type,
              panel_type: msg.panel_type,
              section: msg.section,
              state: msg.state,
              reason: msg.reason,
            };
            flushNavIfIdle();
            break;
          case 'error':
            setError(msg.message ?? 'Unknown voice error');
            emit({ type: 'error', error: msg.message ?? 'Unknown voice error' });
            setStateAndEmit('error');
            setTimeout(() => {
              if (wsRef.current?.readyState === WebSocket.OPEN) {
                // Recovering on a live socket: also clear the error string so a
                // transient server error (no-speech, too-short) doesn't linger
                // as a sticky red label over the recovered "Hold to talk" state.
                setError(null);
                setStateAndEmit('ready');
              }
            }, 2000);
            break;
        }
      } catch (e) {
        console.error('[useVoice] message parse error:', e);
      }
    } else if (event.data instanceof ArrayBuffer) {
      const buf = event.data as ArrayBuffer;
      if (buf.byteLength > 0) {
        const samples = new Float32Array(buf);
        pendingAudioRef.current = true;
        setStateAndEmit('playing');
        audioQueueRef.current.push(samples);
        playNextChunk();
        emit({ type: 'reply_audio', audio: { samples, sampleRate: 24000 } });
      }
    }
  }, [setStateAndEmit, emit, playNextChunk, flushNavIfIdle]);

  // Monotonic connect epoch. Each connectSocket call claims a new epoch; a
  // call that gets superseded while awaiting the daemon token (e.g. the
  // session switched and a rebind connect started) aborts instead of resuming
  // and binding a socket to a stale session URL after the newer one.
  const connectEpochRef = useRef(0);

  // --- Core connect ---
  const connectSocket = useCallback(async (): Promise<void> => {
    const epoch = ++connectEpochRef.current;
    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.close();
      wsRef.current = null;
    }

    setStateAndEmit('connecting');
    setError(null);

    let token: string | null = null;
    try {
      token = await getStreamToken();
    } catch (e) {
      console.error('[useVoice] getStreamToken failed:', e);
    }

    // A newer connect started while we awaited the token — it owns the socket
    // now (and carries the current session_id); creating ours would clobber it
    // with a stale-session connection.
    if (epoch !== connectEpochRef.current) {
      throw new Error('connect superseded by a newer connect');
    }

    const base = getApiBaseUrl().replace(/^http/, 'ws');
    const params = new URLSearchParams();
    if (sessionId) params.set('session_id', sessionId);
    if (token) params.set('token', token);
    const url = `${base}/voice?${params}`;

    return new Promise<void>((resolve, reject) => {
      readyResolveRef.current = resolve;
      readyRejectRef.current = reject;

      const ws = new WebSocket(url);
      ws.binaryType = 'arraybuffer';
      wsRef.current = ws;

      ws.onmessage = handleWsMessage;
      ws.onerror = () => {};

      ws.onclose = (ev) => {
        const detail = `code=${ev.code} reason=${ev.reason || '(none)'} wasClean=${ev.wasClean}`;
        console.log(`[useVoice] ws.onclose: ${detail}`);
        wsRef.current = null;

        if (readyRejectRef.current) {
          readyRejectRef.current(new Error(`WebSocket closed: ${detail}`));
          readyResolveRef.current = null;
          readyRejectRef.current = null;
        }

        if (ev.code !== 1000 && ev.code !== 1005) {
          let msg = 'Voice connection lost';
          if (ev.code === 1006) msg = `Connection refused (${token ? 'token sent' : 'NO TOKEN'})`;
          if (ev.reason) msg = ev.reason;
          setError(msg);
          setStateAndEmit('error');

          if (activeRef.current) {
            setTimeout(() => {
              if (activeRef.current && !connectingRef.current) {
                connectingRef.current = true;
                connectSocket()
                  .catch(() => {})
                  .finally(() => { connectingRef.current = false; });
              }
            }, 2000);
          } else {
            setTimeout(() => setStateAndEmit('idle'), 2000);
          }
        } else {
          setStateAndEmit('idle');
        }
      };
    });
  }, [sessionId, setStateAndEmit, handleWsMessage]);

  // Force-clear a wedged voice state. Tears down playback resources, then —
  // if voice is still active — reconnects a FRESH socket (which aborts a hung
  // daemon handler, since the close sets its cancellation flag) so the next
  // push-to-talk reaches a live handler. If inactive, drops to idle. This is
  // the guaranteed "Speaking..." reset the watchdog and session-switch use.
  const forceRecover = useCallback((reason: string) => {
    console.warn(`[useVoice] forceRecover: ${reason}`);
    // Tear down playback so a half-played buffer can't hold the busy state.
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    lastActivityRef.current = Date.now();

    if (activeRef.current) {
      // Even if a connect is already in flight, start a fresh one: the
      // in-flight attempt may be bound to a stale session URL (session-switch
      // recovery) or wedged. connectSocket's epoch guard aborts the stale
      // attempt; its preamble closes any socket it already opened.
      connectingRef.current = true;
      connectSocket()
        .catch(() => {})
        .finally(() => { connectingRef.current = false; });
    } else {
      if (wsRef.current) {
        wsRef.current.onclose = null;
        wsRef.current.close();
        wsRef.current = null;
      }
      setStateAndEmit('idle');
    }
  }, [connectSocket, setStateAndEmit]);

  // Barge-in (#398): stop Henry mid-response so the user can talk. Halts TTS
  // playback immediately (silence is instant — playback is torn down before the
  // socket work) and cancels the in-flight daemon turn by reconnecting a FRESH
  // socket: closing the old one sets the daemon handler's cancellation flag, so
  // it stops synthesizing further sentences instead of streaming audio the user
  // no longer wants. The reconnect leaves voice ACTIVE (mic stays acquired), so
  // it returns to 'ready' — the mic is listening again for the next turn.
  //
  // No-op unless a reply is actually in flight: in 'ready' a Space press means
  // start recording, and there is nothing to interrupt in idle/connecting/error.
  const interrupt = useCallback(() => {
    if (!isInterruptibleState(stateRef.current)) return;
    forceRecover('user barge-in — interrupt reply');
  }, [forceRecover]);

  // Stuck-state watchdog: a daemon that stops emitting terminal events (a hung
  // LLM/TTS stream sends no reply_end/error yet leaves the socket open) would
  // otherwise pin the UI in 'playing'/'processing' forever — locking the mic.
  // Poll for a wedged state and force a recovery so it can never trap the user.
  useEffect(() => {
    const id = setInterval(() => {
      if (
        isVoiceWedged({
          state: stateRef.current,
          isPlaying: playingRef.current,
          queuedChunks: audioQueueRef.current.length,
          msSinceActivity: Date.now() - lastActivityRef.current,
        })
      ) {
        forceRecover('watchdog: busy state wedged with no daemon activity');
      }
    }, 5000);
    return () => clearInterval(id);
  }, [forceRecover]);

  // Session rebinding: the socket bakes `session_id` into its URL at connect
  // time, so a socket that survives a conversation switch keeps routing voice
  // turns into the PREVIOUS session — push-to-talk in 'ready' after a switch
  // was landing utterances in the old chat. On ANY session change while a
  // socket is live (or a connect is in flight), drop whatever is bound to the
  // old session — loudly, never silently misrouting — and reconnect bound to
  // the new session. The mic stays acquired; only the socket is rebuilt. When
  // voice is fully off there is nothing bound to a session and nothing to do.
  const prevSessionRef = useRef(sessionId);
  useEffect(() => {
    if (prevSessionRef.current === sessionId) return;
    prevSessionRef.current = sessionId;
    const s = stateRef.current;
    if (!wsRef.current && !connectingRef.current && s === 'idle') return;

    if (s === 'recording') {
      // Mid-capture: the frames already streamed to the old session's socket.
      // Never send 'stop' (that would run the turn against the old session) —
      // tear the capture down and drop the utterance, visibly.
      console.warn(
        '[useVoice] session changed mid-recording — dropping the in-flight utterance rather than sending it to the previous session',
      );
      processorRef.current?.disconnect();
      processorRef.current = null;
      audioCtxRef.current?.close().catch(() => {});
      audioCtxRef.current = null;
      frameCountRef.current = 0;
    } else if (s === 'processing' || s === 'playing') {
      console.warn(
        "[useVoice] session changed mid-turn — dropping the previous session's in-flight reply",
      );
    }
    forceRecover('session changed — rebinding voice socket to the new session');
  }, [sessionId, forceRecover]);

  // --- Public API ---

  const ensureReady = useCallback(async (): Promise<void> => {
    if (wsRef.current?.readyState === WebSocket.OPEN && stateRef.current === 'ready') {
      return;
    }
    if (connectingRef.current) {
      return new Promise<void>((resolve, reject) => {
        const prev = readyResolveRef.current;
        readyResolveRef.current = () => { prev?.(); resolve(); };
        const prevRej = readyRejectRef.current;
        readyRejectRef.current = (err) => { prevRej?.(err); reject(err); };
      });
    }
    connectingRef.current = true;
    try {
      await connectSocket();
    } finally {
      connectingRef.current = false;
    }
  }, [connectSocket]);

  /** Activate voice: acquire mic + connect socket. */
  const activate = useCallback(async () => {
    activeRef.current = true;

    // Acquire mic ONCE at activation (persistent until deactivate).
    // This makes startRecording synchronous — no getUserMedia race.
    if (!mediaStreamRef.current) {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: { sampleRate, channelCount: 1, echoCancellation: true },
        });
        mediaStreamRef.current = stream;
        console.log('[useVoice] mic acquired');
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : 'Microphone access failed';
        if (message.includes('Permission') || message.includes('NotAllowed')) {
          setError('Mic permission denied. Grant access in System Settings > Privacy > Microphone.');
        } else if (message.includes('NotFound') || message.includes('DevicesNotFound')) {
          setError('No microphone found.');
        } else {
          setError(message);
        }
        setStateAndEmit('error');
        activeRef.current = false;
        return;
      }
    }

    try {
      await ensureReady();
    } catch (e) {
      console.error('[useVoice] activate failed:', e);
    }
  }, [sampleRate, ensureReady, setStateAndEmit]);

  /** Deactivate voice: release mic + disconnect socket. */
  const deactivate = useCallback(() => {
    activeRef.current = false;
    // Stop recording
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    // Release mic
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    // Stop playback
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    // Close socket
    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.close();
      wsRef.current = null;
    }
    setStateAndEmit('idle');
  }, [setStateAndEmit]);

  /** Start recording. SYNCHRONOUS — no async gaps, no press-release race.
   *  Only works when socket is open, state is 'ready', and mic is acquired. */
  const startRecording = useCallback(() => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (stateRef.current !== 'ready') return;

    const stream = mediaStreamRef.current;
    if (!stream) {
      console.warn('[useVoice] startRecording: no mic stream (not activated?)');
      return;
    }

    const audioCtx = new AudioContext({ sampleRate });
    audioCtxRef.current = audioCtx;
    const source = audioCtx.createMediaStreamSource(stream);
    const processor = audioCtx.createScriptProcessor(4096, 1, 1);
    processorRef.current = processor;
    frameCountRef.current = 0;

    ws.send(JSON.stringify({ type: 'start', sample_rate: sampleRate }));
    setStateAndEmit('recording');

    processor.onaudioprocess = (e) => {
      const sock = wsRef.current;
      if (!sock || sock.readyState !== WebSocket.OPEN) return;
      const input = e.inputBuffer.getChannelData(0);
      const buffer = new ArrayBuffer(input.length * 4);
      new Float32Array(buffer).set(input);
      sock.send(buffer);
      frameCountRef.current++;
    };

    source.connect(processor);
    processor.connect(audioCtx.destination);
  }, [sampleRate, setStateAndEmit]);

  /** Stop recording and send audio for processing. */
  const stopRecording = useCallback(() => {
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    // Do NOT stop mediaStream tracks — mic stays alive for next recording
    pendingAudioRef.current = false;

    const frames = frameCountRef.current;
    frameCountRef.current = 0;
    console.log(`[useVoice] stopRecording: sent ${frames} frames (${(frames * 4096 / sampleRate).toFixed(1)}s)`);

    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN && stateRef.current === 'recording') {
      ws.send(JSON.stringify({ type: 'stop' }));
      // Start the watchdog clock fresh as we enter the busy phase.
      lastActivityRef.current = Date.now();
      setStateAndEmit('processing');
    }
  }, [sampleRate, setStateAndEmit]);

  /** Live TTS frequency analyser (present only while the agent is speaking). */
  const getAnalyser = useCallback(() => analyserRef.current, []);

  /** Live mic frequency analyser (present only while hands-free VAD runs). */
  const getMicAnalyser = useCallback(() => micAnalyserRef.current, []);

  // ── Hands-free mode (#19) ──────────────────────────────────────────────────
  // Click the visualizer → the mic is ALWAYS listening and turns are taken by
  // voice-activity detection instead of push-to-talk:
  //   ready     + speech onset          → startRecording()
  //   recording + 900ms trailing silence → stopRecording()  (turn complete)
  //   playing   + sustained loud speech  → interrupt()      (barge-in)
  // The monitor is its own tiny audio graph on the persistent mic stream, so
  // it runs regardless of the recording processor's lifecycle. Thresholds are
  // RMS over ~128ms buffers; echoCancellation on the mic keeps the agent's own
  // TTS from tripping onset, and barge-in demands a higher, sustained level so
  // residual bleed can't cut the agent off.
  const [handsFree, setHandsFreeState] = useState(false);
  const handsFreeRef = useRef(false);
  const vadCtxRef = useRef<AudioContext | null>(null);
  const vadProcRef = useRef<ScriptProcessorNode | null>(null);
  // Mic-side analyser tapped off the VAD graph — lets the conversation orb
  // visualize the USER's voice while listening (the playback analyser only
  // covers the agent's TTS). Lives and dies with the VAD monitor.
  const micAnalyserRef = useRef<AnalyserNode | null>(null);
  const vadLastVoiceRef = useRef(0);
  const vadHeardSpeechRef = useRef(false);
  const vadBargeStreakRef = useRef(0);
  const vadTurnStartRef = useRef(0);

  const VAD_ONSET = 0.015;
  const VAD_KEEPALIVE = 0.010;
  const VAD_BARGE = 0.05;
  const VAD_SILENCE_MS = 900;
  const VAD_MAX_TURN_MS = 45_000;

  const stopVadMonitor = useCallback(() => {
    vadProcRef.current?.disconnect();
    vadProcRef.current = null;
    vadCtxRef.current?.close().catch(() => {});
    vadCtxRef.current = null;
    micAnalyserRef.current = null;
  }, []);

  const startVadMonitor = useCallback(() => {
    const stream = mediaStreamRef.current;
    if (!stream || vadCtxRef.current) return;
    const ctx = new AudioContext({ sampleRate });
    vadCtxRef.current = ctx;
    const source = ctx.createMediaStreamSource(stream);
    // Tap for the orb visualization — same small-FFT/smoothing profile as the
    // playback analyser. An analyser is a pass-through observer: it doesn't
    // alter the VAD's samples.
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 64;
    analyser.smoothingTimeConstant = 0.75;
    source.connect(analyser);
    micAnalyserRef.current = analyser;
    const proc = ctx.createScriptProcessor(2048, 1, 1);
    vadProcRef.current = proc;
    proc.onaudioprocess = (e) => {
      if (!handsFreeRef.current) return;
      const input = e.inputBuffer.getChannelData(0);
      let sum = 0;
      for (let i = 0; i < input.length; i++) sum += input[i] * input[i];
      const rms = Math.sqrt(sum / input.length);
      const now = Date.now();
      const s = stateRef.current;

      if (s === 'ready') {
        vadBargeStreakRef.current = 0;
        if (rms > VAD_ONSET) {
          vadHeardSpeechRef.current = true;
          vadLastVoiceRef.current = now;
          vadTurnStartRef.current = now;
          startRecordingRef.current?.();
        }
      } else if (s === 'recording') {
        if (rms > VAD_KEEPALIVE) {
          vadHeardSpeechRef.current = true;
          vadLastVoiceRef.current = now;
        }
        const spoke = vadHeardSpeechRef.current;
        const silentFor = now - vadLastVoiceRef.current;
        const turnFor = now - vadTurnStartRef.current;
        if ((spoke && silentFor > VAD_SILENCE_MS) || turnFor > VAD_MAX_TURN_MS) {
          vadHeardSpeechRef.current = false;
          stopRecordingRef.current?.();
        }
      } else if (s === 'playing' || s === 'processing') {
        // Barge-in: demand a sustained loud signal (2 consecutive ~128ms
        // buffers over the high bar) so TTS bleed can't self-interrupt.
        if (rms > VAD_BARGE) {
          vadBargeStreakRef.current += 1;
          if (vadBargeStreakRef.current >= 2 && s === 'playing') {
            vadBargeStreakRef.current = 0;
            vadHeardSpeechRef.current = false;
            interruptRef.current?.();
          }
        } else {
          vadBargeStreakRef.current = 0;
        }
      }
    };
    source.connect(proc);
    proc.connect(ctx.destination);
  }, [sampleRate]);

  /** Enter/exit hands-free. Entering activates voice (mic + socket) if needed. */
  const setHandsFree = useCallback(async (on: boolean) => {
    handsFreeRef.current = on;
    setHandsFreeState(on);
    if (on) {
      if (!activeRef.current) await activate();
      startVadMonitor();
    } else {
      stopVadMonitor();
      // A VAD-started recording without its monitor would never end — close
      // the turn normally so anything already said still gets processed.
      if (stateRef.current === 'recording') stopRecordingRef.current?.();
    }
  }, [activate, startVadMonitor, stopVadMonitor]);

  // Late-bound refs so the monitor callback (created once) always calls the
  // freshest versions without re-wiring the audio graph.
  const startRecordingRef = useRef<(() => void) | null>(null);
  const stopRecordingRef = useRef<(() => void) | null>(null);
  const interruptRef = useRef<(() => void) | null>(null);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      activeRef.current = false;
      processorRef.current?.disconnect();
      audioCtxRef.current?.close().catch(() => {});
      mediaStreamRef.current?.getTracks().forEach(t => t.stop());
      wsRef.current?.close();
    };
  }, []);

  // Keep the VAD monitor's late-bound refs fresh + tear it down on unmount.
  startRecordingRef.current = startRecording;
  stopRecordingRef.current = stopRecording;
  interruptRef.current = interrupt;
  useEffect(() => stopVadMonitor, [stopVadMonitor]);

  return {
    state,
    lastTranscript,
    lastReply,
    error,
    activate,
    deactivate,
    startRecording,
    stopRecording,
    interrupt,
    getAnalyser,
    getMicAnalyser,
    handsFree,
    setHandsFree,
  };
}
