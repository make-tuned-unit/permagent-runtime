/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Backend-agnostic: talks to the /voice WebSocket with PCM f32le frames.
 * Knows nothing about sherpa-onnx, Kokoro, or Moonshine specifically.
 * When the shipping backend swaps to ort+misaki-rs, the frontend stays unchanged.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { getApiBaseUrl, loadDaemonToken } from '../lib/api';

export type VoiceState =
  | 'idle'          // No voice activity
  | 'connecting'    // WebSocket connecting
  | 'ready'         // WebSocket connected, waiting for push-to-talk
  | 'recording'     // Push-to-talk held, capturing audio
  | 'processing'    // Audio sent, waiting for STT + reply + TTS
  | 'playing'       // Playing TTS response
  | 'error'         // Recoverable error (mic denied, no models, etc.)
  | 'unavailable';  // Voice not supported (no mic, not Tauri, etc.)

export interface VoiceEvent {
  type: 'transcript' | 'reply_text' | 'reply_audio' | 'error' | 'state_change';
  text?: string;
  audio?: { samples: Float32Array; sampleRate: number };
  state?: VoiceState;
  error?: string;
}

interface UseVoiceOptions {
  sessionId?: string;
  sampleRate?: number;
  onEvent?: (event: VoiceEvent) => void;
}

// Token is loaded via api.ts's loadDaemonToken (proven, cached, loaded at app init).

