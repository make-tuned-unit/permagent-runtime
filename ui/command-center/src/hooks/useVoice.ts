/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Connection model: persistent socket, opened once and kept alive.
 * State machine: idle → connecting → ready → recording → processing → playing → ready
 * Auto-reconnects on abnormal close. No .send() on non-OPEN sockets, ever.
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
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const pendingAudioRef = useRef(false);
  const audioQueueRef = useRef<Float32Array[]>([]);
  const playingRef = useRef(false);
  const playbackCtxRef = useRef<AudioContext | null>(null);
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  // Track the intended active state (user wants voice on)
  const activeRef = useRef(false);
  // Prevent concurrent connect attempts
  const connectingRef = useRef(false);
  // Resolve/reject for ensureReady waiters
  const readyResolveRef = useRef<(() => void) | null>(null);
  const readyRejectRef = useRef<((err: Error) => void) | null>(null);
  // Stable ref for current state (avoids stale closures)
  const stateRef = useRef<VoiceState>('idle');

  const emit = useCallback((event: VoiceEvent) => {
    onEventRef.current?.(event);
  }, []);

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
  }, [setStateAndEmit]);

  // --- WebSocket message handler (stable, uses refs) ---
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
  }, [setStateAndEmit, emit, playNextChunk]);

  // --- Core connect: creates a WebSocket, returns promise that resolves on ready ---
  const connectSocket = useCallback(async (): Promise<void> => {
    // Clean up any existing socket
    if (wsRef.current) {
      wsRef.current.onclose = null; // prevent reconnect loop
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
      ws.onerror = () => {}; // real info comes from onclose

      ws.onclose = (ev) => {
        const detail = `code=${ev.code} reason=${ev.reason || '(none)'} wasClean=${ev.wasClean}`;
        console.log(`[useVoice] ws.onclose: ${detail}`);
        wsRef.current = null;

        // Reject any pending ensureReady waiter
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

          // Auto-reconnect if the user still wants voice active
          if (activeRef.current) {
            setTimeout(() => {
              if (activeRef.current && !connectingRef.current) {
                console.log('[useVoice] auto-reconnecting...');
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

  /** Ensure the socket is open and server-ready. Connects if needed. */
  const ensureReady = useCallback(async (): Promise<void> => {
    if (wsRef.current?.readyState === WebSocket.OPEN && stateRef.current === 'ready') {
      return; // already ready
    }
    if (connectingRef.current) {
      // Already connecting — wait for it
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

  /** Activate voice: connect and stay connected. */
  const activate = useCallback(async () => {
    activeRef.current = true;
    try {
      await ensureReady();
    } catch (e) {
      console.error('[useVoice] activate failed:', e);
    }
  }, [ensureReady]);

  /** Deactivate voice: disconnect and go idle. */
  const deactivate = useCallback(() => {
    activeRef.current = false;
    // Stop any recording
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    // Stop playback
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
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

  /** Start recording. Auto-connects if needed. */
  const startRecording = useCallback(async () => {
    // Ensure connection is ready before recording
    try {
      await ensureReady();
    } catch {
      return; // connection failed, error already set
    }

    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (stateRef.current !== 'ready') return;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { sampleRate, channelCount: 1, echoCancellation: true },
      });

      // Re-check after await — socket may have closed during mic prompt
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

      wsRef.current.send(JSON.stringify({ type: 'start', sample_rate: sampleRate }));
      setStateAndEmit('recording');

      processor.onaudioprocess = (e) => {
        const sock = wsRef.current;
        if (!sock || sock.readyState !== WebSocket.OPEN) return;
        const input = e.inputBuffer.getChannelData(0);
        const buffer = new ArrayBuffer(input.length * 4);
        new Float32Array(buffer).set(input);
        sock.send(buffer);
      };

      source.connect(processor);
      processor.connect(audioCtx.destination);
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
      setTimeout(() => {
        if (wsRef.current?.readyState === WebSocket.OPEN) setStateAndEmit('ready');
      }, 3000);
    }
  }, [sampleRate, setStateAndEmit, ensureReady]);

  /** Stop recording and send audio for processing. */
  const stopRecording = useCallback(() => {
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    pendingAudioRef.current = false;

    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN && stateRef.current === 'recording') {
      ws.send(JSON.stringify({ type: 'stop' }));
      setStateAndEmit('processing');
    }
  }, [setStateAndEmit]);

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
