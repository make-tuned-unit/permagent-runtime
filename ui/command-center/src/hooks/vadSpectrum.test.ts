// Regression tests for the hands-free VAD's spectral gate.
//
// The gate this replaces rejected essentially ALL speech, leaving the mic hot
// and the orb stuck on "Listening" while nothing was ever sent to the daemon.
// It was un-testable inline, so the error survived a release. These cases are
// built from realistic `getByteFrequencyData` output: a DECIBEL scale mapped to
// 0..255 over [-100, -30] dB, where even quiet bins read well above zero.

import { describe, expect, it } from 'vitest';
import { spectrumLooksLikeVoice } from './vadSpectrum';

/** dB → the byte value getByteFrequencyData would report (defaults -100/-30). */
function db(value: number): number {
  return Math.max(0, Math.min(255, Math.round((255 / 70) * (value + 100))));
}

/** 32 bins (fftSize 64), filled from a per-bin dB function. */
function spectrum(dbAt: (bin: number) => number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => db(dbAt(i)));
}

describe('spectrumLooksLikeVoice', () => {
  it('admits a voiced vowel — the case the old sum-ratio gate rejected', () => {
    // Strong low harmonics, real (not silent) mid and high content.
    const voice = spectrum(i => (i <= 4 ? -40 : i < 12 ? -62 : -78));
    expect(spectrumLooksLikeVoice(voice)).toBe(true);
  });

  it('admits quieter speech, where the tilt is present but modest', () => {
    const quiet = spectrum(i => (i <= 4 ? -58 : i < 12 ? -70 : -80));
    expect(spectrumLooksLikeVoice(quiet)).toBe(true);
  });

  it('rejects a bright broadband transient — the mechanical key click', () => {
    // A click is loud everywhere, and brightest up top.
    const click = spectrum(i => (i <= 4 ? -55 : i < 12 ? -50 : -45));
    expect(spectrumLooksLikeVoice(click)).toBe(false);
  });

  it('rejects a flat broadband buffer', () => {
    const flat = spectrum(() => -55);
    expect(spectrumLooksLikeVoice(flat)).toBe(false);
  });

  // Fail-open contract: an unresponsive gate must never make voice undetectable.
  it('admits when the spectrum is too coarse to judge', () => {
    expect(spectrumLooksLikeVoice(new Uint8Array(4))).toBe(true);
  });

  it('admits digital silence rather than vetoing on no evidence', () => {
    expect(spectrumLooksLikeVoice(new Uint8Array(32))).toBe(true);
  });

  it('admits when the bright band is silent — cannot be a broadband transient', () => {
    const lowOnly = spectrum(i => (i <= 4 ? -45 : -100));
    expect(spectrumLooksLikeVoice(lowOnly)).toBe(true);
  });

  // The specific arithmetic that broke it: the low band's share of the total
  // dB SUM is ~0.3 for clear speech, so any threshold near 0.55 is unreachable.
  it('does not depend on the low band holding a majority of the summed bytes', () => {
    const voice = spectrum(i => (i <= 4 ? -40 : i < 12 ? -62 : -78));
    const total = voice.reduce((a, b) => a + b, 0);
    const lowShare = (voice[0] + voice[1] + voice[2] + voice[3] + voice[4]) / total;
    expect(lowShare).toBeLessThan(0.55); // the old gate's threshold
    expect(spectrumLooksLikeVoice(voice)).toBe(true); // yet it is plainly voice
  });
});
