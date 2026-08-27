//! Speaker print — a gate on `/voice`, not a better ear.
//!
//! Kitchen music is supposed to die in VAD (N1). This module catches a
//! *different talker* (radio host, singer, someone in the room) once Jesse
//! has enrolled. No print → admit (fail open) so a fresh install still hears
//! him. The vector is JSON under `~/.permagent/data/voice_print.json`. The
//! enrollment WAVs are never written.
//!
//! Extract is a 32-band log-mel mean, L2-normalised — cheap enough to run
//! before STT (budget: ≤ 80 ms; this is ~1 ms on an M4). It is a spectral
//! envelope, not a neural x-vector: two voices of similar pitch can overlap.
//! Swap the body of [`extract`] for an ONNX CAM++ / WeSpeaker forward when
//! that model is on disk; the store and the cosine gate stay the same.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Utterances required to enrol. Matches the three orb prompts.
pub const NEED_UTTERANCES: usize = 3;
/// Cosine at or above this admits. Tune in the kitchen (N6); too tight and
/// Jesse vanishes while cooking, too loose and the radio still replies.
pub const ADMIT_THRESHOLD: f32 = 0.62;
const MEL_BINS: usize = 32;
const FFT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gate {
    /// No print on disk, or the vectors cannot be compared.
    Open,
    Admit {
        score: f32,
    },
    Reject {
        score: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicePrint {
    pub dim: usize,
    pub vector: Vec<f32>,
    pub created_at: String,
    pub n_utterances: usize,
}

pub fn store_path() -> PathBuf {
    permagent::config::paths::Paths::in_state_dir("data").join("voice_print.json")
}

pub fn load() -> Option<VoicePrint> {
    load_from(&store_path())
}

fn load_from(path: &Path) -> Option<VoicePrint> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save(print: &VoicePrint) -> std::io::Result<()> {
    save_to(&store_path(), print)
}

fn save_to(path: &Path, print: &VoicePrint) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(print)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
    )?;
    std::fs::rename(&tmp, path)
}

pub fn clear() -> std::io::Result<()> {
    match std::fs::remove_file(store_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let d = na.sqrt() * nb.sqrt();
    if d < 1e-9 {
        return None;
    }
    Some((dot / d).clamp(-1.0, 1.0))
}

pub fn gate(embedding: &[f32]) -> Gate {
    gate_against(load().as_ref().map(|p| p.vector.as_slice()), embedding)
}

pub fn gate_against(print: Option<&[f32]>, embedding: &[f32]) -> Gate {
    let Some(print) = print else {
        return Gate::Open;
    };
    match cosine(print, embedding) {
        Some(score) if score >= ADMIT_THRESHOLD => Gate::Admit { score },
        Some(score) => Gate::Reject { score },
        None => Gate::Open,
    }
}

pub fn mean_l2(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = vectors.first()?.len();
    if dim == 0 || vectors.iter().any(|v| v.len() != dim) {
        return None;
    }
    let n = vectors.len() as f32;
    let mut acc = vec![0.0f32; dim];
    for v in vectors {
        for (a, x) in acc.iter_mut().zip(v) {
            *a += *x / n;
        }
    }
    l2_normalize(&mut acc);
    Some(acc)
}

pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

pub fn from_utterances(vectors: &[Vec<f32>]) -> Option<VoicePrint> {
    let vector = mean_l2(vectors)?;
    Some(VoicePrint {
        dim: vector.len(),
        n_utterances: vectors.len(),
        vector,
        created_at: now_rfc3339(),
    })
}

/// Log-mel mean of one utterance. `None` when the clip is too short to judge.
/// Scores at most the last 2.5 s so a 60 s buffer cannot blow the 80 ms budget.
pub fn extract(samples: &[f32], sample_rate: u32) -> Option<Vec<f32>> {
    let min = (sample_rate as usize).saturating_mul(3) / 10; // 300 ms
    if samples.len() < min || sample_rate == 0 {
        return None;
    }
    let max = (sample_rate as usize).saturating_mul(5) / 2;
    let samples = if samples.len() > max {
        &samples[samples.len() - max..]
    } else {
        samples
    };
    let hop = (sample_rate as usize / 100).max(1); // 10 ms
    let mut acc = vec![0.0f32; MEL_BINS];
    let mut frames = 0u32;
    let mut i = 0;
    while i + FFT <= samples.len() {
        let frame = &samples[i..i + FFT];
        accumulate_mel(frame, sample_rate, &mut acc);
        frames += 1;
        i += hop;
    }
    if frames == 0 {
        return None;
    }
    for x in &mut acc {
        *x /= frames as f32;
    }
    l2_normalize(&mut acc);
    Some(acc)
}

fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-9 {
        return;
    }
    for x in v {
        *x /= n;
    }
}

