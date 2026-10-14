/**
 * useVoice — push-to-talk voice hook for the Chat window.
 *
 * Connection model: persistent socket, opened once and kept alive.
 * Mic: acquired at activation, released at deactivation. Recording start/stop
 *       is synchronous (no getUserMedia race on quick press-and-release).
 * State machine: idle → connecting → ready → recording → processing → playing → ready
 *
 * Speaker print lives on the hub (`voice_print` after ready). Enrollment UI is
 * iOS-only; this desktop hook fails OPEN when no print exists and otherwise
 * shares the same cosine gate — rejected speech arrives as `idle`, like empty STT.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getApiBaseUrl } from '../lib/api';
import { copyText } from '../lib/clipboard';
import { getStreamToken } from '../lib/streamToken';
import { markReplySpoken } from '../lib/speakReplies';
import { spectrumLooksLikeVoice } from './vadSpectrum';
import { getAppVersion } from '../lib/version';
import { useCommandCenter } from '../lib/store';
import {
  beginPlayback,
  clearPlayback,
  endPlayback,
  makeSegment,
  parseWordTimings,
  rawPlaybackRms,
  type WordSpan,
} from '../lib/voicePlaybackClock';
import {
  buildHello,
  createBargeGuard,
  deriveVoiceSurface,
  INTERRUPT_ACK_TIMEOUT_MS,
  interruptMsg,
  isClippedTransient,
  isHeadphoneOutputLabel,
  replyPlaybackCompleteMsg,
  segmentEndedMsg,
  segmentStartedMsg,
  type InterruptReason,
  type VoiceSurface,
  type VoiceTrigger,
} from './voiceClientProtocol';

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
 * Consecutive zero-byte recordings (started, then stopped having captured no
 * frames at all) before the watchdog forces a recovery regardless of state or
 * frame traffic.
 *
 * The liveness half of the watchdog (below) only ever saw `processing`/
 * `playing` sitting idle — a fast start/stop loop looks alive to it because
 * every `Idle` frame resets `lastActivityRef`, and the loop never even
 * REACHES `processing` (a 0-sample stop returns straight to `ready`). The
 * 2026-09-01 incident free-ran ~1,430 such cycles at ~7.9 Hz over three
 * minutes without ever tripping VOICE_WATCHDOG_MS. This backstops fix 3a's
 * own (now removed) teach-specific cap for any fast loop that captures no audio.
 */
export const VOICE_ZERO_BYTE_CAPTURE_STREAK_LIMIT = 5;

/**
 * Pure predicate: is a busy voice state wedged and in need of a forced clear?
 *
 * Wedged = EITHER (a) a mic-locking state (`processing`/`playing`), with NO
 * audio actively playing or queued (so it isn't making local progress), and
 * no daemon activity for longer than the threshold; OR (b) a run of
 * consecutive zero-byte capture cycles — a fast start/stop loop that never
 * captures anything is wedged regardless of which state it currently reports
 * or how much frame traffic is flowing. Extracted so the watchdog decision is
 * unit-testable without a live audio session.
 */
