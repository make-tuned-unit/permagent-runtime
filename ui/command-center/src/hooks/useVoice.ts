/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Backend-agnostic: talks to the /voice WebSocket with PCM f32le frames.
 * Knows nothing about sherpa-onnx, Kokoro, or Moonshine specifically.
 * When the shipping backend swaps to ort+misaki-rs, the frontend stays unchanged.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { getApiBaseUrl } from '../lib/api';

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

async function getDaemonToken(): Promise<string | null> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<string>('get_daemon_token');
  } catch {
    return null;
  }
}

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

    let token: string | null = null;
    try {
      token = await getDaemonToken();
    } catch (e) {
      captureError('getDaemonToken', e);
    }

    const base = getApiBaseUrl().replace(/^http/, 'ws');
    const params = new URLSearchParams();
    if (sessionId) params.set('session_id', sessionId);
    if (token) params.set('token', token);
    const url = `${base}/voice?${params}`;

    // Diagnostic: log connection details
    console.log(`[useVoice] connecting: url=${url.replace(/token=[^&]+/, 'token=***')}, hasToken=${!!token}`);

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
              // State transition handled by playAudio's onended callback.
              // If no audio was received, fall back to ready here.
              if (!pendingAudioRef.current) {
                setStateAndEmit('ready');
              }
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
        // Binary: TTS audio (f32le PCM)
        const buf = event.data as ArrayBuffer;
        if (buf.byteLength > 0) {
          const samples = new Float32Array(buf);
          pendingAudioRef.current = true;
          playAudio(samples, 24000);
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
    if (wsRef.current?.readyState !== WebSocket.OPEN) return;
    if (state !== 'ready') return;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { sampleRate, channelCount: 1, echoCancellation: true },
      });
      mediaStreamRef.current = stream;

      const audioCtx = new AudioContext({ sampleRate });
      audioCtxRef.current = audioCtx;
      const source = audioCtx.createMediaStreamSource(stream);

      // Use ScriptProcessorNode for PCM access (deprecated but reliable in WKWebView)
      const processor = audioCtx.createScriptProcessor(4096, 1, 1);
      processorRef.current = processor;

      // Tell server we're starting
      wsRef.current!.send(JSON.stringify({ type: 'start', sample_rate: sampleRate }));
      setStateAndEmit('recording');

      processor.onaudioprocess = (e) => {
        if (wsRef.current?.readyState !== WebSocket.OPEN) return;
        const input = e.inputBuffer.getChannelData(0);
        // Send as f32le binary
        const buffer = new ArrayBuffer(input.length * 4);
        const view = new Float32Array(buffer);
        view.set(input);
        wsRef.current!.send(buffer);
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

    if (wsRef.current?.readyState === WebSocket.OPEN && state === 'recording') {
      wsRef.current.send(JSON.stringify({ type: 'stop' }));
      setStateAndEmit('processing');
    }
  }, [state, setStateAndEmit]);
  stopRecordingRef.current = stopRecording;

  // Play TTS audio
  const playAudio = useCallback((samples: Float32Array, sr: number) => {
    if (!samples || samples.length === 0) {
      pendingAudioRef.current = false;
      return;
    }
    try {
      const ctx = new AudioContext({ sampleRate: sr });
      const buffer = ctx.createBuffer(1, samples.length, sr);
      buffer.getChannelData(0).set(samples);
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      source.connect(ctx.destination);
      source.onended = () => {
        pendingAudioRef.current = false;
        ctx.close().catch(() => {});
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          setStateAndEmit('ready');
        }
      };
      source.start();
    } catch (err) {
      captureError('playAudio', err);
      pendingAudioRef.current = false;
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        setStateAndEmit('ready');
      }
    }
  }, [setStateAndEmit]);

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