export function useVoice(options: UseVoiceOptions = {}) {
  const { sessionId, sampleRate = 16000, onEvent } = options;
  const [state, setState] = useState<VoiceState>('idle');
  const [lastTranscript, setLastTranscript] = useState('');
  const [lastReply, setLastReply] = useState('');
  const [error, setError] = useState<string | null>(null);
  // Debug: full error + stack trace, visible in the UI for diagnosis.
  // Remove once the runtime crash is resolved.
  const [debugError, setDebugError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const pendingAudioRef = useRef(false);
  const audioQueueRef = useRef<Float32Array[]>([]);
  const playingRef = useRef(false);
  const playbackCtxRef = useRef<AudioContext | null>(null);
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  // Capture any error with full stack for debugging
  const captureError = useCallback((label: string, err: unknown) => {
    const msg = err instanceof Error
      ? `${label}: ${err.message}\n${err.stack ?? '(no stack)'}`
      : `${label}: ${String(err)}`;
    console.error('[useVoice]', msg);
    setDebugError(msg);
  }, []);

  const emit = useCallback((event: VoiceEvent) => {
    onEventRef.current?.(event);
  }, []);

  const setStateAndEmit = useCallback((newState: VoiceState) => {
    setState(newState);
    emit({ type: 'state_change', state: newState });
  }, [emit]);

  // Connect to /voice WebSocket
  const connect = useCallback(async () => {
    if (wsRef.current?.readyState === WebSocket.OPEN) return;

    setStateAndEmit('connecting');
    setError(null);
    setDebugError(null);

    // Use the api module's proven cached token (loaded at app init).
    let token: string | null = null;
    try {
      token = await loadDaemonToken();
    } catch (e) {
      captureError('loadDaemonToken', e);
    }

    const base = getApiBaseUrl().replace(/^http/, 'ws');
    const params = new URLSearchParams();
    if (sessionId) params.set('session_id', sessionId);
    if (token) params.set('token', token);
    const url = `${base}/voice?${params}`;

    // Diagnostic: log connection details (full URL for debugging)
    console.log(`[useVoice] connecting: url=${url}, hasToken=${!!token}, tokenLen=${token?.length ?? 0}, base=${base}`);

    const ws = new WebSocket(url);
    // Force binary frames to arrive as ArrayBuffer (not Blob).
    // WKWebView in Tauri may not support Blob binary frames reliably.
    ws.binaryType = 'arraybuffer';
    wsRef.current = ws;

    ws.onopen = () => {
      // Wait for 'ready' message from server
    };

    ws.onmessage = (event) => {
      if (typeof event.data === 'string') {
        try {
          const msg = JSON.parse(event.data);
          switch (msg.type) {
            case 'ready':
              setStateAndEmit('ready');
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
              // Mark that no more audio chunks are coming.
              pendingAudioRef.current = false;
              // If nothing is playing and queue is empty, go to ready.
              if (!playingRef.current && audioQueueRef.current.length === 0) {
                setStateAndEmit('ready');
              }
              // Otherwise playNextChunk's onended will handle the transition.
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
        } catch (parseErr) {
          captureError('ws.onmessage parse', parseErr);
        }
      } else if (event.data instanceof ArrayBuffer) {
        // Binary: TTS audio chunk (f32le PCM) — queue for sequential playback
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
    };

    ws.onerror = () => {
      // WebSocket error events are opaque — the real reason comes from onclose.
      // Don't set error here; let onclose handle it with the close code/reason.
    };

    ws.onclose = (ev) => {
      const detail = `code=${ev.code} reason=${ev.reason || '(none)'} wasClean=${ev.wasClean} hasToken=${!!token}`;
      console.log(`[useVoice] ws.onclose: ${detail}`);
      if (ev.code !== 1000 && ev.code !== 1005) {
        // Abnormal close — surface the reason
        let msg = 'Voice connection failed';
        if (ev.code === 1006) msg = `Connection refused (${token ? 'token sent' : 'NO TOKEN'})`;
        if (ev.reason) msg = ev.reason;
        setError(msg);
        setDebugError(`ws.onclose: ${detail}`);
        setStateAndEmit('error');
        setTimeout(() => setStateAndEmit('idle'), 3000);
      } else if (state !== 'idle') {
        setStateAndEmit('idle');
      }
    };
  }, [sessionId, setStateAndEmit, emit, state]);

  // Disconnect — uses a ref to avoid stale closure on stopRecording
  // (stopRecording is defined below but the ref is updated after each render).
  const stopRecordingRef = useRef<() => void>(() => {});

  const disconnect = useCallback(() => {
    stopRecordingRef.current();
    wsRef.current?.close();
    wsRef.current = null;
    setStateAndEmit('idle');
  }, [setStateAndEmit]);

  // Start recording (push-to-talk pressed)
  const startRecording = useCallback(async () => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (state !== 'ready') return;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { sampleRate, channelCount: 1, echoCancellation: true },
      });

      // Re-check after await — WS may have closed during mic permission prompt.
      if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) {
        stream.getTracks().forEach(t => t.stop());
        return;
      }

      mediaStreamRef.current = stream;

      const audioCtx = new AudioContext({ sampleRate });
      audioCtxRef.current = audioCtx;
      const source = audioCtx.createMediaStreamSource(stream);

      const processor = audioCtx.createScriptProcessor(4096, 1, 1);
      processorRef.current = processor;

      // Tell server we're starting (safe — re-checked above)
      wsRef.current.send(JSON.stringify({ type: 'start', sample_rate: sampleRate }));
      setStateAndEmit('recording');

      processor.onaudioprocess = (e) => {
        const sock = wsRef.current;
        if (!sock || sock.readyState !== WebSocket.OPEN) return;
        const input = e.inputBuffer.getChannelData(0);
        const buffer = new ArrayBuffer(input.length * 4);
        const view = new Float32Array(buffer);
        view.set(input);
        sock.send(buffer);
      };

      source.connect(processor);
      processor.connect(audioCtx.destination);
    } catch (err: unknown) {
      captureError('startRecording', err);
      const message = err instanceof Error ? err.message : 'Microphone access failed';
      if (message.includes('Permission') || message.includes('NotAllowed')) {
        setError('Microphone permission denied. Grant access in System Settings > Privacy & Security > Microphone.');
      } else if (message.includes('NotFound') || message.includes('DevicesNotFound')) {
        setError('No microphone found. Connect a mic and try again.');
      } else {
        setError(message);
      }
      setStateAndEmit('error');
      setTimeout(() => {
        if (wsRef.current?.readyState === WebSocket.OPEN) setStateAndEmit('ready');
      }, 3000);
    }
  }, [state, sampleRate, setStateAndEmit]);

  // Stop recording (push-to-talk released)
  const stopRecording = useCallback(() => {
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    pendingAudioRef.current = false;

    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN && state === 'recording') {
      ws.send(JSON.stringify({ type: 'stop' }));
      setStateAndEmit('processing');
    }
  }, [state, setStateAndEmit]);
  stopRecordingRef.current = stopRecording;

  // Queue-based audio playback: plays chunks in order as they arrive.
  const playNextChunk = useCallback(() => {
    if (playingRef.current) return; // already playing
    const next = audioQueueRef.current.shift();
    if (!next || next.length === 0) {
      // Queue empty — check if more chunks are coming
      if (!pendingAudioRef.current) {
        // All done
        playbackCtxRef.current?.close().catch(() => {});
        playbackCtxRef.current = null;
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          setStateAndEmit('ready');
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
        playNextChunk(); // play next in queue
      };
      source.start();
    } catch (err) {
      captureError('playNextChunk', err);
      playingRef.current = false;
      pendingAudioRef.current = false;
    }
  }, [setStateAndEmit, captureError]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      stopRecording();
      wsRef.current?.close();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return {
    state,
    lastTranscript,
    lastReply,
    error,
    debugError,
    connect,
    disconnect,
    startRecording,
    stopRecording,
  };
}
