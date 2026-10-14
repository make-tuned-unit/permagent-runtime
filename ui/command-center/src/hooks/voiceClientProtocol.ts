/**
 * Pure pieces of the desktop/World voice client's protocol and guards, kept
 * out of useVoice.ts so they can be unit-tested without a socket or audio.
 * The wire contract is documented in wq-reports/voice/L4-receipt-20261014.md.
 */
import { createPeakHold } from '../lib/voicePlaybackClock';

export type VoiceSurface = 'orb' | 'world';

/** "world" when voice runs from the World view, "orb" for the chat-pane voice view. */
export function deriveVoiceSurface(s: { worldConversationActive: boolean; activeTool: string | null }): VoiceSurface {
  return s.worldConversationActive || s.activeTool === 'world' ? 'world' : 'orb';
}

export type VoiceTrigger = 'user_tap' | 'vad' | 'wake' | 'barge' | 'unknown';

export interface HelloBuild {
  appVersion: string | null;
  buildNumber: string | null;
  gitSha: string | null;
}

export const CLIENT_CAPABILITIES = [
  'hello_v1',
  'turn_telemetry_v2',
  'audio_segment_v1',
  'playback_ack_v1',
  'interrupt_v1',
] as const;

export function buildHello(opts: {
  surface: VoiceSurface;
  build: HelloBuild;
  wireSampleRate: number;
  /** Bumps whenever the hello is re-sent (surface change). */
  revision: number;
}): Record<string, unknown> {
  return {
    type: 'hello',
    protocol_version: 1,
    client: 'desktop_voice',
    surface: opts.surface,
    app_version: opts.build.appVersion,
    build_number: opts.build.buildNumber,
    git_sha: opts.build.gitSha,
    git_dirty: null,
    capabilities: [...CLIENT_CAPABILITIES],
    audio_revision: opts.revision,
    inputs: null,
    capture_sample_rate: null,
    capture_channels: null,
    wire_sample_rate: opts.wireSampleRate,
    wire_channels: 1,
    wire_encoding: 'pcm_f32le',
    io_buffer_duration_ms: null,
  };
}

/** Minimum voiced audio for an automatic (hands-free) capture to be sent as a turn. */
export const MIN_TURN_VOICED_MS = 400;

/**
 * V-18: a hands-free capture with under ~0.4 s of voiced audio is a knock,
 * click or TTS bleed, not a turn. A full-scale peak on a short capture is
 * the signature of a transient, so that bar is a little higher.
 */
export function isClippedTransient(opts: {
  trigger: VoiceTrigger;
  voicedMs: number;
  peak: number;
}): boolean {
  if (opts.trigger !== 'vad' && opts.trigger !== 'barge') return false;
  if (opts.voicedMs < MIN_TURN_VOICED_MS) return true;
  return opts.peak >= 0.98 && opts.voicedMs < 800;
}

/** Output device labels that mean the mic cannot hear the speaker. */
export function isHeadphoneOutputLabel(label: string): boolean {
  return /airpods|headphone|headset|earbud|earphone|bluetooth|beats|bose|sony wh|jabra|buds/i.test(label);
}

export const PLAYBACK_PEAK_HOLD_MS = 1500;
export const SPEAKER_BARGE_CONFIRMATIONS = 3;
export const SPEAKER_BARGE_WINDOW_MS = 1200;
export const BARGE_RMS = 0.05;
/** Share of the recent speaker peak the mic must exceed to count as someone else talking. */
const ECHO_COUPLING = 0.6;

export type BargeVerdict = 'ignore' | 'pending' | 'confirmed';

/**
 * Hands-free barge-in decision, mirroring the iPhone fix in 39ea442b.
 * On headphones: two consecutive loud buffers. On the Mac's speakers there
 * is little or no echo cancellation, so the floor is sized from the speaker's
 * peak over the last 1.5 s (not the instantaneous level, which dips between
 * words) and three over-floor, voice-shaped buffers within 1.2 s are needed.
 */
export function createBargeGuard() {
  const peak = createPeakHold(PLAYBACK_PEAK_HOLD_MS);
  let count = 0;
  let firstAt = 0;
  let consecutive = 0;
  return {
    /** Feed every playback tick (even when the mic is quiet). */
    notePlayback(nowMs: number, playbackRms: number): number {
      return peak.update(nowMs, playbackRms);
    },
    onMicBuffer(opts: { nowMs: number; micRms: number; voiceLike: boolean; speakerRoute: boolean }): BargeVerdict {
      const { nowMs, micRms, voiceLike, speakerRoute } = opts;
      const floor = speakerRoute ? Math.max(BARGE_RMS, ECHO_COUPLING * peak.current(nowMs)) : BARGE_RMS;
      const over = micRms > floor && voiceLike;
      if (!speakerRoute) {
        consecutive = over ? consecutive + 1 : 0;
        return consecutive >= 2 ? (this.reset(), 'confirmed') : over ? 'pending' : 'ignore';
      }
      if (!over) return 'ignore';
      if (count === 0 || nowMs - firstAt > SPEAKER_BARGE_WINDOW_MS) {
        count = 0;
        firstAt = nowMs;
      }
      count += 1;
      if (count < SPEAKER_BARGE_CONFIRMATIONS) return 'pending';
      this.reset();
      return 'confirmed';
    },
    peakNow(nowMs: number): number {
      return peak.current(nowMs);
    },
    reset() {
      count = 0;
      consecutive = 0;
      firstAt = 0;
    },
    resetAll() {
      this.reset();
      peak.reset();
    },
  };
}

/** Client -> server playback acknowledgements (V-26). */
export function segmentStartedMsg(a: { segmentId: number; turnId: string | null; clientMs: number }) {
  return { type: 'segment_started', segment_id: a.segmentId, turn_id: a.turnId, client_ms: a.clientMs };
}
export function segmentEndedMsg(a: {
  segmentId: number;
  turnId: string | null;
  playedMs: number;
  durationMs: number;
  completed: boolean;
  clientMs: number;
}) {
  return {
    type: 'segment_ended',
    segment_id: a.segmentId,
    turn_id: a.turnId,
    played_ms: Math.round(a.playedMs),
    duration_ms: Math.round(a.durationMs),
    completed: a.completed,
    client_ms: a.clientMs,
  };
}
export function replyPlaybackCompleteMsg(a: {
  turnId: string | null;
  segments: number;
  heardMs: number;
  queuedMs: number;
  reason: 'drained' | 'interrupt' | 'halt';
  clientMs: number;
}) {
  return {
    type: 'reply_playback_complete',
    turn_id: a.turnId,
    segments: a.segments,
    heard_ms: Math.round(a.heardMs),
    queued_ms: Math.round(a.queuedMs),
    completed: a.reason === 'drained',
    reason: a.reason,
    client_ms: a.clientMs,
  };
}

export type InterruptReason = 'barge_in' | 'user_tap' | 'watchdog';
export function interruptMsg(a: {
  turnId: string | null;
  reason: InterruptReason;
  heardMs: number;
  micRms?: number;
  playbackPeak?: number;
  route?: 'speaker' | 'headphones';
  clientMs: number;
}) {
  return {
    type: 'interrupt',
    turn_id: a.turnId,
    reason: a.reason,
    heard_ms: Math.round(a.heardMs),
    mic_rms: a.micRms ?? null,
    playback_peak: a.playbackPeak ?? null,
    route: a.route ?? null,
    client_ms: a.clientMs,
  };
}

/** How long the client waits for `interrupted` before falling back to a socket reset. */
export const INTERRUPT_ACK_TIMEOUT_MS = 800;
