import { describe, it, expect } from 'vitest';
import {
  beginVadTurn,
  endpointWindowMs,
  shouldEndVadTurn,
  VAD_MAX_TURN_MS,
  VAD_QUICK_SILENCE_MS,
  VAD_QUICK_TURN_SPEECH_MS,
  VAD_SILENCE_MS,
  isVoiceWedged,
  isInterruptibleState,
  isTransientVoiceIdle,
  routeWakeEvent,
  VOICE_WATCHDOG_MS,
  VOICE_ZERO_BYTE_CAPTURE_STREAK_LIMIT,
  type VoiceState,
} from './useVoice';

describe('wake-word VAD turn timing', () => {
  it('does not end a fresh wake turn before an audio buffer arrives', () => {
    const detectedAt = 1_000_000;
    const timing = beginVadTurn(detectedAt, false);

    // The live failure stopped at 120 ms, before the 256 ms recording
    // processor emitted its first frame.
    expect(shouldEndVadTurn(timing, detectedAt + 120)).toBe(false);
    expect(shouldEndVadTurn(timing, detectedAt + 256)).toBe(false);
  });

  it('endpoints normally after post-wake speech and trailing silence', () => {
    const timing = beginVadTurn(5_000, false);
    timing.heardSpeech = true;
    timing.lastVoiceAt = 5_800;
    expect(shouldEndVadTurn(timing, 6_200)).toBe(false);
    expect(shouldEndVadTurn(timing, 6_301)).toBe(true);
  });

  it('retains the hard cap when no speech follows a wake phrase', () => {
    const timing = beginVadTurn(10_000, false);
    expect(shouldEndVadTurn(timing, 10_000 + VAD_MAX_TURN_MS)).toBe(false);
    expect(shouldEndVadTurn(timing, 10_001 + VAD_MAX_TURN_MS)).toBe(true);
  });
});

describe('FIX 2 (2026-09-01 incident): teach-driven starts arming the VAD clocks', () => {
  it('reproduces the pre-fix bug: a stale, un-armed turnStartedAt ends the turn on the very first VAD callback', () => {
    // This IS the old predicate: three call sites called startRecording()
    // bare, leaving vadTurnStartRef at whatever a PRIOR turn (or the
    // useRef(0) initial value) left it — up to ~100s stale, or effectively
    // "the beginning of time" on a hook's first ever recording.
    const staleTiming = { heardSpeech: false, lastVoiceAt: 0, turnStartedAt: 0 };
    const tStart = 100_000; // 100s after the stale clock — the live failure's order of magnitude.
    expect(shouldEndVadTurn(staleTiming, tStart + 128)).toBe(true);
  });

  it('a teach-driven start that correctly arms the clocks (beginVadTurn) does not end within one VAD callback', () => {
    // What FIX 2's single seam in startRecording now does, unconditionally,
    // for every recording start including the three teach-driven call sites.
    const tStart = 100_000;
    const timing = beginVadTurn(tStart, false);
    expect(shouldEndVadTurn(timing, tStart + 128)).toBe(false);
    expect(shouldEndVadTurn(timing, tStart + 256)).toBe(false);
  });

  it('hardening: recordingStartedAt floors a stale turnStartedAt even if some future call site forgets to arm it', () => {
    // Defense in depth: even the OLD (un-armed) timing object, when judged
    // WITH the current recording's own start time, must not end early.
    const staleTiming = { heardSpeech: false, lastVoiceAt: 0, turnStartedAt: 0 };
    const tStart = 100_000;
    expect(
      shouldEndVadTurn(staleTiming, tStart + 128, { recordingStartedAt: tStart }),
    ).toBe(false);
    // And it still respects the hard cap measured from the recording's own start.
    expect(
      shouldEndVadTurn(staleTiming, tStart + VAD_MAX_TURN_MS + 1, {
        recordingStartedAt: tStart,
      }),
    ).toBe(true);
  });

  it('does not regress a genuinely long-silent turn: the hard cap still fires without recordingStartedAt help', () => {
    const timing = beginVadTurn(0, false);
    expect(shouldEndVadTurn(timing, VAD_MAX_TURN_MS + 1)).toBe(true);
  });
});

