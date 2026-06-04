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

  const wsRef = useRef<WebSocket | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

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

    const token = await getDaemonToken();
    const base = getApiBaseUrl().replace(/^http/, 'ws');
    const params = new URLSearchParams();
    if (sessionId) params.set('session_id', sessionId);
    if (token) params.set('token', token);
    const url = `${base}/voice?${params}`;

    const ws = new WebSocket(url);
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
              setLastTranscript(msg.text);
              emit({ type: 'transcript', text: msg.text });
              break;
            case 'reply_text':
              setLastReply(msg.text);
              emit({ type: 'reply_text', text: msg.text });
              break;
            case 'reply_start':
              setStateAndEmit('playing');
              break;
            case 'reply_end':
              // Audio playback handled when binary data arrives
              setStateAndEmit('ready');
              break;
            case 'error':
              setError(msg.message);
              emit({ type: 'error', error: msg.message });
              setStateAndEmit('error');
              // Auto-recover to ready after a moment
              setTimeout(() => {
                if (wsRef.current?.readyState === WebSocket.OPEN) {
                  setStateAndEmit('ready');
                }
              }, 2000);
              break;
          }
        } catch {
          // Ignore parse errors
        }
      } else if (event.data instanceof Blob) {
        // Binary: TTS audio (f32le PCM)
        event.data.arrayBuffer().then((buf) => {
          const samples = new Float32Array(buf);
          const sr = 24000; // Kokoro default, also sent in reply_end
          playAudio(samples, sr);
          emit({ type: 'reply_audio', audio: { samples, sampleRate: sr } });
        });
      }
    };

    ws.onerror = () => {
      setError('WebSocket connection failed');
      setStateAndEmit('error');
    };

    ws.onclose = () => {
      if (state !== 'idle') {
        setStateAndEmit('idle');
      }
    };
  }, [sessionId, setStateAndEmit, emit, state]);

  // Disconnect
  const disconnect = useCallback(() => {
    stopRecording();
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
    audioCtxRef.current?.close();
    audioCtxRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;

    if (wsRef.current?.readyState === WebSocket.OPEN && state === 'recording') {
      wsRef.current.send(JSON.stringify({ type: 'stop' }));
      setStateAndEmit('processing');
    }
  }, [state, setStateAndEmit]);

  // Play TTS audio
  const playAudio = useCallback((samples: Float32Array, sr: number) => {
    const ctx = new AudioContext({ sampleRate: sr });
    const buffer = ctx.createBuffer(1, samples.length, sr);
    buffer.getChannelData(0).set(samples);
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    source.onended = () => {
      ctx.close();
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        setStateAndEmit('ready');
      }
    };
    source.start();
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
    connect,
    disconnect,
    startRecording,
    stopRecording,
  };
}
