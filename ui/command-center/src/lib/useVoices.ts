import { useState, useEffect, useCallback, useRef } from 'react';
import {
  api,
  synthesizeVoice,
  type VoiceInfo,
  type VoiceModelStatus,
} from './api';

/** Fixed line auditioned when previewing a voice. */
export const VOICE_SAMPLE_LINE =
  "Hi — I'm here to help you think things through.";

export interface UseVoices {
  voices: VoiceInfo[];
  status: VoiceModelStatus | null;
  /** Assets present AND a live TTS provider is loaded (synthesis works now). */
  ready: boolean;
  loading: boolean;
  /** 0–100 while a download is in flight. */
  downloadPercent: number;
  downloadError: string | null;
  startDownload: () => Promise<void>;
  reload: () => Promise<void>;
}

/**
 * Loads the picker roster + voice-asset status, and drives the on-demand
 * download (polling progress, then hot-reloading the roster when the daemon
 * flips `tts_loaded` — no app restart). Shared by the settings picker and the
 * first-run wizard.
 */
export function useVoices(): UseVoices {
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [status, setStatus] = useState<VoiceModelStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [downloadPercent, setDownloadPercent] = useState(0);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const reload = useCallback(async () => {
    try {
      const [st, vs] = await Promise.all([
        api.getVoiceModelStatus(),
        api.getVoices(),
      ]);
      setStatus(st);
      setVoices(vs);
    } catch {
      // Daemon down / voice routes unavailable — leave empty; UI degrades.
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    reload();
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [reload]);

  const startDownload = useCallback(async () => {
    setDownloadError(null);
    setDownloadPercent(0);
    try {
      await api.downloadVoiceModels();
    } catch (e) {
      setDownloadError(e instanceof Error ? e.message : 'Download failed to start');
      return;
    }
    if (pollRef.current) clearInterval(pollRef.current);
    pollRef.current = setInterval(async () => {
      try {
        const p = await api.getVoiceDownloadProgress();
        setDownloadPercent(Math.round(p.progress_percent));
        if (p.status === 'completed') {
          if (pollRef.current) clearInterval(pollRef.current);
          // Give the daemon a beat to hot-swap the provider in, then reload.
          setTimeout(reload, 800);
        } else if (p.status === 'failed' || p.status === 'cancelled') {
          if (pollRef.current) clearInterval(pollRef.current);
          setDownloadError(p.error ?? `Download ${p.status}`);
        }
      } catch {
        /* progress not yet available — keep polling */
      }
    }, 1000);
  }, [reload]);

  const ready = !!status?.models_present && !!status?.tts_loaded;
  return {
    voices,
    status,
    ready,
    loading,
    downloadPercent,
    downloadError,
    startDownload,
    reload,
  };
}

export interface UseVoicePreview {
  /** Synthesize the sample line in `voiceId` and play it. */
  preview: (voiceId: string | null | undefined, text?: string) => Promise<void>;
  /** The voice id currently playing (for a per-row spinner), or null. */
  playingId: string | null;
  error: string | null;
}

/**
 * Audition playback for the picker: synthesize → decode → play, one at a time.
 *
 * Plays through Web Audio (AudioContext + decodeAudioData), mirroring the live
 * voice path in useVoice.ts. WKWebView rejects `new Audio(blobUrl).play()` with
 * "operation is not supported" — it has no HTMLMediaElement backend for blob
 * URLs — so the picker decodes the WAV itself and routes it through the same
 * AudioContext primitive the live reply stream already uses.
 */
export function useVoicePreview(): UseVoicePreview {
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ctxRef = useRef<AudioContext | null>(null);
  const sourceRef = useRef<AudioBufferSourceNode | null>(null);
  // Bumped on every preview() so a slow synth/decode that finishes after a
  // newer click is discarded instead of double-playing.
  const genRef = useRef(0);

  const stop = useCallback(() => {
    if (sourceRef.current) {
      sourceRef.current.onended = null;
      try {
        sourceRef.current.stop();
      } catch {
        /* already stopped */
      }
      sourceRef.current.disconnect();
      sourceRef.current = null;
    }
  }, []);

  useEffect(() => () => {
    genRef.current++;
    stop();
    ctxRef.current?.close().catch(() => {});
    ctxRef.current = null;
  }, [stop]);

  const preview = useCallback(async (voiceId: string | null | undefined, text?: string) => {
    setError(null);
    // Stop anything currently playing and claim this generation.
    stop();
    const gen = ++genRef.current;
    const key = voiceId ?? '__default__';
    setPlayingId(key);
    try {
      const blob = await synthesizeVoice(text ?? VOICE_SAMPLE_LINE, voiceId ?? null);
      const bytes = await blob.arrayBuffer();
      if (!ctxRef.current || ctxRef.current.state === 'closed') {
        ctxRef.current = new AudioContext();
      }
      const ctx = ctxRef.current;
      await ctx.resume().catch(() => {});
      const buffer = await ctx.decodeAudioData(bytes);
      // A newer preview started (or we unmounted) while awaiting synth/decode.
      if (gen !== genRef.current) return;
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      source.connect(ctx.destination);
      source.onended = () => {
        if (sourceRef.current === source) {
          source.disconnect();
          sourceRef.current = null;
          setPlayingId(p => (p === key ? null : p));
        }
      };
      sourceRef.current = source;
      source.start();
    } catch (e) {
      if (gen !== genRef.current) return;
      setError(e instanceof Error ? e.message : 'Preview failed');
      setPlayingId(null);
    }
  }, [stop]);

  return { preview, playingId, error };
}
