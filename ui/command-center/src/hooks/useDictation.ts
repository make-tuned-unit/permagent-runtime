/**
 * useDictation — record a short mic clip and get back transcribed text.
 *
 * Captures raw PCM via the Web Audio API (NOT MediaRecorder) and encodes it as
 * WAV in-browser, then POSTs to `/api/dictation/transcribe` for local Whisper
 * transcription. The Web-Audio-to-WAV path is deliberate: MediaRecorder emits
 * Opus/WebM, which the server-side decoder (symphonia) does not reliably
 * decode, whereas WAV is universally decodable. The transcript is handed back
 * through `onText` so a caller (e.g. the Notes composer) can append it.
 *
 * A 503 from the endpoint means dictation isn't set up on this install (no
 * local model configured); it surfaces as `error` for the caller to show,
 * rather than throwing the whole surface into a broken state.
 */
import { useCallback, useRef, useState } from 'react';
import { api } from '../lib/api';

export type DictationState = 'idle' | 'recording' | 'transcribing' | 'error';

export function useDictation(onText: (text: string) => void) {
  const [state, setState] = useState<DictationState>('idle');
  const [error, setError] = useState<string | null>(null);

  const ctxRef = useRef<AudioContext | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const procRef = useRef<ScriptProcessorNode | null>(null);
  const chunksRef = useRef<Float32Array[]>([]);
  const rateRef = useRef<number>(16000);

  const teardown = useCallback(() => {
    try { procRef.current?.disconnect(); } catch { /* already gone */ }
    streamRef.current?.getTracks().forEach(t => t.stop());
    const ctx = ctxRef.current;
    if (ctx && ctx.state !== 'closed') void ctx.close();
    procRef.current = null;
    ctxRef.current = null;
    streamRef.current = null;
  }, []);

  const start = useCallback(async () => {
    setError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;
      const ctx = new AudioContext();
      ctxRef.current = ctx;
      rateRef.current = ctx.sampleRate;
      const source = ctx.createMediaStreamSource(stream);
      // ScriptProcessorNode is deprecated but universally available in the
      // desktop webview and needs no worklet-module loading — the right
      // trade-off for a short push-to-dictate capture.
      const proc = ctx.createScriptProcessor(4096, 1, 1);
      procRef.current = proc;
      chunksRef.current = [];
      proc.onaudioprocess = (e: AudioProcessingEvent) => {
        chunksRef.current.push(new Float32Array(e.inputBuffer.getChannelData(0)));
      };
      source.connect(proc);
      proc.connect(ctx.destination);
      setState('recording');
    } catch {
      setError('Microphone access was blocked');
      setState('error');
      teardown();
    }
  }, [teardown]);

  const stop = useCallback(async () => {
    const chunks = chunksRef.current;
    const rate = rateRef.current;
    chunksRef.current = [];
    teardown();

    const total = chunks.reduce((n, c) => n + c.length, 0);
    if (total === 0) { setState('idle'); return; }

    const merged = new Float32Array(total);
    let off = 0;
    for (const c of chunks) { merged.set(c, off); off += c.length; }

    setState('transcribing');
    try {
      const wav = encodeWav(merged, rate);
      const { text } = await api.transcribeAudio(new Blob([wav], { type: 'audio/wav' }));
      const trimmed = text.trim();
      if (trimmed) onText(trimmed);
      setState('idle');
    } catch (e) {
      setError((e as Error).message || 'Transcription failed');
      setState('error');
    }
  }, [teardown, onText]);

  /** Toggle: start when idle, stop+transcribe when recording. */
  const toggle = useCallback(() => {
    if (state === 'recording') void stop();
    else if (state === 'idle' || state === 'error') void start();
  }, [state, start, stop]);

  return { state, error, toggle };
}

/** Encode mono Float32 PCM `[-1, 1]` as a 16-bit PCM WAV. */
function encodeWav(samples: Float32Array, sampleRate: number): ArrayBuffer {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const writeStr = (off: number, s: string) => {
    for (let i = 0; i < s.length; i++) view.setUint8(off + i, s.charCodeAt(i));
  };
  writeStr(0, 'RIFF');
  view.setUint32(4, 36 + samples.length * 2, true);
  writeStr(8, 'WAVE');
  writeStr(12, 'fmt ');
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true); // byte rate
  view.setUint16(32, 2, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  writeStr(36, 'data');
  view.setUint32(40, samples.length * 2, true);
  let off = 44;
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(off, s < 0 ? s * 0x8000 : s * 0x7fff, true);
    off += 2;
  }
  return buffer;
}
