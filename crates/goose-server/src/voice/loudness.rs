//! Post-synthesis mastering for Kokoro (and any other f32 PCM we speak).
//!
//! Kokoro packs are not loudness-matched — measured ~12 dB between the quietest
//! and loudest voices — and a typical utterance peaks far below 0 dBFS. On a
//! phone at max volume that still sounds quiet. Peak-normalize to -1 dBFS, then
//! apply a light punctuation contour the model itself does not produce:
//!
//! - `?` — rising gain on the last 600 ms of voiced audio (questions)
//! - `!` — attack boost on the first 400 ms (exclamations)
//!
//! Contours follow the kokoro-tts-kotlin SentencePostProcessor, which exists
//! because Kokoro barely differentiates intonation by punctuation on its own.
//! Envelopes run on the voiced region only, so model-generated silence at the
//! edges is not amplified into a hiss.

/// Peak target after normalize: 10^(-1/20) ≈ 0.891, i.e. -1 dBFS.
const TARGET_PEAK: f32 = 0.891;
/// Below this peak the buffer is treated as silence and left alone.
const SILENCE_PEAK: f32 = 0.01;
/// Voiced if a 10 ms window's RMS clears this (after peak-normalize).
const VOICED_RMS: f32 = 0.02;
const CLIP: f32 = 0.99;

const QUESTION_RAMP_SECS: f32 = 0.600;
const QUESTION_END_GAIN: f32 = 1.15;
const EXCLAIM_ATTACK_SECS: f32 = 0.400;
const EXCLAIM_START_GAIN: f32 = 1.20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contour {
    Statement,
    Question,
    Exclamation,
}

/// Bring `samples` up to a consistent level and shape them from `speech`.
pub fn master(samples: &mut [f32], sample_rate: u32, speech: &str) {
    if samples.is_empty() || sample_rate == 0 {
        return;
    }
    peak_normalize(samples);
    match contour(speech) {
        Contour::Question => rising_ramp(samples, sample_rate),
        Contour::Exclamation => attack_boost(samples, sample_rate),
        Contour::Statement => {}
    }
    clip(samples);
}

fn contour(speech: &str) -> Contour {
    match speech.trim().chars().next_back() {
        Some('?') => Contour::Question,
        Some('!') => Contour::Exclamation,
        _ => Contour::Statement,
    }
}

fn peak_normalize(samples: &mut [f32]) {
    let peak = samples.iter().fold(0.0f32, |p, &s| p.max(s.abs()));
    if peak < SILENCE_PEAK {
        return;
    }
    let gain = TARGET_PEAK / peak;
    for s in samples.iter_mut() {
        *s *= gain;
    }
}

fn rising_ramp(samples: &mut [f32], sample_rate: u32) {
    let Some((start, end)) = voiced_bounds(samples, sample_rate) else {
        return;
    };
    let n = ((QUESTION_RAMP_SECS * sample_rate as f32) as usize).max(1);
    let from = end.saturating_sub(n).max(start);
    let span = (end - from).max(1) as f32;
    for (i, s) in samples[from..end].iter_mut().enumerate() {
        let t = i as f32 / span;
        // Quadratic ease-in: most of the rise lands on the last syllables.
        let g = 1.0 + (QUESTION_END_GAIN - 1.0) * t * t;
        *s *= g;
    }
}

fn attack_boost(samples: &mut [f32], sample_rate: u32) {
    let Some((start, end)) = voiced_bounds(samples, sample_rate) else {
        return;
    };
    let n = ((EXCLAIM_ATTACK_SECS * sample_rate as f32) as usize).max(1);
    let to = (start + n).min(end);
    let span = (to - start).max(1) as f32;
    for (i, s) in samples[start..to].iter_mut().enumerate() {
        let t = i as f32 / span;
        let g = EXCLAIM_START_GAIN + (1.0 - EXCLAIM_START_GAIN) * t;
        *s *= g;
    }
}

fn clip(samples: &mut [f32]) {
    for s in samples.iter_mut() {
        *s = s.clamp(-CLIP, CLIP);
    }
}

