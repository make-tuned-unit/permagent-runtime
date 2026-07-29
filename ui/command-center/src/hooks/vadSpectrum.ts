/**
 * Spectral voice/transient discriminator for the hands-free VAD.
 *
 * Level alone cannot separate a mechanical key click from a word — the click is
 * often LOUDER. The usable difference is spectral shape: a click is a bright
 * broadband transient, while voiced speech tilts its energy low.
 *
 * ── Why this is a pure function ──
 * The first version of this lived inline in useVoice and was wrong in a way
 * nothing could catch: it summed `getByteFrequencyData` output and required the
 * low bins to hold 55% of that sum. But those bytes are a DECIBEL scale mapped
 * to 0..255 over [minDecibels, maxDecibels] = [-100, -30] dB — not linear
 * energy. Every bin above the noise floor contributes a large byte value, so
 * with 32 bins the 27 non-voice bins swamp the 5 voice bins: real speech scores
 * around 0.3 and could never reach 0.55. The gate rejected essentially all
 * speech, the onset streak never completed, and the mic stayed hot with the orb
 * reading "Listening" while nothing was ever sent.
 *
 * Comparing band AVERAGES avoids the trap entirely: the tilt between bands is
 * meaningful on a dB scale, whereas a band's share of a dB SUM is not.
 */

/**
 * How much louder (on the byte-dB scale) the voice band must be than the bright
 * band before a loud buffer counts as speech.
 *
 * Deliberately close to 1.0 — this is a VETO on obvious broadband transients,
 * not a positive test for speech. Failing to reject a keystroke costs one empty
 * transcript; failing to admit speech makes the agent unresponsive, which is
 * far worse. Raise only with real recordings of both cases in hand.
 */
export const VOICE_TILT = 1.12;

/** Fraction of the spectrum treated as the voice band (~0-1.2kHz at fftSize 64
 *  on a 16kHz context). */
const LOW_FRACTION = 0.16;
/** Where the "bright" band starts (~3kHz+) — a key click's signature lives
 *  here, a voiced vowel's does not. */
const HIGH_FRACTION = 0.38;

/**
 * True when the spectrum looks like voice rather than a broadband transient.
 *
 * Fails OPEN (returns true) whenever there is nothing to judge — too few bins,
 * or a spectrum with no meaningful low-band signal. Voice must never become
 * undetectable because the analyser was unavailable or the numbers were odd.
 */
export function spectrumLooksLikeVoice(data: ArrayLike<number>): boolean {
  const n = data.length;
  if (n < 8) return true; // too coarse to judge — admit

  const lowEnd = Math.max(1, Math.round(n * LOW_FRACTION));
  const highStart = Math.max(lowEnd + 1, Math.round(n * HIGH_FRACTION));
  if (highStart >= n) return true; // no bright band to compare against — admit

  let low = 0;
  for (let i = 0; i < lowEnd; i++) low += data[i];
  let high = 0;
  for (let i = highStart; i < n; i++) high += data[i];

  const lowAvg = low / lowEnd;
  const highAvg = high / (n - highStart);

  // Nothing audible in the voice band — no evidence either way, so admit and
  // let the level threshold and onset streak do the deciding.
  if (lowAvg <= 1) return true;
  // A silent bright band cannot be a broadband transient.
  if (highAvg <= 1) return true;

  return lowAvg >= highAvg * VOICE_TILT;
}
