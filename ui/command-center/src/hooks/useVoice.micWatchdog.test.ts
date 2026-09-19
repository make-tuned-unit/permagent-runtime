/**
 * Mic-input watchdog — the pure decisions.
 *
 * User report (2026-09-18): voice "isn't responding and just gets stuck on
 * listening". The hands-free VAD graph had gone deaf (a Bluetooth profile flip
 * or sleep/wake suspends its AudioContext; the ScriptProcessor stops firing)
 * and nothing could tell "armed and hearing you" from "armed and receiving
 * nothing". These lock down the diagnosis and the rebuild-then-surface policy
 * without an audio graph; the hook wiring is covered in
 * useVoice.micWatchdog.hook.test.tsx.
 */
import { describe, expect, it } from 'vitest';
import {
  MIC_FAULT_MESSAGE,
  MIC_FAULT_RETRY_MS,
  MIC_SILENCE_PEAK,
  MIC_SILENT_MESSAGE,
  MIC_SILENT_MS,
  MIC_STALL_MS,
  WAKE_DEAF_GATED_MS,
  WAKE_DEAF_VOICED_EPISODES,
  VAD_KEEPALIVE_RMS,
  VAD_ONSET_RMS,
  bufferPeak,
  diagnoseMicInput,
  micFaultMessage,
  micInputLabel,
  micWatchdogStep,
  msSinceMicSignal,
  vadOnsetLevel,
  wakeHintFor,
  wakeSpotterLooksDeaf,
  type MicInputSample,
} from './useVoice';

const healthy: MicInputSample = {
  hasStream: true,
  trackEnded: false,
  hasGraph: true,
  ctxState: 'running',
  msTrackMuted: 0,
  msSinceLastBuffer: 128,
  msSinceSignal: 128,
};

describe('diagnoseMicInput', () => {
  it('is healthy while buffers keep arriving on a running context', () => {
    expect(diagnoseMicInput(healthy)).toBeNull();
  });

  it('flags a suspended context — the Bluetooth profile-flip case', () => {
    expect(diagnoseMicInput({ ...healthy, ctxState: 'suspended' })).toBe('context-not-running');
    expect(diagnoseMicInput({ ...healthy, ctxState: 'interrupted' })).toBe('context-not-running');
    expect(diagnoseMicInput({ ...healthy, ctxState: 'closed' })).toBe('context-not-running');
  });

  it('flags a "running" graph whose processor stopped firing', () => {
    expect(diagnoseMicInput({ ...healthy, msSinceLastBuffer: MIC_STALL_MS + 1 })).toBe('no-buffers');
    // A graph that has never processed a buffer is not healthy either.
    expect(diagnoseMicInput({ ...healthy, msSinceLastBuffer: Number.POSITIVE_INFINITY })).toBe('no-buffers');
  });

  it('tolerates a brief mute but flags a sustained one', () => {
    expect(diagnoseMicInput({ ...healthy, msTrackMuted: MIC_STALL_MS - 1 })).toBeNull();
    expect(diagnoseMicInput({ ...healthy, msTrackMuted: MIC_STALL_MS + 1 })).toBe('track-muted');
  });

  it('flags a missing stream, an ended track, and a missing graph', () => {
    expect(diagnoseMicInput({ ...healthy, hasStream: false })).toBe('no-stream');
    expect(diagnoseMicInput({ ...healthy, trackEnded: true })).toBe('track-ended');
    expect(diagnoseMicInput({ ...healthy, hasGraph: false, ctxState: null })).toBe('no-graph');
  });
});

describe('diagnoseMicInput — digital silence', () => {
  // The reviewer's case: HFP/SCO renegotiation or a hardware-muted input keeps
  // the processor firing with all-zero buffers. "Fired at all" called that a
  // healthy mic, so the orb kept saying "Listening" to a dead input.
  it('flags buffers that arrive but carry nothing, once it has lasted', () => {
    expect(diagnoseMicInput({ ...healthy, msSinceSignal: MIC_SILENT_MS - 1 })).toBeNull();
    expect(diagnoseMicInput({ ...healthy, msSinceSignal: MIC_SILENT_MS + 1 })).toBe('silent');
  });

  it('reports a stopped processor ahead of silence (the more specific fault)', () => {
    expect(diagnoseMicInput({ ...healthy, msSinceLastBuffer: MIC_STALL_MS + 1, msSinceSignal: 60_000 }))
      .toBe('no-buffers');
  });
});

