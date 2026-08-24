import { describe, it, expect } from 'vitest';
import {
  isVoiceWedged,
  isInterruptibleState,
  isTransientVoiceIdle,
  routeWakeEvent,
  VOICE_WATCHDOG_MS,
  type VoiceState,
} from './useVoice';

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