describe('isVoiceWedged', () => {
  const base = {
    state: 'playing' as const,
    isPlaying: false,
    queuedChunks: 0,
    msSinceActivity: VOICE_WATCHDOG_MS + 1,
  };

  it('flags a "playing" state stuck past the threshold with no audio', () => {
    expect(isVoiceWedged(base)).toBe(true);
  });

  it('flags a stuck "processing" state too', () => {
    expect(isVoiceWedged({ ...base, state: 'processing' })).toBe(true);
  });

  it('never flags non-busy states (ready/idle/recording/connecting/error)', () => {
    for (const state of ['ready', 'idle', 'recording', 'connecting', 'error'] as const) {
      expect(isVoiceWedged({ ...base, state })).toBe(false);
    }
  });

  it('does not flag while audio is actively playing (local progress)', () => {
    expect(isVoiceWedged({ ...base, isPlaying: true })).toBe(false);
  });

  it('does not flag while audio chunks are still queued', () => {
    expect(isVoiceWedged({ ...base, queuedChunks: 2 })).toBe(false);
  });

  it('does not flag a slow-but-live reply still within the threshold', () => {
    expect(isVoiceWedged({ ...base, msSinceActivity: VOICE_WATCHDOG_MS - 1 })).toBe(false);
  });

  it('respects a custom threshold', () => {
    expect(isVoiceWedged({ ...base, msSinceActivity: 5000, thresholdMs: 4000 })).toBe(true);
    expect(isVoiceWedged({ ...base, msSinceActivity: 5000, thresholdMs: 6000 })).toBe(false);
  });
});

describe('isVoiceWedged — zero-byte capture backstop (ALSO, 2026-09-01 incident)', () => {
  // The 2026-09-01 incident free-ran ~1,430 start/stop cycles at ~7.9 Hz over
  // three minutes: every cycle returned to 'ready' via an 'Idle' frame before
  // ever reaching 'processing', and every Idle reset lastActivityRef — so the
  // liveness-only watchdog (below) never once saw a wedged state.
  const fastLoopButLooksAlive = {
    state: 'ready' as const,
    isPlaying: false,
    queuedChunks: 0,
    msSinceActivity: 0, // an Idle frame JUST landed — liveness looks perfect.
  };

  it('fails today: a state the liveness-only predicate cannot flag, no matter how fast the loop spins', () => {
    // The pre-fix predicate (no zero-byte-capture awareness at all) never
    // returns true here — this IS the incident's blind spot.
    expect(isVoiceWedged(fastLoopButLooksAlive)).toBe(false);
    for (let n = 0; n < 50; n++) {
      expect(isVoiceWedged({ ...fastLoopButLooksAlive, msSinceActivity: 0 })).toBe(false);
    }
  });

  it('flags a run of zero-byte captures regardless of state or liveness', () => {
    expect(
      isVoiceWedged({
        ...fastLoopButLooksAlive,
        consecutiveZeroByteCaptures: VOICE_ZERO_BYTE_CAPTURE_STREAK_LIMIT,
      }),
    ).toBe(true);
  });

  it('does not flag below the limit', () => {
    expect(
      isVoiceWedged({
        ...fastLoopButLooksAlive,
        consecutiveZeroByteCaptures: VOICE_ZERO_BYTE_CAPTURE_STREAK_LIMIT - 1,
      }),
    ).toBe(false);
  });

  it('respects a custom zero-byte-capture limit', () => {
    expect(
      isVoiceWedged({
        ...fastLoopButLooksAlive,
        consecutiveZeroByteCaptures: 2,
        zeroByteCaptureLimit: 2,
      }),
    ).toBe(true);
    expect(
      isVoiceWedged({
        ...fastLoopButLooksAlive,
        consecutiveZeroByteCaptures: 2,
        zeroByteCaptureLimit: 3,
      }),
    ).toBe(false);
  });
});