fn voiced_bounds(samples: &[f32], sample_rate: u32) -> Option<(usize, usize)> {
    let win = (sample_rate as usize / 100).max(1);
    let mut first = None;
    let mut last = 0;
    let mut i = 0;
    while i < samples.len() {
        let end = (i + win).min(samples.len());
        let n = (end - i) as f32;
        let rms = (samples[i..end].iter().map(|s| s * s).sum::<f32>() / n).sqrt();
        if rms >= VOICED_RMS {
            if first.is_none() {
                first = Some(i);
            }
            last = end;
        }
        i = end;
    }
    first.map(|s| (s, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 24_000;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        (0..n).map(|i| amp * (i as f32 * 0.1).sin()).collect()
    }

    fn peak_of(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |p, &s| p.max(s.abs()))
    }

    #[test]
    fn silence_is_left_alone() {
        let mut samples = vec![0.0f32; 100];
        master(&mut samples, SR, "Hello.");
        assert!(samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn quiet_speech_is_brought_up_to_target() {
        let mut samples = tone(0.5, 0.2);
        master(&mut samples, SR, "Okay.");
        let p = peak_of(&samples);
        assert!(
            (p - TARGET_PEAK).abs() < 0.02,
            "peak {p} should sit at {TARGET_PEAK}"
        );
    }

    #[test]
    fn already_loud_speech_is_brought_down() {
        let mut samples = tone(0.5, 0.99);
        master(&mut samples, SR, "Okay.");
        let p = peak_of(&samples);
        assert!(p <= CLIP + 1e-6, "must not clip: {p}");
        assert!(
            (p - TARGET_PEAK).abs() < 0.02,
            "peak {p} should sit at {TARGET_PEAK}"
        );
    }

    #[test]
    fn question_gets_louder_toward_the_end() {
        let mut samples = tone(1.0, 0.3);
        master(&mut samples, SR, "Ready?");
        let mid = samples.len() / 2;
        let head: f32 = samples[..mid].iter().map(|s| s.abs()).sum::<f32>() / mid as f32;
        let tail: f32 =
            samples[mid..].iter().map(|s| s.abs()).sum::<f32>() / (samples.len() - mid) as f32;
        assert!(
            tail > head * 1.02,
            "question tail ({tail}) should exceed head ({head})"
        );
        assert!(peak_of(&samples) <= CLIP + 1e-6);
    }

    #[test]
    fn exclamation_boosts_the_attack() {
        let mut samples = tone(1.0, 0.3);
        master(&mut samples, SR, "Yes!");
        let attack_n = (EXCLAIM_ATTACK_SECS * SR as f32) as usize;
        let attack: f32 =
            samples[..attack_n].iter().map(|s| s.abs()).sum::<f32>() / attack_n as f32;
        let rest: f32 = samples[attack_n..].iter().map(|s| s.abs()).sum::<f32>()
            / (samples.len() - attack_n) as f32;
        assert!(
            attack > rest * 1.02,
            "exclamation attack ({attack}) should exceed the rest ({rest})"
        );
        assert!(peak_of(&samples) <= CLIP + 1e-6);
    }

    #[test]
    fn statement_does_not_tilt_the_envelope() {
        let mut samples = tone(1.0, 0.3);
        master(&mut samples, SR, "Okay.");
        let mid = samples.len() / 2;
        let head: f32 = samples[..mid].iter().map(|s| s.abs()).sum::<f32>() / mid as f32;
        let tail: f32 =
            samples[mid..].iter().map(|s| s.abs()).sum::<f32>() / (samples.len() - mid) as f32;
        let ratio = tail / head.max(1e-6);
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "statement should stay flat, ratio={ratio}"
        );
    }

    #[test]
    fn empty_and_zero_rate_are_noops() {
        let mut empty: Vec<f32> = vec![];
        master(&mut empty, SR, "Hi!");
        let mut samples = vec![0.2; 8];
        master(&mut samples, 0, "Hi!");
        assert_eq!(samples, vec![0.2; 8]);
    }
}