describe('bufferPeak', () => {
  it('is zero for an all-zero buffer and catches a single non-zero sample of either sign', () => {
    expect(bufferPeak(new Float32Array(2048))).toBe(0);
    const b = new Float32Array(2048);
    b[1000] = -0.003;
    expect(bufferPeak(b)).toBeCloseTo(0.003);
    expect(bufferPeak(b)).toBeGreaterThan(MIC_SILENCE_PEAK);
  });

  it('puts ordinary room tone well above the silence floor', () => {
    // -60 dBFS room tone is 1e-3 — two orders of magnitude over the floor.
    expect(bufferPeak(new Float32Array(2048).fill(1e-3))).toBeGreaterThan(MIC_SILENCE_PEAK * 10);
  });
});

describe('msSinceMicSignal', () => {
  it('counts from the last non-silent buffer', () => {
    expect(msSinceMicSignal({ now: 10_000, lastSignalAt: 9_000, graphBuiltAt: 0, judgingSince: 1 })).toBe(1_000);
  });

  it('never counts silence from before the current graph existed', () => {
    expect(msSinceMicSignal({ now: 10_000, lastSignalAt: 0, graphBuiltAt: 8_000, judgingSince: 1 })).toBe(2_000);
  });

  it('never counts silence from an unjudged stretch (echo cancellation gating during a reply)', () => {
    // Last signal at 2s, reply played 2s..9s with the mic gated to zeros, loop
    // back in 'ready' at 9s: at 10s that is 1s of judged silence, not 8s.
    expect(msSinceMicSignal({ now: 10_000, lastSignalAt: 2_000, graphBuiltAt: 0, judgingSince: 9_000 })).toBe(1_000);
  });
});

describe('wakeSpotterLooksDeaf', () => {
  const deaf = { gatedForMs: WAKE_DEAF_GATED_MS, voicedEpisodes: WAKE_DEAF_VOICED_EPISODES, spotterHasFired: false };

  it('fails open after repeated ignored speech over enough time — no more "Say Hey Henry" forever', () => {
    expect(wakeSpotterLooksDeaf(deaf)).toBe(true);
  });

  it('needs both the time and the repetitions', () => {
    expect(wakeSpotterLooksDeaf({ ...deaf, gatedForMs: WAKE_DEAF_GATED_MS - 1 })).toBe(false);
    expect(wakeSpotterLooksDeaf({ ...deaf, voicedEpisodes: WAKE_DEAF_VOICED_EPISODES - 1 })).toBe(false);
  });

  it('never fails open once the spotter has fired this session — it works', () => {
    expect(wakeSpotterLooksDeaf({ ...deaf, gatedForMs: 10 * WAKE_DEAF_GATED_MS, voicedEpisodes: 50, spotterHasFired: true }))
      .toBe(false);
  });
});

describe('vadOnsetLevel', () => {
  // The 2026-09-01 forensic mic (Bose QC35 on HFP): DSP-flattened speech
  // between keepalive and onset. The deaf verdict is counted at keepalive.
  const FORENSIC_SPEECH_RMS = 0.015;

  it('a normal gate keeps the onset bar that rejects room noise and key clicks', () => {
    expect(vadOnsetLevel(false)).toBe(VAD_ONSET_RMS);
    expect(FORENSIC_SPEECH_RMS > vadOnsetLevel(false)).toBe(false);
  });

  it('after a deaf verdict, the speech that earned it can open a turn — "just talk" is true', () => {
    expect(vadOnsetLevel(true)).toBe(VAD_KEEPALIVE_RMS);
    expect(FORENSIC_SPEECH_RMS > vadOnsetLevel(true)).toBe(true);
  });
});