describe('isInterruptibleState (barge-in routing)', () => {
  it('is interruptible while Henry is producing or speaking a reply', () => {
    // The states where "stop and let me talk" is meaningful.
    expect(isInterruptibleState('processing')).toBe(true);
    expect(isInterruptibleState('playing')).toBe(true);
  });

  it('is NOT interruptible in ready/idle/recording/connecting/error', () => {
    // In 'ready' a Space press starts recording; the rest have nothing to stop.
    for (const state of ['ready', 'idle', 'recording', 'connecting', 'error'] as VoiceState[]) {
      expect(isInterruptibleState(state)).toBe(false);
    }
  });

  it('covers exactly the two busy states that lock the mic', () => {
    // Guards against the barge-in set drifting from the mic-locking set.
    const all: VoiceState[] = ['idle', 'connecting', 'ready', 'recording', 'processing', 'playing', 'error'];
    expect(all.filter(isInterruptibleState)).toEqual(['processing', 'playing']);
  });
});

describe('routeWakeEvent (wake-word / spoken-stop routing)', () => {
  const all: VoiceState[] = ['idle', 'connecting', 'ready', 'recording', 'processing', 'playing', 'error'];

  it('a wake detection opens a turn only from ready', () => {
    expect(routeWakeEvent('wake', 'ready')).toBe('start-turn');
    for (const state of all.filter(s => s !== 'ready')) {
      // Mid-turn (or with no live socket) there is nothing to open.
      expect(routeWakeEvent('wake', state)).toBe('ignore');
    }
  });

  it('a spoken stop halts a reply being produced or spoken', () => {
    expect(routeWakeEvent('stop', 'playing')).toBe('halt-playback');
    expect(routeWakeEvent('stop', 'processing')).toBe('halt-playback');
  });

  it('a spoken stop with nothing in flight is not a command', () => {
    for (const state of all.filter(s => s !== 'playing' && s !== 'processing')) {
      expect(routeWakeEvent('stop', state)).toBe('ignore');
    }
  });

  it('unknown kinds are ignored in every state', () => {
    for (const state of all) {
      expect(routeWakeEvent('mystery', state)).toBe('ignore');
      expect(routeWakeEvent('', state)).toBe('ignore');
    }
  });
});

describe('isTransientVoiceIdle (20260821_14 empty STT flash)', () => {
  it('treats last night\'s empty-STT toasts as idle, not errors', () => {
    expect(isTransientVoiceIdle('No speech detected — try again')).toBe(true);
    expect(isTransientVoiceIdle('Recording too short — hold longer to speak')).toBe(true);
  });

  it('leaves real faults visible', () => {
    expect(isTransientVoiceIdle('STT failed: model missing')).toBe(false);
    expect(isTransientVoiceIdle('Voice reply failed: timeout')).toBe(false);
    expect(isTransientVoiceIdle(null)).toBe(false);
  });
});

describe('endpointWindowMs (how long "listening" holds after you stop)', () => {
  it('hands a short conversational ask over on the tight window', () => {
    expect(endpointWindowMs(0)).toBe(VAD_QUICK_SILENCE_MS);
    expect(endpointWindowMs(VAD_QUICK_TURN_SPEECH_MS - 1)).toBe(VAD_QUICK_SILENCE_MS);
  });

  it('keeps the patient window for a dictation-length turn', () => {
    // Deliberate: a mid-thought pause must survive, and we have no semantic
    // end-of-turn model to buy the short window back with.
    expect(endpointWindowMs(VAD_QUICK_TURN_SPEECH_MS)).toBe(VAD_SILENCE_MS);
    expect(endpointWindowMs(30_000)).toBe(VAD_SILENCE_MS);
  });

  it('keeps the tight window inside the published 300-800ms band', () => {
    expect(VAD_QUICK_SILENCE_MS).toBeGreaterThanOrEqual(300);
    expect(VAD_QUICK_SILENCE_MS).toBeLessThanOrEqual(800);
    expect(VAD_QUICK_SILENCE_MS).toBeLessThan(VAD_SILENCE_MS);
  });

  it('never lets the quick tier outlast the patient one', () => {
    expect(endpointWindowMs(0, { quickMs: 1_500, longMs: 600 })).toBe(600);
  });

  it('falls back to the patient window on a garbage clock', () => {
    expect(endpointWindowMs(NaN)).toBe(VAD_SILENCE_MS);
    expect(endpointWindowMs(-1)).toBe(VAD_SILENCE_MS);
  });
});