fn accumulate_mel(frame: &[f32], sample_rate: u32, acc: &mut [f32]) {
    let n = frame.len();
    let mut mags = vec![0.0f32; n / 2];
    for k in 0..n / 2 {
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, &x) in frame.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos();
            let ang = 2.0 * std::f32::consts::PI * k as f32 * i as f32 / n as f32;
            re += x * w * ang.cos();
            im -= x * w * ang.sin();
        }
        mags[k] = (re * re + im * im).sqrt();
    }
    let nyquist = sample_rate as f32 / 2.0;
    for (b, slot) in acc.iter_mut().enumerate() {
        let lo = mel_hz(b, MEL_BINS, nyquist);
        let hi = mel_hz(b + 1, MEL_BINS, nyquist);
        let mut e = 0.0f32;
        let mut c = 0u32;
        for (k, &m) in mags.iter().enumerate() {
            let hz = k as f32 * nyquist / (n / 2) as f32;
            if hz >= lo && hz < hi {
                e += m;
                c += 1;
            }
        }
        let avg = if c == 0 { 0.0 } else { e / c as f32 };
        *slot += (avg + 1e-9).ln();
    }
}

fn mel_hz(band: usize, n_bands: usize, nyquist: f32) -> f32 {
    let m_lo = hz_to_mel(80.0);
    let m_hi = hz_to_mel(nyquist.min(7600.0));
    let m = m_lo + (m_hi - m_lo) * band as f32 / n_bands as f32;
    mel_to_hz(m)
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
}

/// Orb prompts — same strings the iOS client shows. Server echoes the next
/// one so a reconnect cannot desync the count.
pub const PROMPTS: [&str; NEED_UTTERANCES] = [
    "What's on my board?",
    "Henry, I'm in the kitchen.",
    "Tell me something interesting.",
];

pub fn prompt_at(have: usize) -> Option<&'static str> {
    PROMPTS.get(have).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, sr: u32, secs: f32) -> Vec<f32> {
        let n = (sr as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / sr as f32).sin() * 0.2)
            .collect()
    }

    #[test]
    fn no_print_is_fail_open() {
        assert_eq!(gate_against(None, &[0.1, 0.2, 0.3]), Gate::Open);
    }

    #[test]
    fn self_admits_and_other_rejects() {
        let me = vec![1.0f32, 0.0, 0.0];
        let also_me = vec![0.99, 0.01, 0.0];
        let other = vec![0.0, 0.0, 1.0];
        match gate_against(Some(&me), &also_me) {
            Gate::Admit { score } => assert!(score > 0.9, "{score}"),
            g => panic!("self must admit: {g:?}"),
        }
        match gate_against(Some(&me), &other) {
            Gate::Reject { score } => assert!(score < 0.2, "{score}"),
            g => panic!("other must reject: {g:?}"),
        }
    }

    #[test]
    fn dim_mismatch_fails_open() {
        assert_eq!(
            gate_against(Some(&[1.0, 0.0]), &[1.0, 0.0, 0.0]),
            Gate::Open
        );
    }

    #[test]
    fn two_sines_are_not_the_same_speaker() {
        let a = extract(&tone(180.0, 16_000, 0.6), 16_000).expect("low");
        let b = extract(&tone(420.0, 16_000, 0.6), 16_000).expect("high");
        let score = cosine(&a, &b).unwrap();
        assert!(
            score < ADMIT_THRESHOLD,
            "180 Hz vs 420 Hz must not admit ({score})"
        );
    }

    #[test]
    fn same_sine_admits() {
        let a = extract(&tone(180.0, 16_000, 0.6), 16_000).unwrap();
        let b = extract(&tone(180.0, 16_000, 0.7), 16_000).unwrap();
        match gate_against(Some(&a), &b) {
            Gate::Admit { .. } => {}
            g => panic!("same tone must admit: {g:?}"),
        }
    }

    #[test]
    fn three_utterances_make_a_print() {
        let v = extract(&tone(200.0, 16_000, 0.5), 16_000).unwrap();
        let print = from_utterances(&[v.clone(), v.clone(), v]).unwrap();
        assert_eq!(print.n_utterances, 3);
        assert_eq!(print.dim, MEL_BINS);
    }

    #[test]
    fn persist_round_trip() {
        let dir = std::env::temp_dir().join(format!("voice-print-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voice_print.json");
        let print = VoicePrint {
            dim: 2,
            vector: vec![0.6, 0.8],
            created_at: "t0".into(),
            n_utterances: 3,
        };
        save_to(&path, &print).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, print);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_short_does_not_extract() {
        assert!(extract(&[0.1; 100], 16_000).is_none());
    }

    #[test]
    fn extract_stays_inside_the_score_budget() {
        let clip = tone(180.0, 16_000, 2.5);
        let t0 = std::time::Instant::now();
        assert!(extract(&clip, 16_000).is_some());
        let ms = t0.elapsed().as_millis();
        let budget = if cfg!(debug_assertions) { 400 } else { 80 };
        assert!(
            ms <= budget,
            "extract of 2.5 s must stay ≤ {budget} ms (80 ms in release, not on the LLM clock); took {ms} ms"
        );
    }
}