export function isVoiceWedged(opts: {
  state: VoiceState;
  isPlaying: boolean;
  queuedChunks: number;
  msSinceActivity: number;
  thresholdMs?: number;
  consecutiveZeroByteCaptures?: number;
  zeroByteCaptureLimit?: number;
}): boolean {
  const {
    state,
    isPlaying,
    queuedChunks,
    msSinceActivity,
    thresholdMs = VOICE_WATCHDOG_MS,
    consecutiveZeroByteCaptures = 0,
    zeroByteCaptureLimit = VOICE_ZERO_BYTE_CAPTURE_STREAK_LIMIT,
  } = opts;
  if (consecutiveZeroByteCaptures >= zeroByteCaptureLimit) return true;
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

/**
 * Pure routing for a daemon keyword detection (`{"type":"wake","kind":...}`).
 *
 * "wake" opens a turn, but only from `ready` — mid-turn it has nothing to
 * open. "stop" halts a reply being spoken or produced; in `ready` there is
 * nothing to halt (the utterance "stop" while idle is not a command).
 * Extracted so the routing is unit-testable without a live socket.
 */
/** Empty / too-short captures are not faults — last night they flashed
 *  "No speech detected" on the orb after real speech (20260821_14). */
export function isTransientVoiceIdle(message: string | null | undefined): boolean {
  if (!message) return false;
  const m = message.toLowerCase();
  return m.includes('no speech detected') || m.includes('recording too short');
}

export function routeWakeEvent(
  kind: string,
  state: VoiceState,
): 'start-turn' | 'halt-playback' | 'ignore' {
  if (kind === 'wake') return state === 'ready' ? 'start-turn' : 'ignore';
  if (kind === 'stop') {
    return state === 'playing' || state === 'processing' ? 'halt-playback' : 'ignore';
  }
  return 'ignore';
}

// ── Endpointing ────────────────────────────────────────────────────────────
//
// How long we keep `recording` after the speaker goes quiet. This window is
// the whole of the "the app sits on listening too long" complaint on the
// client side: nothing downstream starts until it expires.
//
// Two tiers, matching ios/PermagentMobile/PermagentMobile/VoiceVAD.swift. A
// short conversational ask is almost always finished when the speaker stops,
// so it gets the tight window; a dictation-length turn contains mid-thought
// pauses that must survive, so it keeps the patient one. The agents that run a
// tight window on BOTH tiers (LiveKit, Pipecat, OpenAI Realtime's semantic
// mode) all pair the timer with a semantic end-of-turn model; we have no such
// signal, and a false endpoint costs far more than the extra 400 ms.
// See docs/research/VOICE_LATENCY_AND_ORB_2026-08-25.md §3.

/** Tight window, for a turn with less than {@link VAD_QUICK_TURN_SPEECH_MS} of
 *  voiced audio. 500 ms is OpenAI Realtime's `server_vad` `silence_duration_ms`
 *  default, inside the 300-800 ms band LiveKit and Pipecat also land in. */
export const VAD_QUICK_SILENCE_MS = 500;

/** Patient window, for a dictation-length turn. Unchanged. */
export const VAD_SILENCE_MS = 900;

/** Voiced duration below which a turn counts as a quick ask. */
export const VAD_QUICK_TURN_SPEECH_MS = 3_500;

/**
 * Pure: the trailing-silence window that should end a turn whose voiced
 * duration so far is `voicedMs`. Exported so the endpoint timing is testable
 * without an audio graph.
 */
export function endpointWindowMs(
  voicedMs: number,
  opts: { quickMs?: number; longMs?: number; quickTurnMs?: number } = {},
): number {
  const {
    quickMs = VAD_QUICK_SILENCE_MS,
    longMs = VAD_SILENCE_MS,
    quickTurnMs = VAD_QUICK_TURN_SPEECH_MS,
  } = opts;
  // A quick window longer than the patient one would invert the two tiers.
  const quick = Math.min(quickMs, longMs);
  if (!Number.isFinite(voicedMs) || voicedMs < 0) return longMs;
  return voicedMs < quickTurnMs ? quick : longMs;
}

/** Hard safety cap for a single hands-free utterance. */
export const VAD_MAX_TURN_MS = 45_000;

export interface VadTurnTiming {
  heardSpeech: boolean;
  lastVoiceAt: number;
  turnStartedAt: number;
}

export type VoiceTurnEndReason =
  | 'silence'
  | 'max_turn'
  | 'user_tap'
  | 'client_error'
  | 'interruption'
  | 'noise_abort'
  | 'barge'
  | 'unknown';

/**
 * Initialize every clock used by hands-free endpointing.
 *
 * Wake-word turns and ordinary VAD turns must enter `recording` through this
 * same seam. The old wake path left both timestamps at zero, so the first
 * monitor callback computed `now - 0 > VAD_MAX_TURN_MS` and stopped recording
 * before its 256 ms ScriptProcessor emitted a single buffer.
 */
export function beginVadTurn(now: number, heardSpeech: boolean): VadTurnTiming {
  return { heardSpeech, lastVoiceAt: now, turnStartedAt: now };
}

/**
 * Pure endpoint decision, kept outside the audio callback for regression tests.
 *
 * `opts.recordingStartedAt`, when given, is when the CURRENT recording
 * actually began — set unconditionally by every call to `startRecording`
 * (see its docstring). It's a floor under `timing.turnStartedAt`: a
 * `turnStartedAt` older than the recording it's supposed to describe is by
 * definition a stale leftover from a PREVIOUS turn — the exact failure class
 * `beginVadTurn` exists to close — so it's judged from the recording's own
 * start instead of trusting the stale clock. Defense in depth on top of the
 * single-seam fix in `startRecording`: even a future call site that forgets
 * to arm the turn clocks can't make this function kill a turn before its
 * first buffer.
 */
export function shouldEndVadTurn(
  timing: VadTurnTiming,
  now: number,
  opts: { maxTurnMs?: number; recordingStartedAt?: number } = {},
): boolean {
  const maxTurnMs = opts.maxTurnMs ?? VAD_MAX_TURN_MS;
  const effectiveStart =
    opts.recordingStartedAt !== undefined
      ? Math.max(timing.turnStartedAt, opts.recordingStartedAt)
      : timing.turnStartedAt;
  const turnFor = Math.max(0, now - effectiveStart);
  if (turnFor > maxTurnMs) return true;
  if (!timing.heardSpeech) return false;
  const silentFor = Math.max(0, now - timing.lastVoiceAt);
  const voicedFor = Math.max(0, timing.lastVoiceAt - effectiveStart);
  return silentFor > endpointWindowMs(voicedFor);
}

export function classifyVadTurnEnd(
  timing: VadTurnTiming,
  now: number,
  opts: { maxTurnMs?: number; recordingStartedAt?: number } = {},
): VoiceTurnEndReason | null {
  const maxTurnMs = opts.maxTurnMs ?? VAD_MAX_TURN_MS;
  const effectiveStart =
    opts.recordingStartedAt !== undefined
      ? Math.max(timing.turnStartedAt, opts.recordingStartedAt)
      : timing.turnStartedAt;
  const turnFor = Math.max(0, now - effectiveStart);
  if (turnFor > maxTurnMs) return 'max_turn';
  if (!timing.heardSpeech) return null;
  const silentFor = Math.max(0, now - timing.lastVoiceAt);
  const voicedFor = Math.max(0, timing.lastVoiceAt - effectiveStart);
  return silentFor > endpointWindowMs(voicedFor) ? 'silence' : null;
}

// ── Mic-input watchdog ────────────────────────────────────────────────────
//
// "It isn't responding and just gets stuck on Listening" (2026-09-18). The
// daemon log showed the wake/VAD gate never opened a turn across five
// sessions, and nothing on the client could tell "armed and hearing you" from
// "armed and receiving nothing". The hands-free VAD graph is an AudioContext +
// ScriptProcessor on the persistent mic stream: a Bluetooth A2DP<->HFP profile
// flip or a sleep/wake suspends that context (or mutes/ends the track), the
// processor simply stops firing, and nothing noticed — `startVadMonitor`
// returns early while `vadCtxRef` exists, so the dead graph was never rebuilt
// and the orb said "Listening" forever. The pure half of the fix lives here so
// it is unit-testable without an audio graph, like `vadSpectrum.ts`.

/** How long the armed VAD graph may go without processing a buffer (or sit
 *  with a muted track) before the input counts as dead. The monitor fires
 *  every ~128ms, so 3s is ~23 missed buffers — never a hiccup. It is also the
 *  grace a freshly built graph gets before it is judged. */
export const MIC_STALL_MS = 3_000;

/** Once the fault is on screen, how often to try one more rebuild — cheap,
 *  and it recovers an input that came back without firing `devicechange`. */
export const MIC_FAULT_RETRY_MS = 10_000;

/** Watchdog poll period. */
export const MIC_WATCHDOG_TICK_MS = 1_000;

/** What the orb says instead of "Listening" when the mic is delivering nothing. */
export const MIC_FAULT_MESSAGE = 'Not hearing the microphone — check the input device';

/** Peak |sample| at or below which a buffer counts as digital silence —
 *  1e-5 is -100 dBFS, far under any real capsule's self-noise (even a quiet
 *  room with noise suppression sits tens of dB above it). What lands here is
 *  an input delivering zeros: an HFP/SCO link renegotiating, or a device
 *  muted in hardware. Those buffers still ARRIVE, so a liveness check that
 *  only asked "did the processor fire?" called them healthy. */
export const MIC_SILENCE_PEAK = 1e-5;

/** How long the armed input may deliver only digital silence before it
 *  counts as dead. Longer than MIC_STALL_MS: silence is a softer signal than
 *  "no buffers at all", and a false alarm here would nag in a quiet room. */
export const MIC_SILENT_MS = 5_000;

/** What the orb says when buffers arrive but carry nothing. */
export const MIC_SILENT_MESSAGE = 'Microphone is sending silence — check the input device';

export type MicInputFault =
  | 'no-stream'           // no mic stream at all (re-acquire failed)
  | 'track-ended'         // device unplugged / revoked
  | 'no-graph'            // VAD graph missing while hands-free is armed
  | 'context-not-running' // AudioContext suspended / interrupted / closed
  | 'track-muted'         // the OS is feeding the track no audio
  | 'no-buffers'          // graph "running" but the processor stopped firing
  | 'silent';             // buffers arrive, but every one is digital silence

export interface MicInputSample {
  hasStream: boolean;
  trackEnded: boolean;
  hasGraph: boolean;
  /** `AudioContext.state` of the VAD graph, or null without one. */
  ctxState: string | null;
  /** How long the track has been continuously muted (0 = not muted). */
  msTrackMuted: number;
  /** Since the last buffer processed by the CURRENT graph; Infinity if the
   *  current graph has never processed one. */
  msSinceLastBuffer: number;
  /** Since the current graph last delivered a buffer with any signal in it
   *  (peak above MIC_SILENCE_PEAK) — see {@link msSinceMicSignal}. */
  msSinceSignal: number;
}

/**
 * Pure: how long the input has been digitally silent, for the watchdog.
 *
 * Measured from the LATEST of: the last non-silent buffer, the graph build
 * (a buffer from an older graph says nothing about this one), and the moment
 * the watchdog began judging this stretch (`judgingSince`). The last matters
 * because echo cancellation may legitimately gate the mic to zeros while the
 * agent is SPEAKING — a state the watchdog does not judge — and that stretch
 * must not count against the mic the instant the loop returns to 'ready'.
 */
export function msSinceMicSignal(opts: {
  now: number;
  lastSignalAt: number;
  graphBuiltAt: number;
  judgingSince: number;
}): number {
  const { now, lastSignalAt, graphBuiltAt, judgingSince } = opts;
  return Math.max(0, now - Math.max(lastSignalAt, graphBuiltAt, judgingSince));
}

/** Pure: peak |x| of a buffer — the digital-silence test's input. */
export function bufferPeak(input: ArrayLike<number>): number {
  let peak = 0;
  for (let i = 0; i < input.length; i++) {
    const v = input[i] < 0 ? -input[i] : input[i];
    if (v > peak) peak = v;
  }
  return peak;
}

/**
 * Pure: is the armed hands-free input actually delivering audio? Returns the
 * most specific fault, or null when healthy. Order matters only for the
 * diagnosis logged/shown — any fault means "not hearing you".
 */
export function diagnoseMicInput(
  s: MicInputSample,
  stallMs: number = MIC_STALL_MS,
  silentMs: number = MIC_SILENT_MS,
): MicInputFault | null {
  if (!s.hasStream) return 'no-stream';
  if (s.trackEnded) return 'track-ended';
  if (!s.hasGraph) return 'no-graph';
  if (s.ctxState !== 'running') return 'context-not-running';
  if (s.msTrackMuted > stallMs) return 'track-muted';
  if (!Number.isFinite(s.msSinceLastBuffer) || s.msSinceLastBuffer > stallMs) return 'no-buffers';
  if (s.msSinceSignal > silentMs) return 'silent';
  return null;
}

export type MicWatchdogAction = 'healthy' | 'wait' | 'rebuild' | 'surface-fault';

/**
 * Pure: what the watchdog does with a diagnosis.
 *
 * One rebuild first (a profile flip usually just needs a fresh context), and
 * only if the rebuilt graph is ALSO dead once its grace expires does the fault
 * go on screen. After that, a slow retry. `msSinceGraphBuilt` counts from the
 * last build ATTEMPT, so a graph is never judged inside its own start-up.
 *
 * `provenLive`: the current graph has delivered at least one buffer with
 * signal in it. No fault is not the same as proof — a freshly (re)built graph
 * pumping zeros is inside its silence window, and treating that as 'healthy'
 * reset the rebuild count and cleared the on-screen fault every retry, so an
 * all-zero input flapped between rebuilds and never said what was wrong.
 */
export function micWatchdogStep(opts: {
  fault: MicInputFault | null;
  provenLive?: boolean;
  rebuildAttempts: number;
  msSinceGraphBuilt: number;
  faultSurfaced: boolean;
  graceMs?: number;
  retryMs?: number;
}): MicWatchdogAction {
  const {
    fault,
    provenLive = true,
    rebuildAttempts,
    msSinceGraphBuilt,
    faultSurfaced,
    graceMs = MIC_STALL_MS,
    retryMs = MIC_FAULT_RETRY_MS,
  } = opts;
  if (!fault) return provenLive ? 'healthy' : 'wait';
  if (msSinceGraphBuilt < graceMs) return 'wait';
  if (rebuildAttempts <= 0) return 'rebuild';
  if (!faultSurfaced) return 'surface-fault';
  return msSinceGraphBuilt >= retryMs ? 'rebuild' : 'wait';
}

/** Pure: the user-facing sentence for a fault. */
export function micFaultMessage(fault: MicInputFault): string {
  if (fault === 'no-stream' || fault === 'track-ended') {
    return 'Microphone disconnected — check the input device';
  }
  if (fault === 'silent') return MIC_SILENT_MESSAGE;
  return MIC_FAULT_MESSAGE;
}

// ── Deaf wake-word spotter (hint only) ─────────────────────────────────────
//
// The rest of "stuck on listening": the 2026-09-01 forensics found a Bose HFP
// mic that DID deliver buffers — low-level, but speech — to a keyword spotter
// that never fired, behind an armed wake gate. While gated, VAD onset may not
// open a turn and only a wake hit clears the gate, so the orb said
// `Say "Hey Henry"` forever while the user said exactly that. The mic watchdog
// cannot see this (the input is alive); only the pattern can: repeated,
// sustained, voice-shaped audio and a spotter that has never answered.
//
// Once that pattern holds, the orb shows WAKE_DEAF_HINT instead of asking for
// a phrase nobody is hearing (until deactivate, a wake hit, switching the wake
// word off, or an input-device change). The gate itself stays CLOSED: bare
// speech never opens a turn behind an armed wake word. A spotter that has
// fired even once on the current connection is working, and is never flagged.

/** Minimum time armed-and-gated before the spotter can be judged deaf. */
export const WAKE_DEAF_GATED_MS = 15_000;

/** Distinct voiced episodes (sustained, voice-shaped onsets with a quiet gap
 *  between them) the spotter must have ignored. Three is "said it, said it
 *  again, said it a third time" — a single cough or a sentence is not. */
export const WAKE_DEAF_VOICED_EPISODES = 3;

/**
 * Shown when the wake-word spotter has ignored several voiced episodes.
 *
 * It used to read "just talk" because the verdict also OPENED the wake gate.
 * That was the wrong trade: the verdict cannot tell a deaf spotter from speech
 * that simply was not addressed to the agent, so fifteen seconds of room
 * conversation or a TV was enough to disable a working wake word for the
 * rest of the activation — after which the agent answered everything said
 * nearby. The gate now stays closed and this says what is true and what the
 * user can actually do; there is no tap-to-talk on the orb (its click ends
 * the conversation), so it does not offer one.
 */
export const WAKE_DEAF_HINT = 'Not hearing the wake word — check the mic, or turn the wake word off in Settings';

/** Per-buffer RMS a sustained, voice-shaped streak must exceed to open a turn
 *  from 'ready'. Set against keyboard clicks and room noise on a normal mic. */
export const VAD_ONSET_RMS = 0.025;
/** Per-buffer RMS that keeps a recording turn alive, and the level the
 *  deaf-spotter episodes are counted at. */
export const VAD_KEEPALIVE_RMS = 0.010;

/** Pure: has the wake-word spotter ignored enough speech to show the deaf hint? */
export function wakeSpotterLooksDeaf(opts: {
  gatedForMs: number;
  voicedEpisodes: number;
  spotterHasFired: boolean;
  minGatedMs?: number;
  minEpisodes?: number;
}): boolean {
  const {
    gatedForMs,
    voicedEpisodes,
    spotterHasFired,
    minGatedMs = WAKE_DEAF_GATED_MS,
    minEpisodes = WAKE_DEAF_VOICED_EPISODES,
  } = opts;
  if (spotterHasFired) return false;
  return gatedForMs >= minGatedMs && voicedEpisodes >= minEpisodes;
}

/** localStorage key for the wake-word preference (default: enabled). */
export const WAKE_WORD_LS_KEY = 'voice:wakeWordEnabled';

/**
 * After this long sitting idle in `ready`, the conversation is considered
 * over and the wake gate re-arms: the next turn needs the wake phrase again
 * instead of bare voice onset. While engaged (within this window of the last
 * turn), VAD onset keeps turns flowing so the user isn't forced to repeat
 * "hey <name>" every exchange.
 */
export const WAKE_REARM_MS = 30_000;

// ── Engine verdicts: relayed to mirror windows, kept for inspection ─────────
//
// The watchdog and the deaf-spotter hint decide things the user must SEE,
// and the 2026-09-18 report ("isn't responding and just gets stuck on
// listening") reached the user through two gaps after both fixes shipped:
//
//  1. The popped-out chat window renders a MIRROR orb (ChatApp) from the live
//     feed, which carries only state + level. Its window has no engine in its
//     store, so over a dead mic it said "Voice error — click to exit", over a
//     failed-open gate plain "Listening", and while wake-gated plain
//     "Listening" instead of `Say "Hey Henry"` — the exact "stuck on
//     listening" words. The engine now publishes what it decided under its own
//     key, and a mirror orb reads it (VoiceOrb, mirror mode). Same transport
//     and reasoning as lib/voiceHandoff: localStorage, polled, because
//     WKWebView does not reliably deliver `storage` events across webviews.
//
//  2. QA could not confirm either verdict: they were `console.warn` lines only,
//     and the daemon has no message to log them under (daemon.err was searched
//     and could never have shown them). The engine now also keeps a short,
//     bounded trail of verdict EVENTS in localStorage, which WKWebView persists
//     to the app's LocalStorage sqlite on disk — readable without driving the
//     GUI or opening the console.
//
//  3. That trail only recorded FAILURES, so the first real-app check could not
//     confirm anything: the snapshot read {"error":null,"wakeHint":null,...}
//     and the trail was empty, which is equally "hands-free was never armed"
//     and "armed, the watchdog judged the mic, and it was fine". The trail now
//     also records the lifecycle — hands-free armed (with the input it got)
//     and the watchdog's first healthy verdict on each graph — and the
//     snapshot says whether hands-free is on, so a session with no fault at
//     all still proves the watchdog ran against the real input.
//
//  4. The second real-app check still could not confirm: the snapshot was
//     {"error":null,"wakeHint":null,"spotterDeaf":false} with no `handsFree`
//     field and the trail was EMPTY. Both are what a bundle built before (3)
//     writes, and an empty trail is ALSO what (3) writes for a session in
//     which hands-free was never switched on (the watchdog only runs while it
//     is) — so "the fix never reached the app" and "it did, nobody armed it"
//     read the same. The engine now stamps its trail format into the snapshot
//     (`schema`) and writes an `engine_up` line when it mounts, saying whether
//     hands-free is armed. A bundle carrying this code can no longer leave an
//     empty trail, and a trail with `engine_up` but no `hands_free_armed`
//     says, in words, that the watchdog had nothing to judge yet.

/** Current engine verdict (owner window → mirror orbs). */
export const VOICE_VERDICT_KEY = 'permagent-voice-verdict';
/** Bounded trail of verdict events, newest last, for after-the-fact checks. */
export const VOICE_VERDICT_LOG_KEY = 'permagent-voice-verdict-log';
// 40, not 20: each hands-free session now adds two lifecycle lines, and they
// must not push the fault lines QA is looking for out of the trail.
export const VOICE_VERDICT_LOG_MAX = 40;
/** Version of what the engine writes under the two keys above. Bump it when
 *  the snapshot or the trail changes shape, so a real-app check can tell which
 *  bundle wrote what it is reading (a missing `schema` = a pre-v4 bundle). */
export const VOICE_VERDICT_SCHEMA = 4;

export interface VoiceVerdict {
  /** The engine's error text (the watchdog's sentence, or any other fault). */
  error: string | null;
  /** Armed wake gate: what to say to open a turn (e.g. `Say "Hey Henry"`). */
  wakeHint: string | null;
  /** The spotter ignored repeated speech; the orb shows WAKE_DEAF_HINT (the gate stays closed). */
  spotterDeaf: boolean;
}

export type VoiceVerdictEventName =
  | 'engine_up'           // the engine mounted; detail = trail schema + hands-free state
  | 'hands_free_armed'    // hands-free switched on; detail = input + graph state
  | 'hands_free_off'
  | 'mic_live'            // the watchdog's first healthy verdict on a graph
  | 'mic_rebuild'
  | 'mic_fault_surfaced'
  | 'mic_fault_cleared'
  | 'wake_spotter_looks_deaf';

export interface VoiceVerdictEvent {
  /** ISO timestamp, so the trail lines up against daemon.err by eye. */
  at: string;
  event: VoiceVerdictEventName;
  detail: string | null;
}

/**
 * Pure: the wake hint for a wake-word surface. The same rule VoiceHost applies
 * to the main-window takeover; exported so the relayed verdict cannot drift
 * from it.
 */
export function wakeHintFor(w: { active: boolean; gated: boolean; phrase: string | null }): string | null {
  return w.active && w.gated && w.phrase ? `Say "${w.phrase}"` : null;
}

/** Pure: how a mic stream is named in the verdict trail — the track label
 *  (e.g. "Bose QC35 II") is what tells an HFP headset from the built-in mic. */
export function micInputLabel(
  stream: { getTracks(): Array<{ label?: string; readyState?: string }> } | null,
): string {
  if (!stream) return 'no microphone stream';
  const track = stream.getTracks()[0];
  if (!track) return 'no microphone track';
  const name = track.label ? `"${track.label}"` : 'unnamed input';
  return track.readyState === 'ended' ? `${name} (ended)` : name;
}

/** `handsFree` rides along in the stored snapshot only (mirror orbs do not
 *  need it); it is what lets QA tell "never armed" from "armed and healthy". */
export function publishVoiceVerdict(v: VoiceVerdict & { handsFree?: boolean }): void {
  try {
    localStorage.setItem(
      VOICE_VERDICT_KEY,
      JSON.stringify({ ...v, schema: VOICE_VERDICT_SCHEMA, at: Date.now() }),
    );
  } catch { /* private mode / quota: the main window still reads the store */ }
}

export function clearVoiceVerdict(): void {
  try { localStorage.removeItem(VOICE_VERDICT_KEY); } catch { /* ignore */ }
}

export function readVoiceVerdict(): VoiceVerdict | null {
  try {
    const raw = localStorage.getItem(VOICE_VERDICT_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<VoiceVerdict>;
    return {
      error: typeof v.error === 'string' && v.error ? v.error : null,
      wakeHint: typeof v.wakeHint === 'string' && v.wakeHint ? v.wakeHint : null,
      spotterDeaf: v.spotterDeaf === true,
    };
  } catch {
    return null;
  }
}

export function readVoiceVerdictLog(): VoiceVerdictEvent[] {
  try {
    const raw = localStorage.getItem(VOICE_VERDICT_LOG_KEY);
    const list = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(list) ? (list as VoiceVerdictEvent[]) : [];
  } catch {
    return [];
  }
}

export function recordVoiceVerdictEvent(event: VoiceVerdictEventName, detail: string | null = null): void {
  try {
    const next = [...readVoiceVerdictLog(), { at: new Date().toISOString(), event, detail }];
    localStorage.setItem(VOICE_VERDICT_LOG_KEY, JSON.stringify(next.slice(-VOICE_VERDICT_LOG_MAX)));
  } catch { /* the console line is still there */ }
}

/** Pure: the `engine_up` detail. Says outright when the watchdog has nothing
 *  to judge, because an absent watchdog line is otherwise unreadable. */
export function engineUpDetail(handsFree: boolean): string {
  return handsFree
    ? `schema ${VOICE_VERDICT_SCHEMA}; hands-free on — the mic watchdog is judging the input`
    : `schema ${VOICE_VERDICT_SCHEMA}; hands-free off — the mic watchdog runs only once hands-free is armed`;
}

/**
 * Mark the engine's mount in the trail. Consecutive mounts with nothing in
 * between (a window reload, a remount) REPLACE the previous `engine_up` line
 * rather than stacking, so reloads cannot push the fault lines QA is looking
 * for out of the bounded trail.
 */
export function recordEngineUp(handsFree: boolean): void {
  try {
    const log = readVoiceVerdictLog();
    const last = log[log.length - 1];
    const kept = last && last.event === 'engine_up' ? log.slice(0, -1) : log;
    const next = [...kept, { at: new Date().toISOString(), event: 'engine_up' as const, detail: engineUpDetail(handsFree) }];
    localStorage.setItem(VOICE_VERDICT_LOG_KEY, JSON.stringify(next.slice(-VOICE_VERDICT_LOG_MAX)));
  } catch { /* the snapshot's `schema` still identifies the bundle */ }
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

/** Build identity for the hello (V-28). Injected by the bundler when available. */
let cachedAppVersion: string | null = null;
void getAppVersion().then(v => { cachedAppVersion = v; }).catch(() => {});
function clientBuild() {
  return {
    appVersion: cachedAppVersion,
    // Literal member access so the bundler's `define` replaces them.
    buildNumber: import.meta.env.VITE_BUILD_NUMBER ?? null,
    gitSha: import.meta.env.VITE_GIT_SHA ?? null,
  };
}

/** Which view the voice session belongs to right now: the World, or the chat-pane Orb. */
function currentVoiceSurface(): VoiceSurface {
  const st = useCommandCenter.getState();
  const ws = st.workspaces.find(w => w.id === st.activeWorkspaceId);
  let node = ws?.layoutJson;
  while (node && node.type === 'split' && node.children.length > 0) node = node.children[0];
  const activeTool = node && node.type === 'panel' ? node.tool : null;
  return deriveVoiceSurface({ worldConversationActive: st.worldConversationActive, activeTool });
}

type SegmentMeta = { segmentId: number; text: string; words: WordSpan[]; durationMs: number };

export function useVoice(options: UseVoiceOptions = {}) {
  const { sessionId, sampleRate = 16000, onEvent } = options;
  const [state, setState] = useState<VoiceState>('idle');
  const [lastTranscript, setLastTranscript] = useState('');
  const [lastReply, setLastReply] = useState('');
  const [error, setError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const turnSequenceRef = useRef(0);
  const activeTurnIdRef = useRef<string | null>(null);
  const activeTurnTriggerRef = useRef<VoiceTrigger>('unknown');
  const activeTurnAudioRevisionRef = useRef(1);
  const activeTurnStartedAtMonotonicRef = useRef<number | null>(null);
  const voiceHelloSentRef = useRef(false);
  // Persistent mic stream — acquired at activation, released at deactivation.
  // Eliminates the getUserMedia async gap that caused the press-release race.
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const processorRef = useRef<ScriptProcessorNode | null>(null);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const pendingAudioRef = useRef(false);
  const audioQueueRef = useRef<Float32Array[]>([]);
  const playingRef = useRef(false);
  const playbackCtxRef = useRef<AudioContext | null>(null);
  // ALSO (backstops fix 3a): consecutive recordings that captured zero
  // frames, regardless of state or which loop caused them. Reset in
  // stopRecording whenever a recording captures at least one frame.
  const zeroByteCaptureStreakRef = useRef(0);
  // Frequency analyser tapping the TTS output, so the chat can draw a live
  // waveform while the agent speaks. Lives on the playback context; recreated
  // with it, nulled when it closes.
  const analyserRef = useRef<AnalyserNode | null>(null);
  // ── Playback acknowledgements + in-band interrupt (V-26, V-29) ──
  /** `audio_segment` metadata awaiting its binary frame (they arrive in pairs, in order). */
  const pendingSegMetaRef = useRef<SegmentMeta[]>([]);
  const segMetaRef = useRef(new WeakMap<Float32Array, SegmentMeta>());
  const currentSegRef = useRef<{ meta: SegmentMeta; startedPerf: number; durationMs: number } | null>(null);
  const replyAckRef = useRef({ segments: 0, heardMs: 0, queuedMs: 0 });
  /** Client turn id the reply being played answers. */
  const replyTurnIdRef = useRef<string | null>(null);
  const awaitingInterruptAckRef = useRef(false);
  const interruptTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const surfaceRef = useRef<VoiceSurface>('orb');
  const bargeGuardRef = useRef(createBargeGuard());
  const speakerRouteRef = useRef(true);
  const lastBargeAtRef = useRef(0);
  const vadVoicedMsRef = useRef(0);
  const vadPeakRef = useRef(0);
  const sendJson = useCallback((obj: unknown) => {
    const w = wsRef.current;
    if (w && w.readyState === WebSocket.OPEN) w.send(JSON.stringify(obj));
  }, []);
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;
  const activeRef = useRef(false);
  const connectingRef = useRef(false);
  /// Last `error` frame the daemon sent on THIS socket. The server states the
  /// real diagnosis (e.g. "Voice providers not available — models not loaded")
  /// and then closes; the close arrives as 1006, whose generic label would
  /// otherwise bury the actual reason. Cleared on every fresh connect.
  const serverErrorRef = useRef<string | null>(null);
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

  // ── Wake word (#hands-free activation) ────────────────────────────────────
  // The daemon runs an on-device keyword spotter (sherpa-onnx KWS) over mic
  // monitor frames we stream while hands-free. "hey <name>" opens a turn
  // without touching the keyboard; a spoken "stop" ends the agent's turn.
  const [wakeWordEnabled, setWakeWordEnabledState] = useState<boolean>(() => {
    try {
      return localStorage.getItem(WAKE_WORD_LS_KEY) !== '0';
    } catch {
      return true;
    }
  });
  const wakeEnabledRef = useRef(wakeWordEnabled);
  // Daemon confirmed it is spotting on this socket.
  const [wakeActive, setWakeActive] = useState(false);
  const wakeActiveRef = useRef(false);
  const [wakePhrase, setWakePhrase] = useState<string | null>(null);
  // Gated = the wake phrase is required to open the next turn. Ungated
  // ("engaged") = a conversation is flowing and bare VAD onset opens turns.
  const [wakeGated, setWakeGated] = useState(false);
  const wakeGatedRef = useRef(false);
  // Deaf-spotter hint (see wakeSpotterLooksDeaf). `wakeDeaf` = the gate
  // flags that the spotter ignored repeated speech; the gate stays closed.
  const [wakeDeaf, setWakeDeaf] = useState(false);
  const wakeDeafRef = useRef(false);
  // The spotter has answered at least once on this handler — it works.
  const wakeSpotterFiredRef = useRef(false);
  // When the current armed-gate stretch began (0 = not gated).
  const wakeGatedSinceRef = useRef(0);
  // Voiced episodes heard during the current gated stretch, the running
  // voiced-buffer streak, and a latch so one long utterance counts once.
  const wakeVoicedEpisodesRef = useRef(0);
  const wakeEpisodeStreakRef = useRef(0);
  const wakeEpisodeLatchRef = useRef(false);
  // Last moment the loop was in any non-ready state — the "conversation is
  // live" clock that decides when the wake gate re-arms.
  const lastConvActivityRef = useRef(0);
  // Retry timer while the daemon reports the KWS model is still downloading.
  const wakeRetryRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Third state between inactive and acknowledged-active. Socket-ready and
  // setHandsFree can reconcile in the same activation; this prevents both
  // from sending wake_start and resetting the spotter back-to-back.
  const wakeStartPendingRef = useRef(false);
  const syncWakeModeRef = useRef<(() => void) | null>(null);

  // Output route: the barge-in echo guard is stricter on the Mac's speakers.
  // Defaults to speaker (the conservative answer) when labels are unavailable.
  useEffect(() => {
    const md = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    if (!md?.enumerateDevices) return;
    let cancelled = false;
    const refresh = async () => {
      try {
        const outs = (await md.enumerateDevices()).filter(d => d.kind === 'audiooutput');
        const def = outs.find(d => d.deviceId === 'default') ?? outs[0];
        if (!cancelled) speakerRouteRef.current = !(def && isHeadphoneOutputLabel(def.label));
      } catch { /* keep the conservative default */ }
    };
    void refresh();
    md.addEventListener?.('devicechange', refresh);
    return () => { cancelled = true; md.removeEventListener?.('devicechange', refresh); };
  }, []);

  // The same engine serves the World and the chat-pane Orb; if the user moves
  // between them mid-session, restate the surface to the daemon.
  useEffect(() => useCommandCenter.subscribe(() => {
    const next = currentVoiceSurface();
    if (next === surfaceRef.current) return;
    surfaceRef.current = next;
    if (voiceHelloSentRef.current) {
      sendJson(buildHello({ surface: next, build: clientBuild(), wireSampleRate: sampleRate, revision: 1 }));
    }
  }), [sendJson, sampleRate]);

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

  // --- Playback acknowledgements (V-26) ---
  const finishSegmentRef = useRef<((completed: boolean) => void) | null>(null);
  const finishReplyRef = useRef<((reason: 'drained' | 'interrupt' | 'halt') => void) | null>(null);
  finishSegmentRef.current = (completed: boolean) => {
    const cur = currentSegRef.current;
    if (!cur) return;
    currentSegRef.current = null;
    const played = completed ? cur.durationMs : Math.min(cur.durationMs, performance.now() - cur.startedPerf);
    replyAckRef.current.heardMs += played;
    endPlayback(cur.meta.segmentId);
    sendJson(segmentEndedMsg({
      segmentId: cur.meta.segmentId,
      turnId: replyTurnIdRef.current,
      playedMs: played,
      durationMs: cur.durationMs,
      completed,
      clientMs: Date.now(),
    }));
  };
  finishReplyRef.current = (reason) => {
    finishSegmentRef.current?.(reason === 'drained');
    const ack = replyAckRef.current;
    if (ack.queuedMs === 0 && ack.segments === 0) return;
    replyAckRef.current = { segments: 0, heardMs: 0, queuedMs: 0 };
    sendJson(replyPlaybackCompleteMsg({
      turnId: replyTurnIdRef.current,
      segments: ack.segments,
      heardMs: ack.heardMs,
      queuedMs: ack.queuedMs,
      reason,
      clientMs: Date.now(),
    }));
  };

  // --- Audio playback queue ---
  const playNextChunk = useCallback(() => {
    if (playingRef.current) return;
    const next = audioQueueRef.current.shift();
    if (!next || next.length === 0) {
      if (!pendingAudioRef.current) {
        playbackCtxRef.current?.close().catch(() => {});
        playbackCtxRef.current = null;
        analyserRef.current = null;
        finishReplyRef.current?.('drained');
        if (wsRef.current?.readyState === WebSocket.OPEN) {
          setStateAndEmit('ready');
          // Narration finished playing — release any deferred navigation.
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
      const meta = segMetaRef.current.get(next);
      source.onended = () => {
        if (meta && currentSegRef.current?.meta === meta) finishSegmentRef.current?.(true);
        playingRef.current = false;
        playNextChunk();
      };
      source.start();
      if (meta) {
        const latencyMs = ((ctx as AudioContext & { outputLatency?: number }).outputLatency ?? ctx.baseLatency ?? 0) * 1000;
        const startedPerf = performance.now();
        currentSegRef.current = { meta, startedPerf, durationMs: (next.length / 24000) * 1000 };
        beginPlayback(makeSegment({
          segmentId: meta.segmentId,
          startedAtMs: startedPerf + latencyMs,
          text: meta.text,
          words: meta.words,
          samples: next,
          sampleRate: 24000,
          durationMs: meta.durationMs,
        }));
        sendJson(segmentStartedMsg({ segmentId: meta.segmentId, turnId: replyTurnIdRef.current, clientMs: Date.now() }));
        replyAckRef.current.segments += 1;
      }
    } catch {
      playingRef.current = false;
      pendingAudioRef.current = false;
    }
  }, [setStateAndEmit, flushNavIfIdle, sendJson]);

  // Halt playback and return to ready WITHOUT touching the socket — the
  // response to a spoken "stop": the daemon already ended (or never had) the
  // turn, so unlike barge-in there is nothing to cancel via reconnect. Any
  // deferred navigation is dropped: the user just asked for the turn to end.
  const haltPlayback = useCallback(() => {
    finishReplyRef.current?.('halt');
    clearPlayback();
    pendingSegMetaRef.current = [];
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      setStateAndEmit('ready');
    }
  }, [setStateAndEmit]);

  // --- WebSocket message handler ---
  const handleWsMessage = useCallback((event: MessageEvent) => {
    // Any inbound message proves the daemon is alive — reset the watchdog.
    lastActivityRef.current = Date.now();
    if (typeof event.data === 'string') {
      try {
        const msg = JSON.parse(event.data);
        // After an in-band interrupt the old reply's tail can still be in
        // flight; the server's `interrupted` marks the cut. Drop it until then.
        if (awaitingInterruptAckRef.current
          && ['reply_start', 'audio_segment', 'reply_text', 'reply_end', 'navigate'].includes(msg.type)) {
          return;
        }
        switch (msg.type) {
          case 'audio_segment': {
            const sid = Number(msg.segment_id);
            pendingSegMetaRef.current.push({
              segmentId: Number.isFinite(sid) ? sid : pendingSegMetaRef.current.length,
              text: typeof msg.text === 'string' ? msg.text : '',
              words: parseWordTimings(msg.word_timings),
              durationMs: Number(msg.duration_ms) || 0,
            });
            break;
          }
          case 'interrupted':
            awaitingInterruptAckRef.current = false;
            if (interruptTimerRef.current) {
              clearTimeout(interruptTimerRef.current);
              interruptTimerRef.current = null;
            }
            pendingSegMetaRef.current = [];
            if (stateRef.current !== 'recording') setStateAndEmit('ready');
            break;
          case 'ready':
            setStateAndEmit('ready');
            if (!voiceHelloSentRef.current && wsRef.current?.readyState === WebSocket.OPEN) {
              surfaceRef.current = currentVoiceSurface();
              wsRef.current.send(JSON.stringify(buildHello({
                surface: surfaceRef.current,
                build: clientBuild(),
                wireSampleRate: sampleRate,
                revision: 1,
              })));
              voiceHelloSentRef.current = true;
            }
            readyResolveRef.current?.();
            readyResolveRef.current = null;
            readyRejectRef.current = null;
            // Fresh handler on the daemon side — (re)request wake listening if
            // hands-free wants it (covers first connect AND every reconnect).
            syncWakeModeRef.current?.();
            break;
          case 'wake_status': {
            wakeStartPendingRef.current = false;
            const active = !!msg.active;
            wakeActiveRef.current = active;
            setWakeActive(active);
            setWakePhrase(msg.phrase ?? null);
            if (active) {
              // The daemon sends this on EVERY socket (re)connect, and a
              // barge-in interrupt tears the socket down (V3 forensics). The
              // deaf verdict is about this mic feeding this spotter model, not
              // about one socket, so it is KEPT for the rest of the activation:
              // clearing it here re-trapped a user already judged unheard
              // behind the gate for another 15s and 3 ignored attempts after
              // every reconnect. It ends only on deactivate(), a wake hit, the
              // wake word being switched off, or the input device changing.
              wakeSpotterFiredRef.current = false;
              wakeGatedSinceRef.current = 0;
              wakeVoicedEpisodesRef.current = 0;
              // Arm the gate unless a conversation was just interrupted (e.g.
              // a barge-in reconnect): mid-conversation the user expects to
              // keep talking without re-invoking the wake phrase.
              const engaged = Date.now() - lastConvActivityRef.current < WAKE_REARM_MS;
              // A deaf verdict is a hint, never a reason to disarm the gate.
              const gate = !engaged;
              wakeGatedRef.current = gate;
              setWakeGated(gate);
            } else if (msg.reason === 'downloading') {
              // The daemon is fetching the KWS model; ask again shortly.
              if (wakeRetryRef.current) clearTimeout(wakeRetryRef.current);
              wakeRetryRef.current = setTimeout(() => {
                wakeRetryRef.current = null;
                syncWakeModeRef.current?.();
              }, 8000);
            }
            break;
          }
          case 'wake': {
            // Any detection proves the spotter can hear — it is not deaf, and
            // from here on the gate behaves normally for this session.
            wakeSpotterFiredRef.current = true;
            if (wakeDeafRef.current) {
              wakeDeafRef.current = false;
              setWakeDeaf(false);
            }
            const action = routeWakeEvent(msg.kind ?? '', stateRef.current);
            if (action === 'start-turn') {
              wakeGatedRef.current = false;
              setWakeGated(false);
              lastConvActivityRef.current = Date.now();
              // FIX 2: startRecording itself arms the VAD turn clocks now —
              // a single seam every recording start goes through, so there
              // is nothing to duplicate here.
              startRecordingRef.current?.('wake');
            } else if (action === 'halt-playback') {
              haltPlayback();
            }
            break;
          }
          case 'stopped':
            // Spoken stop cancelled the in-flight turn server-side — no more
            // audio is coming; drop whatever is still queued locally.
            haltPlayback();
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
            // This turn is being spoken by the voice pipeline — record it so a
            // later SSE replay (fresh window, reconnect) doesn't synthesize
            // and speak it a second time via speak-replies.
            markReplySpoken(sessionId ?? null, msg.text ?? '');
            emit({ type: 'reply_text', text: msg.text ?? '' });
            break;
          case 'reply_start':
            replyAckRef.current = { segments: 0, heardMs: 0, queuedMs: 0 };
            pendingSegMetaRef.current = [];
            setStateAndEmit('playing');
            break;
          case 'reply_end':
            pendingAudioRef.current = false;
            if (!playingRef.current && audioQueueRef.current.length === 0) {
              finishReplyRef.current?.('drained');
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
          case 'clipboard':
            // Copy immediately — unlike navigate, the pasteboard should be
            // ready before they switch to Notes, even while confirmation audio
            // is still playing. The hub now sends this as soon as the tool
            // runs; still report failure instead of pretending.
            void copyText(msg.text ?? '').then((ok) => {
              if (!ok) console.warn('[useVoice] clipboard write failed');
            });
            break;
          case 'idle':
            setError(null);
            setStateAndEmit('ready');
            break;
          case 'error':
            if (isTransientVoiceIdle(msg.message)) {
              setError(null);
              setStateAndEmit('ready');
              break;
            }
            serverErrorRef.current = msg.message ?? null;
            setError(msg.message ?? 'Unknown voice error');
            emit({ type: 'error', error: msg.message ?? 'Unknown voice error' });
            setStateAndEmit('error');
            setTimeout(() => {
              if (wsRef.current?.readyState === WebSocket.OPEN) {
                // Recovering on a live socket: also clear the error string so a
                // transient server error doesn't linger as a sticky red label
                // over the recovered "Hold to talk" state.
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
        if (awaitingInterruptAckRef.current) return;
        const samples = new Float32Array(buf);
        const segMeta = pendingSegMetaRef.current.shift();
        if (segMeta) segMetaRef.current.set(samples, segMeta);
        replyAckRef.current.queuedMs += (samples.length / 24000) * 1000;
        pendingAudioRef.current = true;
        setStateAndEmit('playing');
        audioQueueRef.current.push(samples);
        playNextChunk();
        emit({ type: 'reply_audio', audio: { samples, sampleRate: 24000 } });
      }
    }
  }, [setStateAndEmit, emit, playNextChunk, flushNavIfIdle, haltPlayback, sessionId, sampleRate, sendJson]);

  // Monotonic connect epoch. Each connectSocket call claims a new epoch; a
  // call that gets superseded while awaiting the daemon token (e.g. the
  // session switched and a rebind connect started) aborts instead of resuming
  // and binding a socket to a stale session URL after the newer one.
  const connectEpochRef = useRef(0);

  // --- Core connect ---
  const connectSocket = useCallback(async (): Promise<void> => {
    const epoch = ++connectEpochRef.current;
    // V-31: the old socket must be fully closed before the new one opens, or
    // the daemon sees two live epochs. Detach its handlers first so a late
    // frame cannot reach this hook, then wait (bounded) for the close.
    const stale = wsRef.current;
    if (stale) {
      stale.onclose = null;
      stale.onmessage = null;
      wsRef.current = null;
      const canWait = stale.readyState !== WebSocket.CLOSED && typeof stale.addEventListener === 'function';
      if (canWait) {
        await new Promise<void>(resolve => {
          const done = () => { clearTimeout(timer); resolve(); };
          const timer = setTimeout(resolve, 400);
          stale.addEventListener('close', done, { once: true });
          try { stale.close(); } catch { done(); }
        });
      } else {
        try { stale.close(); } catch { /* already closed */ }
      }
      if (epoch !== connectEpochRef.current) throw new Error('connect superseded by a newer connect');
    }
    awaitingInterruptAckRef.current = false;
    pendingSegMetaRef.current = [];
    voiceHelloSentRef.current = false;
    // A new socket means a new daemon handler with no wake session; the
    // 'ready' handler re-requests it once the connection is up.
    wakeActiveRef.current = false;
    wakeStartPendingRef.current = false;
    setWakeActive(false);

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
    params.set('client', 'desktop_voice');
    if (token) params.set('token', token);
    const url = `${base}/voice?${params}`;

    return new Promise<void>((resolve, reject) => {
      readyResolveRef.current = resolve;
      readyRejectRef.current = reject;

      serverErrorRef.current = null;
      const ws = new WebSocket(url);
      ws.binaryType = 'arraybuffer';
      wsRef.current = ws;

      ws.onmessage = handleWsMessage;
      ws.onerror = () => {};

      ws.onclose = (ev) => {
        const detail = `code=${ev.code} reason=${ev.reason || '(none)'} wasClean=${ev.wasClean}`;
        console.log(`[useVoice] ws.onclose: ${detail}`);
        wsRef.current = null;
        voiceHelloSentRef.current = false;

        if (readyRejectRef.current) {
          readyRejectRef.current(new Error(`WebSocket closed: ${detail}`));
          readyResolveRef.current = null;
          readyRejectRef.current = null;
        }

        if (ev.code !== 1000 && ev.code !== 1005) {
          let msg = 'Voice connection lost';
          if (ev.code === 1006) msg = `Connection refused (${token ? 'token sent' : 'NO TOKEN'})`;
          // The daemon states the real cause in an `error` frame and then closes
          // (models not loaded, no provider, …). That close lands as a bare 1006,
          // so the label above would replace a precise diagnosis with a network
          // red herring — "Connection refused" sent us hunting ports for a
          // missing-model problem. Server truth outranks the generic label.
          if (serverErrorRef.current) msg = serverErrorRef.current;
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
    finishReplyRef.current?.('halt');
    clearPlayback();
    pendingSegMetaRef.current = [];
    awaitingInterruptAckRef.current = false;
    // Tear down playback so a half-played buffer can't hold the busy state.
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    lastActivityRef.current = Date.now();
    // A fresh socket starts a clean capture streak — otherwise a
    // watchdog-triggered recovery would find the same stale streak still
    // over the limit and immediately re-trigger.
    zeroByteCaptureStreakRef.current = 0;

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
  const interrupt = useCallback((arg?: unknown) => {
    if (!isInterruptibleState(stateRef.current)) return;
    const given = (arg && typeof arg === 'object' && 'reason' in (arg as object)) ? (arg as {
      reason: InterruptReason; micRms?: number; playbackPeak?: number; route?: 'speaker' | 'headphones';
    }) : null;
    const reason: InterruptReason = given?.reason ?? 'user_tap';
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      forceRecover('user barge-in — no live socket');
      return;
    }
    // V-29: in-band. Silence locally first, then ask the daemon to cancel the
    // stream and clear its queue on THIS socket. If it never acknowledges
    // (older daemon), fall back to the old socket reset.
    const heardMs = replyAckRef.current.heardMs
      + (currentSegRef.current ? Math.min(currentSegRef.current.durationMs, performance.now() - currentSegRef.current.startedPerf) : 0);
    finishReplyRef.current?.('interrupt');
    clearPlayback();
    pendingSegMetaRef.current = [];
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    lastActivityRef.current = Date.now();
    awaitingInterruptAckRef.current = true;
    ws.send(JSON.stringify(interruptMsg({
      turnId: replyTurnIdRef.current,
      reason,
      heardMs,
      micRms: given?.micRms,
      playbackPeak: given?.playbackPeak,
      route: given?.route,
      clientMs: Date.now(),
    })));
    setStateAndEmit('ready');
    if (interruptTimerRef.current) clearTimeout(interruptTimerRef.current);
    interruptTimerRef.current = setTimeout(() => {
      interruptTimerRef.current = null;
      if (awaitingInterruptAckRef.current) {
        awaitingInterruptAckRef.current = false;
        forceRecover('interrupt not acknowledged — resetting socket');
      }
    }, INTERRUPT_ACK_TIMEOUT_MS);
  }, [forceRecover, setStateAndEmit]);

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
          consecutiveZeroByteCaptures: zeroByteCaptureStreakRef.current,
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
  const monotonicNowMs = useCallback((): number => {
    if (typeof performance !== 'undefined' && typeof performance.now === 'function') {
      return performance.now();
    }
    return Date.now();
  }, []);

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

  /**
   * Open the mic with the VAD-friendly constraints and watch its tracks. Used
   * at activation AND by the mic watchdog's rebuild, so a re-acquired stream
   * (new default device after a Bluetooth profile flip) is configured and
   * watched exactly like the first one.
   */
  const acquireMicStream = useCallback(async (): Promise<MediaStream> => {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        sampleRate,
        channelCount: 1,
        echoCancellation: true,
        // Browser-level noise suppression — attenuates broadband
        // non-speech (keyboard clatter, fans) before the VAD ever sees it.
        noiseSuppression: true,
        // AGC is deliberately OFF. It renormalizes toward a target level,
        // so during silence it AMPLIFIES the noise floor — pushing room
        // tone and key clicks up over our fixed RMS thresholds. It works
        // directly against a fixed-threshold VAD.
        autoGainControl: false,
      },
    });
    // Track listeners: nothing used to notice the device going away or the OS
    // muting it (the "stuck on Listening" report). Each handler only records
    // what happened; the watchdog tick owns every rebuild decision, so a
    // storm of events can never stack rebuilds. Handlers for a stream that has
    // since been replaced are ignored.
    for (const track of stream.getTracks()) {
      track.onended = () => {
        if (mediaStreamRef.current !== stream) return;
        console.warn('[useVoice] mic track ended — re-acquiring the input');
        micReacquirePendingRef.current = true;
      };
      track.onmute = () => {
        if (mediaStreamRef.current !== stream) return;
        console.warn('[useVoice] mic track muted by the system');
        if (!micMutedSinceRef.current) micMutedSinceRef.current = Date.now();
      };
      track.onunmute = () => {
        if (mediaStreamRef.current !== stream) return;
        micMutedSinceRef.current = 0;
      };
    }
    return stream;
  }, [sampleRate]);

  /** Activate voice: acquire mic + connect socket. */
  const activate = useCallback(async () => {
    activeRef.current = true;

    // Acquire mic ONCE at activation (persistent until deactivate).
    // This makes startRecording synchronous — no getUserMedia race.
    if (!mediaStreamRef.current) {
      try {
        const stream = await acquireMicStream();
        mediaStreamRef.current = stream;
        micMutedSinceRef.current = 0;
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
  }, [acquireMicStream, ensureReady, setStateAndEmit]);

  /** Deactivate voice: release mic + disconnect socket. */
  const deactivate = useCallback(() => {
    activeRef.current = false;
    if (stateRef.current === 'recording') {
      stopRecordingRef.current?.('interruption');
    } else {
      // Stop recording
      processorRef.current?.disconnect();
      processorRef.current = null;
      audioCtxRef.current?.close().catch(() => {});
      audioCtxRef.current = null;
    }
    // Release mic
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    // Stop playback
    finishReplyRef.current?.('halt');
    clearPlayback();
    pendingSegMetaRef.current = [];
    playingRef.current = false;
    audioQueueRef.current = [];
    pendingAudioRef.current = false;
    pendingNavRef.current = null;
    playbackCtxRef.current?.close().catch(() => {});
    playbackCtxRef.current = null;
    analyserRef.current = null;
    // Wake listening dies with the socket
    if (wakeRetryRef.current) {
      clearTimeout(wakeRetryRef.current);
      wakeRetryRef.current = null;
    }
    wakeActiveRef.current = false;
    wakeStartPendingRef.current = false;
    setWakeActive(false);
    wakeGatedRef.current = false;
    setWakeGated(false);
    wakeDeafRef.current = false;
    setWakeDeaf(false);
    wakeSpotterFiredRef.current = false;
    wakeGatedSinceRef.current = 0;
    wakeVoicedEpisodesRef.current = 0;
    // The mic watchdog's bookkeeping belongs to this activation only, and its
    // fault text must not outlive it on the mic button.
    if (micFaultSurfacedRef.current) setError(null);
    micRebuildAttemptsRef.current = 0;
    micFaultSurfacedRef.current = false;
    micReacquirePendingRef.current = false;
    micMutedSinceRef.current = 0;
    // Close socket
    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.close();
      wsRef.current = null;
    }
    setStateAndEmit('idle');
  }, [setStateAndEmit]);

  /**
   * Start recording. SYNCHRONOUS — no async gaps, no press-release race.
   * Only works when socket is open, state is 'ready', and mic is acquired.
   *
   * FIX 2 (2026-09-01 incident, single seam): every recording start — push-
   * to-talk, wake word, VAD onset, and VAD onset — arms the
   * VAD turn clocks here, unconditionally. Three teach-driven call sites
   * used to call this with the clocks never (re)armed, leaving
   * vadTurnStartRef holding a value up to ~100s stale from a PRIOR turn;
   * shouldEndVadTurn's very first check (`now - turnStartedAt > 45000ms`)
   * then stopped the recording before its 256ms ScriptProcessor emitted a
   * single buffer — 0-sample recordings. This is the exact bug class
   * beginVadTurn's own docstring was written to close, now closed at the
   * one place every start goes through instead of duplicated (and
   * incompletely applied) at each call site.
   */
  const startRecording = useCallback((trigger: VoiceTrigger = 'user_tap') => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (stateRef.current !== 'ready') return;

    const stream = mediaStreamRef.current;
    if (!stream) {
      console.warn('[useVoice] startRecording: no mic stream (not activated?)');
      return;
    }

    const now = Date.now();
    recordingStartedAtRef.current = now;
    const timing = beginVadTurn(now, false);
    vadHeardSpeechRef.current = timing.heardSpeech;
    vadLastVoiceRef.current = timing.lastVoiceAt;
    vadTurnStartRef.current = timing.turnStartedAt;

    const audioCtx = new AudioContext({ sampleRate });
    audioCtxRef.current = audioCtx;
    const source = audioCtx.createMediaStreamSource(stream);
    const processor = audioCtx.createScriptProcessor(4096, 1, 1);
    processorRef.current = processor;
    frameCountRef.current = 0;

    turnSequenceRef.current += 1;
    const turnId = `web-${turnSequenceRef.current}`;
    activeTurnIdRef.current = turnId;
    activeTurnTriggerRef.current = trigger;
    vadVoicedMsRef.current = trigger === 'vad' || trigger === 'barge' ? 256 : 0;
    vadPeakRef.current = 0;
    replyTurnIdRef.current = turnId;
    // The surface can change between views mid-session; every turn restates it.
    surfaceRef.current = currentVoiceSurface();
    activeTurnAudioRevisionRef.current = 1;
    activeTurnStartedAtMonotonicRef.current = monotonicNowMs();
    ws.send(JSON.stringify({
      type: 'start',
      sample_rate: sampleRate,
      turn_id: turnId,
      trigger,
      surface: surfaceRef.current,
      audio_revision: activeTurnAudioRevisionRef.current,
    }));
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
  }, [monotonicNowMs, sampleRate, setStateAndEmit]);

  /** Stop recording and send audio for processing. */
  const stopRecording = useCallback((endReason: VoiceTurnEndReason = 'user_tap') => {
    processorRef.current?.disconnect();
    processorRef.current = null;
    audioCtxRef.current?.close().catch(() => {});
    audioCtxRef.current = null;
    // Do NOT stop mediaStream tracks — mic stays alive for next recording
    pendingAudioRef.current = false;

    const frames = frameCountRef.current;
    frameCountRef.current = 0;
    console.log(`[useVoice] stopRecording: sent ${frames} frames (${(frames * 4096 / sampleRate).toFixed(1)}s)`);
    // ALSO (backstops fix 3a): track consecutive captures that sent NOTHING —
    // the watchdog (isVoiceWedged) catches a run of these regardless of
    // state or how much OTHER frame traffic (VAD monitor frames, JSON
    // control messages) is flowing, which is what let the 2026-09-01
    // incident's fast start/stop loop look alive.
    if (frames === 0) {
      zeroByteCaptureStreakRef.current += 1;
    } else {
      zeroByteCaptureStreakRef.current = 0;
    }

    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN && stateRef.current === 'recording') {
      const startedAtMonotonic = activeTurnStartedAtMonotonicRef.current;
      const captureDurationMs = startedAtMonotonic === null
        ? null
        : Math.max(0, Math.round(monotonicNowMs() - startedAtMonotonic));
      const stopPayload: Record<string, unknown> = {
        type: 'stop',
        turn_id: activeTurnIdRef.current ?? undefined,
        audio_revision: activeTurnAudioRevisionRef.current,
        trigger: activeTurnTriggerRef.current,
        end_reason: endReason,
        vad_noise_floor_rms: null,
        vad_onset_rms: null,
        vad_keepalive_rms: null,
        vad_committed: null,
        vad_voiced_ms: activeTurnTriggerRef.current === 'vad' || activeTurnTriggerRef.current === 'barge'
          ? Math.round(vadVoicedMsRef.current)
          : null,
        vad_trailing_silence_ms: null,
        vad_silence_window_ms: null,
        vad_max_turn_ms: null,
        vad_max_keepalive_gap_ms: null,
      };
      if (captureDurationMs !== null) {
        stopPayload.capture_duration_ms = captureDurationMs;
      }
      if (endReason === 'noise_abort') stopPayload.client_discard = true;
      stopPayload.surface = surfaceRef.current;
      ws.send(JSON.stringify(stopPayload));
      // Start the watchdog clock fresh as we enter the busy phase.
      lastActivityRef.current = Date.now();
      setStateAndEmit('processing');
    }
    activeTurnStartedAtMonotonicRef.current = null;
    activeTurnIdRef.current = null;
    activeTurnTriggerRef.current = 'unknown';
  }, [monotonicNowMs, sampleRate, setStateAndEmit]);

  // Reconcile daemon-side wake listening with what the client wants
  // (hands-free on + preference enabled). Safe to call any time; no-ops
  // without an open socket. Late-bound via syncWakeModeRef for the message
  // handler and setters declared earlier.
  const syncWakeMode = useCallback(() => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const want = handsFreeRef.current && wakeEnabledRef.current;
    if (want && !wakeActiveRef.current && !wakeStartPendingRef.current) {
      wakeStartPendingRef.current = true;
      ws.send(JSON.stringify({ type: 'wake_start', sample_rate: sampleRate }));
    } else if (!want && (wakeActiveRef.current || wakeStartPendingRef.current)) {
      ws.send(JSON.stringify({ type: 'wake_stop' }));
      wakeActiveRef.current = false;
      wakeStartPendingRef.current = false;
      setWakeActive(false);
      wakeGatedRef.current = false;
      setWakeGated(false);
      // Wake word switched off: there is no spotter left to be deaf, and the
      // orb must not keep saying the wake word is not being detected.
      wakeDeafRef.current = false;
      setWakeDeaf(false);
    }
  }, [sampleRate]);
  syncWakeModeRef.current = syncWakeMode;

  /** Enable/disable the wake word (persisted; applies immediately). */
  const setWakeWordEnabled = useCallback((on: boolean) => {
    wakeEnabledRef.current = on;
    setWakeWordEnabledState(on);
    try {
      localStorage.setItem(WAKE_WORD_LS_KEY, on ? '1' : '0');
    } catch {
      // Preference simply won't persist.
    }
    syncWakeModeRef.current?.();
  }, []);

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
  const vadOnsetStreakRef = useRef(0);
  const vadTurnStartRef = useRef(0);
  // FIX 2 hardening: when the CURRENT recording actually began, set
  // unconditionally by startRecording. Passed into shouldEndVadTurn as a
  // floor under vadTurnStartRef — see that function's docstring.
  const recordingStartedAtRef = useRef(0);

  // Mic watchdog state (see diagnoseMicInput / micWatchdogStep).
  // Last time the VAD processor handed us a buffer — the liveness signal.
  const vadLastBufferAtRef = useRef(0);
  // Last time a buffer carried any signal (peak above MIC_SILENCE_PEAK).
  const vadLastSignalAtRef = useRef(0);
  // When the watchdog began judging the current stretch (0 = not judging).
  const micJudgingSinceRef = useRef(0);
  // Last time a VAD graph build was ATTEMPTED (first build or rebuild).
  const micGraphBuiltAtRef = useRef(0);
  // Rebuilds since the input was last healthy.
  const micRebuildAttemptsRef = useRef(0);
  // The orb is currently showing the mic fault (state 'error' is ours).
  const micFaultSurfacedRef = useRef(false);
  const micRebuildInFlightRef = useRef(false);
  // A device change / ended track asked for a fresh getUserMedia.
  const micReacquirePendingRef = useRef(false);
  // When the current track went muted (0 = not muted).
  const micMutedSinceRef = useRef(0);
  // The graph (by build time) whose healthy verdict is already in the trail.
  const micLiveLoggedForRef = useRef(0);

  // A mechanical keyboard's click is a loud, very SHORT transient. Speech is
  // quieter per-buffer but sustained — so the discriminator that works is
  // duration, not level. Onset must hold across several consecutive buffers.
  const VAD_KEEPALIVE = VAD_KEEPALIVE_RMS;
  /** Consecutive ~128ms buffers over onset before a turn starts. A mechanical
   *  keyboard click is a single-buffer transient; speech sustains — this was
   *  tripping recording on every keystroke (empty-transcript chatter in the
   *  daemon log). */
  // Two buffers (~256ms), not three. Every buffer in the streak must ALSO pass
  // the spectral gate, and natural speech is full of fricatives and plosives
  // whose buffers are legitimately bright — a long all-must-pass run is easy to
  // stall on. See the decay below: a single failing buffer no longer wipes the
  // streak, so onset survives a consonant without dropping the word.
  const VAD_ONSET_STREAK = 2;

  const stopVadMonitor = useCallback(() => {
    vadProcRef.current?.disconnect();
    vadProcRef.current = null;
    vadCtxRef.current?.close().catch(() => {});
    vadCtxRef.current = null;
    micAnalyserRef.current = null;
    // The onset streak is hook-lifetime state, so a half-formed streak (one
    // loud buffer) would survive a monitor teardown and let the NEXT single
    // keyboard click complete it — defeating the debounce entirely.
    vadOnsetStreakRef.current = 0;
    vadBargeStreakRef.current = 0;
    bargeGuardRef.current.resetAll();
  }, []);

  /** True when the current mic spectrum looks like voice rather than a
   *  broadband transient. Fails OPEN when no analyser exists, so voice never
   *  becomes undetectable. The judgement itself is pure and unit-tested in
   *  vadSpectrum.ts — it was previously inline, and wrong in a way that
   *  rejected all speech. */
  const vadSpectrumRef = useRef<Uint8Array<ArrayBuffer> | null>(null);
  const looksLikeVoice = useCallback(() => {
    const analyser = micAnalyserRef.current;
    if (!analyser) return true;
    if (!vadSpectrumRef.current) {
      vadSpectrumRef.current = new Uint8Array(analyser.frequencyBinCount);
    }
    const data = vadSpectrumRef.current;
    analyser.getByteFrequencyData(data);
    return spectrumLooksLikeVoice(data);
  }, []);

  const startVadMonitor = useCallback(() => {
    const stream = mediaStreamRef.current;
    if (!stream || vadCtxRef.current) return;
    const ctx = new AudioContext({ sampleRate });
    vadCtxRef.current = ctx;
    micGraphBuiltAtRef.current = Date.now();
    // A Bluetooth profile flip or sleep/wake SUSPENDS this context and the
    // processor below silently stops firing — the "stuck on Listening"
    // report. Resume on the spot; if that does not bring buffers back, the
    // watchdog rebuilds the graph. Events from a context we already replaced
    // are ignored (closing it on teardown fires one too). Property-style
    // handler + optional resume: WebKit also reports a non-standard
    // 'interrupted' state, and not every AudioContext implementation has
    // both members.
    const tryResume = () => {
      if (typeof ctx.resume === 'function') ctx.resume().catch(() => {});
    };
    ctx.onstatechange = () => {
      if (vadCtxRef.current !== ctx) return;
      const st = ctx.state as string;
      if (st === 'suspended' || st === 'interrupted') {
        console.warn(`[useVoice] VAD audio context ${st} — resuming`);
        tryResume();
      }
    };
    if (ctx.state === 'suspended') tryResume();
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
      // Liveness for the mic watchdog — stamped before anything else, since
      // "the processor fired at all" is exactly what it needs to know.
      vadLastBufferAtRef.current = Date.now();
      const input = e.inputBuffer.getChannelData(0);
      // ...and whether it carried anything: an HFP link renegotiating keeps
      // the processor firing with all-zero buffers, which "fired at all" alone
      // would call a healthy mic.
      if (bufferPeak(input) > MIC_SILENCE_PEAK) vadLastSignalAtRef.current = vadLastBufferAtRef.current;
      if (!handsFreeRef.current) return;
      let sum = 0;
      for (let i = 0; i < input.length; i++) sum += input[i] * input[i];
      const rms = Math.sqrt(sum / input.length);
      const now = Date.now();
      const s = stateRef.current;

      // Wake mode: stream monitor frames to the daemon's on-device keyword
      // spotter whenever the recording processor isn't already sending audio.
      // These frames feed ONLY the spotter — never STT, never the LLM.
      if (wakeActiveRef.current && s !== 'recording') {
        const sock = wsRef.current;
        if (sock && sock.readyState === WebSocket.OPEN) {
          const buffer = new ArrayBuffer(input.length * 4);
          new Float32Array(buffer).set(input);
          sock.send(buffer);
        }
      }
      // Conversation-liveness clock: any non-ready state means turns are
      // flowing. Sitting in ready past the window re-arms the wake gate, so
      // the NEXT turn needs the wake phrase again.
      if (s !== 'ready') {
        lastConvActivityRef.current = now;
      } else if (
        wakeActiveRef.current &&
        !wakeGatedRef.current &&
        now - lastConvActivityRef.current > WAKE_REARM_MS
      ) {
        wakeGatedRef.current = true;
        setWakeGated(true);
      }

      if (s === 'ready') {
        vadBargeStreakRef.current = 0;
        // While the wake gate is armed, VAD onset must NOT open a turn — the
        // wake phrase (detected daemon-side) is the only way in...
        if (wakeActiveRef.current && wakeGatedRef.current) {
          vadOnsetStreakRef.current = 0;
          // ...unless the spotter is deaf. Count voiced episodes it ignored;
          // the bar is the keepalive level, not onset, because the reported
          // case was a LOW-level HFP mic. One episode = a sustained voiced
          // streak, then quiet again before the next can count.
          if (!wakeGatedSinceRef.current) wakeGatedSinceRef.current = now;
          if (rms > VAD_KEEPALIVE && looksLikeVoice()) {
            wakeEpisodeStreakRef.current += 1;
            if (wakeEpisodeStreakRef.current >= VAD_ONSET_STREAK && !wakeEpisodeLatchRef.current) {
              wakeEpisodeLatchRef.current = true;
              wakeVoicedEpisodesRef.current += 1;
            }
          } else {
            wakeEpisodeStreakRef.current = Math.max(0, wakeEpisodeStreakRef.current - 1);
            if (wakeEpisodeStreakRef.current === 0) wakeEpisodeLatchRef.current = false;
          }
          if (wakeSpotterLooksDeaf({
            gatedForMs: now - wakeGatedSinceRef.current,
            voicedEpisodes: wakeVoicedEpisodesRef.current,
            spotterHasFired: wakeSpotterFiredRef.current,
          })) {
            // Say so — but do NOT open the gate. See WAKE_DEAF_HINT for why
            // failing open was withdrawn: ambient, unaddressed speech trips
            // this verdict just as a deaf spotter does.
            if (!wakeDeafRef.current) {
              console.warn(
                `[useVoice] wake spotter silent through ${wakeVoicedEpisodesRef.current} voiced ` +
                'episodes — showing the hint; the wake gate stays closed',
              );
              recordVoiceVerdictEvent('wake_spotter_looks_deaf', `${wakeVoicedEpisodesRef.current} voiced episodes ignored`);
              wakeDeafRef.current = true;
              setWakeDeaf(true);
            }
          }
          return;
        }
        wakeGatedSinceRef.current = 0;
        wakeVoicedEpisodesRef.current = 0;
        wakeEpisodeStreakRef.current = 0;
        wakeEpisodeLatchRef.current = false;
        // Sustained-onset gate: one loud buffer (a key click) is ignored;
        // real speech clears the streak in ~250ms — imperceptible latency,
        // and the recording still captures the utterance from its 2nd buffer.
        if (rms > VAD_ONSET_RMS && looksLikeVoice()) {
          vadOnsetStreakRef.current += 1;
          if (vadOnsetStreakRef.current >= VAD_ONSET_STREAK) {
            vadOnsetStreakRef.current = 0;
            // FIX 2: startRecording itself arms the VAD turn clocks now —
            // single seam, nothing to duplicate here.
            startRecordingRef.current?.(Date.now() - lastBargeAtRef.current < 4000 ? 'barge' : 'vad');
          }
        } else {
          // DECAY, don't reset. A hard reset meant one legitimately bright
          // buffer mid-onset (a fricative or plosive opening a word) threw away
          // the whole streak, so speech starting on "s", "t" or "wh" could
          // stall indefinitely. Decay still starves an isolated key click,
          // which produces a single loud buffer surrounded by quiet ones.
          vadOnsetStreakRef.current = Math.max(0, vadOnsetStreakRef.current - 1);
        }
      } else if (s === 'recording') {
        if (rms > VAD_KEEPALIVE) {
          vadHeardSpeechRef.current = true;
          vadLastVoiceRef.current = now;
          vadVoicedMsRef.current += (input.length / sampleRate) * 1000;
        }
        vadPeakRef.current = Math.max(vadPeakRef.current, bufferPeak(input));
        let endReason = classifyVadTurnEnd({
          heardSpeech: vadHeardSpeechRef.current,
          lastVoiceAt: vadLastVoiceRef.current,
          turnStartedAt: vadTurnStartRef.current,
        }, now, { recordingStartedAt: recordingStartedAtRef.current });
        // V-18: a knock, click or TTS bleed is not a turn.
        if (endReason === 'silence' && isClippedTransient({
          trigger: activeTurnTriggerRef.current,
          voicedMs: vadVoicedMsRef.current,
          peak: vadPeakRef.current,
        })) {
          console.info(`[useVoice] voice_transient_discarded voiced_ms=${Math.round(vadVoicedMsRef.current)} peak=${vadPeakRef.current.toFixed(2)}`);
          endReason = 'noise_abort';
        }
        if (endReason) {
          vadHeardSpeechRef.current = false;
          stopRecordingRef.current?.(endReason);
        }
      } else if (s === 'playing' || s === 'processing') {
        // Barge-in. D7: stays on for Mac speakers, behind an echo guard that
        // mirrors the iPhone fix (39ea442b): size the echo floor from the
        // speaker's 1.5 s peak and need three confirmations within 1.2 s.
        const guard = bargeGuardRef.current;
        const nowP = performance.now();
        guard.notePlayback(nowP, rawPlaybackRms(nowP));
        if (s === 'playing') {
          const speakerRoute = speakerRouteRef.current;
          const verdict = guard.onMicBuffer({
            nowMs: nowP,
            micRms: rms,
            voiceLike: looksLikeVoice(),
            speakerRoute,
          });
          if (verdict === 'pending' && speakerRoute) {
            console.info(`[useVoice] voice_barge_pending route=speaker mic_rms=${rms.toFixed(3)} playback_peak=${guard.peakNow(nowP).toFixed(3)}`);
          } else if (verdict === 'confirmed') {
            console.info(`[useVoice] voice_barge_confirmed route=${speakerRoute ? 'speaker' : 'headphones'} mic_rms=${rms.toFixed(3)} playback_peak=${guard.peakNow(nowP).toFixed(3)}`);
            lastBargeAtRef.current = Date.now();
            vadHeardSpeechRef.current = false;
            interruptRef.current?.({
              reason: 'barge_in',
              micRms: rms,
              playbackPeak: guard.peakNow(nowP),
              route: speakerRoute ? 'speaker' : 'headphones',
            });
          }
        } else {
          guard.reset();
        }
      }
    };
    source.connect(proc);
    proc.connect(ctx.destination);
  }, [sampleRate]);

  /** Enter/exit hands-free. Entering activates voice (mic + socket) if needed. */
  const setHandsFree = useCallback(async (on: boolean) => {
    const handsFreeWasOn = handsFreeRef.current;
    handsFreeRef.current = on;
    setHandsFreeState(on);
    if (on) {
      if (!activeRef.current) await activate();
      startVadMonitor();
      const ctx = vadCtxRef.current;
      recordVoiceVerdictEvent(
        'hands_free_armed',
        `${micInputLabel(mediaStreamRef.current)}; listening graph ${ctx ? ctx.state : 'missing'}`,
      );
    } else {
      if (handsFreeWasOn) recordVoiceVerdictEvent('hands_free_off');
      stopVadMonitor();
      // The mic fault describes the hands-free graph we just tore down; leaving
      // it would strand push-to-talk in 'error' behind a stale message.
      micRebuildAttemptsRef.current = 0;
      micReacquirePendingRef.current = false;
      if (micFaultSurfacedRef.current) {
        micFaultSurfacedRef.current = false;
        setError(null);
        if (stateRef.current === 'error' && wsRef.current?.readyState === WebSocket.OPEN) {
          setStateAndEmit('ready');
        }
      }
      // A VAD-started recording without its monitor would never end — close
      // the turn normally so anything already said still gets processed.
      if (stateRef.current === 'recording') stopRecordingRef.current?.('interruption');
    }
    // Wake listening follows hands-free (on: start spotting if enabled;
    // off: tell the daemon to stop).
    syncWakeModeRef.current?.();
  }, [activate, startVadMonitor, stopVadMonitor, setStateAndEmit]);

  // Late-bound refs so the monitor callback (created once) always calls the
  // freshest versions without re-wiring the audio graph.
  const startRecordingRef = useRef<((trigger?: VoiceTrigger) => void) | null>(null);
  const stopRecordingRef = useRef<((reason?: VoiceTurnEndReason) => void) | null>(null);
  const interruptRef = useRef<((arg?: unknown) => void) | null>(null);

  // ── Mic-input watchdog ───────────────────────────────────────────────────
  // Tear down and rebuild the hands-free listening graph, re-acquiring the
  // mic when asked to (device change, ended track) or when the current track
  // is dead. The new stream is acquired BEFORE the old one is released, so a
  // failed getUserMedia keeps whatever input still existed. Never swaps the
  // stream under a live recording — its processor is reading the old one.
  const rebuildMicGraph = useCallback(async (reason: string, reacquire: boolean) => {
    if (micRebuildInFlightRef.current) return;
    micRebuildInFlightRef.current = true;
    micRebuildAttemptsRef.current += 1;
    micGraphBuiltAtRef.current = Date.now();
    console.warn(`[useVoice] mic watchdog: rebuilding the listening graph — ${reason}`);
    recordVoiceVerdictEvent('mic_rebuild', reason);
    try {
      stopVadMonitor();
      const old = mediaStreamRef.current;
      const oldDead = !old || old.getTracks().every(t => t.readyState === 'ended');
      if ((reacquire || oldDead) && stateRef.current !== 'recording') {
        const fresh = await acquireMicStream();
        if (!activeRef.current) {
          // Voice was turned off while we waited — don't leak a live mic.
          fresh.getTracks().forEach(t => t.stop());
          return;
        }
        mediaStreamRef.current = fresh;
        micMutedSinceRef.current = 0;
        old?.getTracks().forEach(t => t.stop());
      }
      if (!handsFreeRef.current || !activeRef.current) return;
      startVadMonitor();
    } catch (e) {
      console.warn('[useVoice] mic watchdog: rebuild failed:', e);
    } finally {
      // Grace for the new graph counts from when it actually exists.
      micGraphBuiltAtRef.current = Date.now();
      micRebuildInFlightRef.current = false;
    }
  }, [acquireMicStream, startVadMonitor, stopVadMonitor]);

  // One watchdog tick. Reads only refs, so the interval never needs rewiring.
  const micWatchdogTick = useCallback(() => {
    if (!handsFreeRef.current || !activeRef.current) return;
    if (micRebuildInFlightRef.current) return;
    const s = stateRef.current;

    // A device change or ended track asked for a fresh input. Honoured in any
    // state but 'recording' (that capture is still reading the old stream).
    if (micReacquirePendingRef.current && s !== 'recording') {
      micReacquirePendingRef.current = false;
      void rebuildMicGraph('input device changed', true);
      return;
    }

    // Judge only the states where the user is waiting on the mic: armed
    // ('ready', including a wake-gated ready — the gate used to be able to
    // hide a dead input indefinitely), mid-capture, or already showing our
    // own fault. Processing/playing are left alone; a graph that died during
    // a reply is caught the moment the loop returns to 'ready'.
    // `showing`, not just the flag: a reconnect's 'ready' frame can paint over
    // the fault while the input is still dead, and it must go back up.
    const showing = s === 'error' && micFaultSurfacedRef.current;
    if (s !== 'ready' && s !== 'recording' && !showing) {
      micJudgingSinceRef.current = 0;
      return;
    }
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      micJudgingSinceRef.current = 0;
      return;
    }

    const now = Date.now();
    if (!micJudgingSinceRef.current) micJudgingSinceRef.current = now;
    const stream = mediaStreamRef.current;
    const tracks = stream ? stream.getTracks() : [];
    const muted = tracks.some(t => t.muted);
    if (muted) {
      if (!micMutedSinceRef.current) micMutedSinceRef.current = now;
    } else {
      micMutedSinceRef.current = 0;
    }
    const ctx = vadCtxRef.current;
    const lastBuf = vadLastBufferAtRef.current;
    const fault = diagnoseMicInput({
      hasStream: !!stream,
      trackEnded: tracks.length > 0 && tracks.every(t => t.readyState === 'ended'),
      hasGraph: !!ctx && !!vadProcRef.current,
      ctxState: ctx ? ctx.state : null,
      msTrackMuted: micMutedSinceRef.current ? now - micMutedSinceRef.current : 0,
      // A buffer from BEFORE the current graph was built says nothing about it.
      msSinceLastBuffer: lastBuf >= micGraphBuiltAtRef.current && lastBuf > 0
        ? now - lastBuf
        : Number.POSITIVE_INFINITY,
      msSinceSignal: msSinceMicSignal({
        now,
        lastSignalAt: vadLastSignalAtRef.current,
        graphBuiltAt: micGraphBuiltAtRef.current,
        judgingSince: micJudgingSinceRef.current,
      }),
    });
    const lastSignal = vadLastSignalAtRef.current;
    const action = micWatchdogStep({
      fault,
      provenLive: lastSignal > 0 && lastSignal >= micGraphBuiltAtRef.current,
      rebuildAttempts: micRebuildAttemptsRef.current,
      msSinceGraphBuilt: now - micGraphBuiltAtRef.current,
      faultSurfaced: showing,
    });

    if (action === 'healthy') {
      const rebuilt = micRebuildAttemptsRef.current > 0;
      micRebuildAttemptsRef.current = 0;
      const firstOnGraph = micLiveLoggedForRef.current !== micGraphBuiltAtRef.current;
      micLiveLoggedForRef.current = micGraphBuiltAtRef.current;
      if (micFaultSurfacedRef.current) {
        micFaultSurfacedRef.current = false;
        console.log('[useVoice] mic watchdog: input is back');
        recordVoiceVerdictEvent('mic_fault_cleared', micInputLabel(stream));
        if (stateRef.current === 'error') {
          setError(null);
          setStateAndEmit('ready');
        }
      } else if (firstOnGraph) {
        // Once per graph, never per tick: proof the watchdog judged the REAL
        // input, which an empty trail could not give (see VOICE_VERDICT_LOG_KEY).
        recordVoiceVerdictEvent(
          'mic_live',
          `${micInputLabel(stream)}; signal ${now - lastSignal}ms ago${rebuilt ? '; after a rebuild' : ''}`,
        );
      }
      return;
    }
    if (action === 'wait') return;

    // Either remedy means this capture is deaf too — a VAD-started recording
    // can only end from the VAD callback that just stopped firing, so it would
    // otherwise sit on "Listening" forever. Close it normally (whatever was
    // captured still gets processed).
    if (s === 'recording') stopRecordingRef.current?.('client_error');

    if (action === 'rebuild') {
      // A silent input is re-acquired too: a fresh getUserMedia is what gets
      // a renegotiated HFP link (or the new default device) delivering again.
      const dead = fault === 'track-ended' || fault === 'no-stream' || fault === 'track-muted'
        || fault === 'silent';
      void rebuildMicGraph(`input ${fault}`, dead);
      return;
    }
    // 'surface-fault': the rebuilt graph is dead too. Say so instead of
    // "Listening" — only from 'ready' (a stopped recording is 'processing'
    // now; the next tick back in 'ready' surfaces it).
    if (stateRef.current !== 'ready' || !fault) return;
    const message = micFaultMessage(fault);
    console.warn(`[useVoice] mic watchdog: ${fault} after a rebuild — surfacing "${message}"`);
    recordVoiceVerdictEvent('mic_fault_surfaced', `${fault}: ${message}`);
    micFaultSurfacedRef.current = true;
    setError(message);
    emit({ type: 'error', error: message });
    setStateAndEmit('error');
  }, [rebuildMicGraph, setStateAndEmit, emit]);

  // Run the watchdog, and listen for input-device changes, only while
  // hands-free is on — that is when the VAD graph is supposed to be live.
  useEffect(() => {
    if (!handsFree) return;
    const id = setInterval(micWatchdogTick, MIC_WATCHDOG_TICK_MS);
    // A Bluetooth A2DP<->HFP flip or headphones plugged in change the input
    // devices; the old stream may keep pointing at a device that now delivers
    // nothing. Re-acquire (the tick applies it, coalescing bursts of events).
    const md = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    const onDeviceChange = () => {
      console.warn('[useVoice] input devices changed — scheduling a mic re-acquire');
      micReacquirePendingRef.current = true;
      // The deaf verdict was about the OLD input feeding the spotter. A new
      // device (say, leaving the Bose HFP mic for the built-in one) deserves a
      // working wake gate again; the idle re-arm puts the gate back once the
      // conversation goes quiet, and the spotter can be judged afresh.
      if (wakeDeafRef.current) {
        wakeDeafRef.current = false;
        setWakeDeaf(false);
      }
    };
    if (md && typeof md.addEventListener === 'function') {
      md.addEventListener('devicechange', onDeviceChange);
    }
    return () => {
      clearInterval(id);
      if (md && typeof md.removeEventListener === 'function') {
        md.removeEventListener('devicechange', onDeviceChange);
      }
    };
  }, [handsFree, micWatchdogTick]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      activeRef.current = false;
      if (wakeRetryRef.current) clearTimeout(wakeRetryRef.current);
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

  // Relay what the engine decided to mirror orbs (see VOICE_VERDICT_KEY): the
  // popped-out chat window has no engine, so without this its orb said
  // "Listening" over a dead mic's fault and over a failed-open gate. Written on
  // change, cleared when the engine goes away so no window reads a dead one.
  const verdictWakeHint = wakeHintFor({ active: wakeActive, gated: wakeGated, phrase: wakePhrase });
  useEffect(() => {
    publishVoiceVerdict({ error, wakeHint: verdictWakeHint, spotterDeaf: wakeDeaf, handsFree });
  }, [error, verdictWakeHint, wakeDeaf, handsFree]);
  useEffect(() => clearVoiceVerdict, []);
  // Once per mount (see VOICE_VERDICT_SCHEMA): without this line a session that
  // never armed hands-free left an empty trail, indistinguishable from a
  // bundle that predates the trail entirely.
  useEffect(() => { recordEngineUp(handsFreeRef.current); }, []);

  // Stable identity so consumers can use it as an effect dependency without
  // re-firing on unrelated renders.
  const wakeWord = useMemo(
    () => ({
      enabled: wakeWordEnabled,
      setEnabled: setWakeWordEnabled,
      /** Daemon is actively spotting on this socket. */
      active: wakeActive,
      /** Human-readable wake phrase (e.g. "Hey Henry") for UI hints. */
      phrase: wakePhrase,
      /** True when the wake phrase is required to open the next turn. */
      gated: wakeGated,
      /** The spotter ignored repeated speech, so the gate failed open: bare
       *  speech opens turns and the UI should say so (WAKE_DEAF_HINT). */
      spotterDeaf: wakeDeaf,
    }),
    [wakeWordEnabled, setWakeWordEnabled, wakeActive, wakePhrase, wakeGated, wakeDeaf],
  );

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
    /** Wake-word surface: preference, live daemon status, and gate state. */
    wakeWord,
  };
}