describe('micWatchdogStep', () => {
  const base = { rebuildAttempts: 0, msSinceGraphBuilt: 60_000, faultSurfaced: false };

  it('does nothing when healthy', () => {
    expect(micWatchdogStep({ ...base, fault: null })).toBe('healthy');
    expect(micWatchdogStep({ ...base, fault: null, rebuildAttempts: 3, faultSurfaced: true })).toBe('healthy');
  });

  it('does not call an unproven graph healthy (all-zero input inside its silence window)', () => {
    // Otherwise every retry reset the rebuild count and cleared the fault.
    expect(micWatchdogStep({ ...base, fault: null, provenLive: false, rebuildAttempts: 1, faultSurfaced: true }))
      .toBe('wait');
  });

  it('never judges a graph inside its own start-up grace', () => {
    expect(micWatchdogStep({ ...base, fault: 'no-buffers', msSinceGraphBuilt: MIC_STALL_MS - 1 })).toBe('wait');
  });

  it('rebuilds once before telling the user anything', () => {
    expect(micWatchdogStep({ ...base, fault: 'context-not-running' })).toBe('rebuild');
  });

  it('surfaces the fault when the rebuilt graph is dead too — never "Listening" forever', () => {
    expect(
      micWatchdogStep({ ...base, fault: 'no-buffers', rebuildAttempts: 1, msSinceGraphBuilt: MIC_STALL_MS }),
    ).toBe('surface-fault');
  });

  it('keeps retrying slowly once the fault is on screen', () => {
    const surfaced = { ...base, fault: 'no-buffers' as const, rebuildAttempts: 1, faultSurfaced: true };
    expect(micWatchdogStep({ ...surfaced, msSinceGraphBuilt: MIC_FAULT_RETRY_MS - 1 })).toBe('wait');
    expect(micWatchdogStep({ ...surfaced, msSinceGraphBuilt: MIC_FAULT_RETRY_MS })).toBe('rebuild');
  });
});

describe('micFaultMessage', () => {
  it('tells the user to check the input device', () => {
    expect(micFaultMessage('no-buffers')).toBe(MIC_FAULT_MESSAGE);
    expect(micFaultMessage('context-not-running')).toMatch(/check the input device/);
    expect(micFaultMessage('track-ended')).toMatch(/disconnected/);
    expect(micFaultMessage('silent')).toBe(MIC_SILENT_MESSAGE);
  });
});

describe('wakeHintFor — the hint relayed to the popped-out orb', () => {
  // The mirror orb's host never passed a hint, so an armed gate read as a bare
  // "Listening" there. The relayed hint must follow VoiceHost's rule exactly.
  it('asks for the phrase only while the daemon spots and the gate is armed', () => {
    expect(wakeHintFor({ active: true, gated: true, phrase: 'Hey Henry' })).toBe('Say "Hey Henry"');
    expect(wakeHintFor({ active: true, gated: false, phrase: 'Hey Henry' })).toBeNull();
    expect(wakeHintFor({ active: false, gated: true, phrase: 'Hey Henry' })).toBeNull();
    expect(wakeHintFor({ active: true, gated: true, phrase: null })).toBeNull();
  });
});

describe('micInputLabel — names the input in the verdict trail', () => {
  const stream = (track?: { label?: string; readyState?: string }) => ({ getTracks: () => (track ? [track] : []) });
  it('uses the track label, which tells an HFP headset from the built-in mic', () => {
    expect(micInputLabel(stream({ label: 'Bose QC35 II', readyState: 'live' }))).toBe('"Bose QC35 II"');
  });
  it('says when there is no stream, no track, no label, or the track ended', () => {
    expect(micInputLabel(null)).toBe('no microphone stream');
    expect(micInputLabel(stream())).toBe('no microphone track');
    expect(micInputLabel(stream({ label: '', readyState: 'live' }))).toBe('unnamed input');
    expect(micInputLabel(stream({ label: 'MacBook Pro Microphone', readyState: 'ended' })))
      .toBe('"MacBook Pro Microphone" (ended)');
  });
});
