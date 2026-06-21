/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Connection model: persistent socket, opened once and kept alive.
 * Mic: acquired at activation, released at deactivation. Recording start/stop
 *       is synchronous (no getUserMedia race on quick press-and-release).
 * State machine: idle → connecting → ready → recording → processing → playing → ready
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { getApiBaseUrl, loadDaemonToken } from '../lib/api';

export type VoiceState =
  | 'idle'          // Voice off — no socket
  | 'connecting'    // WebSocket connecting, waiting for server ready
  | 'ready'         // Socket open + server ready, waiting for push-to-talk
  | 'recording'     // Push-to-talk held, capturing audio
  | 'processing'    // Audio sent, waiting for STT + reply + TTS
  | 'playing'       // Playing TTS response
  | 'error';        // Recoverable error (auto-recovers to ready/idle)

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
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const activeRef = useRef(false);
  const connectingRef = useRef(false);
  const readyResolveRef = useRef<(() => void) | null>(null);
  const readyRejectRef = useRef<((err: Error) => void) | null>(null);
  const stateRef = useRef<VoiceState>('idle');
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
      }
      const ctx = playbackCtxRef.current;
      const buffer = ctx.createBuffer(1, next.length, 24000);
      buffer.getChannelData(0).set(next);
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      source.connect(ctx.destination);
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

  // --- Core connect ---
  const connectSocket = useCallback(async (): Promise<void> => {
    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.close();
      wsRef.current = null;
    }

    setStateAndEmit('connecting');
    setError(null);

    let token: string | null = null;
    try {
      token = await loadDaemonToken();
    } catch (e) {
      console.error('[useVoice] loadDaemonToken failed:', e);
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
      setStateAndEmit('processing');
    }
  }, [sampleRate, setStateAndEmit]);

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

  return {
    state,
    lastTranscript,
    lastReply,
    error,
    activate,
    deactivate,
    startRecording,
    stopRecording,
  };
}
