//! sherpa-onnx STT backend (Moonshine).
//!
//! TTS has been moved to ort_kokoro_backend.rs (standalone Kokoro via ort +
//! misaki-rs, GPL-clean). This module provides only STT via sherpa-onnx.

use super::provider::{SpeechToText, SttConfig};
use anyhow::Context;
use sherpa_onnx::{
    OfflineModelConfig, OfflineMoonshineModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
};
use std::path::{Path, PathBuf};

/// sherpa-onnx Moonshine STT backend.
pub struct SherpaMoonshineStt {
    recognizer: OfflineRecognizer,
}

impl SherpaMoonshineStt {
    /// Create a new Moonshine STT recognizer.
    /// `model_dir` should contain: preprocess.onnx, encode.int8.onnx,
    /// uncached_decode.int8.onnx, cached_decode.int8.onnx, tokens.txt
    pub fn new(model_dir: &Path, num_threads: i32) -> anyhow::Result<Self> {
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                moonshine: OfflineMoonshineModelConfig {
                    preprocessor: Some(model_dir.join("preprocess.onnx").to_string_lossy().into()),
                    encoder: Some(model_dir.join("encode.int8.onnx").to_string_lossy().into()),
                    uncached_decoder: Some(
                        model_dir
                            .join("uncached_decode.int8.onnx")
                            .to_string_lossy()
                            .into(),
                    ),
                    cached_decoder: Some(
                        model_dir
                            .join("cached_decode.int8.onnx")
                            .to_string_lossy()
                            .into(),
                    ),
                    ..Default::default()
                },
                tokens: Some(model_dir.join("tokens.txt").to_string_lossy().into()),
                num_threads,
                ..Default::default()
            },
            ..Default::default()
        };

        let recognizer =
            OfflineRecognizer::create(&config).context("Failed to create Moonshine recognizer")?;
        Ok(Self { recognizer })
    }
}

/// Moonshine is a *short-form* recognizer: it decodes the whole waveform in a
/// single pass with a bounded decoder, so a long utterance (a multi-clause
/// spoken instruction) gets truncated — the later clauses are silently dropped
/// (#452). Above this length we split the audio into windows and transcribe
/// each, then join, which is the standard sherpa-onnx pattern for long Moonshine
/// audio. Kept comfortably below Moonshine's practical single-pass ceiling.
const MAX_SEGMENT_SECS: f32 = 12.0;
/// Only segment audio LONGER than this. Normal push-to-talk turns stay on the
/// exact single-pass path (zero behaviour change); only genuinely long turns are
/// windowed. Set above `MAX_SEGMENT_SECS` so a turn just over the window size
/// isn't split into an awkward tiny tail.
const SEGMENT_THRESHOLD_SECS: f32 = 14.0;
/// Frame size (20 ms) for the RMS energy envelope used to find the quietest
/// point — a between-word/clause silence — to cut on, so no word is sliced.
const ENERGY_FRAME_SECS: f32 = 0.02;

/// Split `samples` into transcription windows as index ranges.
///
/// Short audio (`<= SEGMENT_THRESHOLD_SECS`) returns a single whole-buffer range
/// so the common path is byte-for-byte unchanged. Long audio is cut into
/// `<= MAX_SEGMENT_SECS` windows, each break placed at the lowest-energy frame in
/// a search band before the hard limit (a natural pause between words) so a
/// window edge never lands mid-word. The returned ranges are contiguous,
/// non-overlapping, and cover `0..samples.len()` exactly.
///
/// Pure (no model, no I/O) so the windowing logic is unit-testable on synthetic
/// signals without the Moonshine assets.
fn segment_ranges(samples: &[f32], sample_rate: u32) -> Vec<std::ops::Range<usize>> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let sr = sample_rate.max(1) as f32;
    let max_len = ((MAX_SEGMENT_SECS * sr) as usize).max(1);
    let threshold_len = (SEGMENT_THRESHOLD_SECS * sr) as usize;
    if n <= threshold_len {
        // A single whole-buffer window (the original single-pass path).
        return std::iter::once(0..n).collect();
    }

    let frame = ((ENERGY_FRAME_SECS * sr) as usize).max(1);
    let mut ranges = Vec::new();
    let mut start = 0usize;
    while n - start > max_len {
        let hard = start + max_len;
        // Look for the quietest frame in the last ~40% of the window, so the cut
        // lands on a pause near — but not past — the hard limit.
        let band_start = start + ((max_len as f32 * 0.6) as usize);
        let mut best_cut = hard;
        let mut best_energy = f32::MAX;
        let mut i = band_start;
        while i < hard {
            let end = (i + frame).min(hard);
            let e: f32 =
                samples[i..end].iter().map(|s| s * s).sum::<f32>() / (end - i).max(1) as f32;
            if e < best_energy {
                best_energy = e;
                best_cut = i + (end - i) / 2;
            }
            i += frame;
        }
        // Guarantee forward progress (never emit an empty or backwards range).
        if best_cut <= start {
            best_cut = hard;
        }
        ranges.push(start..best_cut);
        start = best_cut;
    }
    if start < n {
        ranges.push(start..n);
    }
    ranges
}

impl SherpaMoonshineStt {
    /// Single-pass decode of one window. This is the raw Moonshine call; the
    /// public `transcribe` splits long audio into windows and calls this per
    /// window so a long turn is never truncated to its first clause.
    fn transcribe_window(&self, samples: &[f32], sample_rate: u32) -> anyhow::Result<String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, samples);
        self.recognizer.decode(&stream);
        let result = stream
            .get_result()
            .context("Moonshine: failed to get recognition result")?;
        Ok(result.text.trim().to_string())
    }
}

impl SpeechToText for SherpaMoonshineStt {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        _config: &SttConfig,
    ) -> anyhow::Result<String> {
        let ranges = segment_ranges(samples, sample_rate);
        // Short audio: one window, identical to the original single-pass path.
        if ranges.len() <= 1 {
            return self.transcribe_window(samples, sample_rate);
        }
        // Long audio: transcribe each window and join, so no clause is dropped.
        let mut parts: Vec<String> = Vec::with_capacity(ranges.len());
        for r in ranges {
            let text = self.transcribe_window(&samples[r], sample_rate)?;
            if !text.is_empty() {
                parts.push(text);
            }
        }
        Ok(parts.join(" "))
    }
}

/// STT model paths.
#[derive(Clone, Debug)]
pub struct VoiceModelPaths {
    pub stt_model_dir: PathBuf,
}

impl VoiceModelPaths {
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("permagent")
            .join("models")
            .join("voice");
        Self {
            stt_model_dir: base.join("sherpa-onnx-moonshine-tiny-en-int8"),
        }
    }

    pub fn models_exist(&self) -> bool {
        self.stt_model_dir.join("tokens.txt").exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    fn secs(n: f32) -> usize {
        (n * SR as f32) as usize
    }

    /// A short turn is transcribed in one pass — the exact original path, no
    /// windowing, so the common case is untouched.
    #[test]
    fn short_audio_is_a_single_whole_buffer_window() {
        let samples = vec![0.2f32; secs(5.0)];
        let ranges = segment_ranges(&samples, SR);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 0..samples.len());
    }

    /// Exactly at the threshold still counts as short (single pass).
    #[test]
    fn threshold_length_is_not_split() {
        let samples = vec![0.2f32; secs(SEGMENT_THRESHOLD_SECS)];
        let ranges = segment_ranges(&samples, SR);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 0..samples.len());
    }

    /// Empty input yields no windows (guards the `samples[r]` slicing).
    #[test]
    fn empty_audio_yields_no_windows() {
        assert!(segment_ranges(&[], SR).is_empty());
    }

    /// A long turn is split into multiple windows that are contiguous,
    /// non-overlapping, each within the hard limit, and cover the whole buffer —
    /// the invariant that guarantees no audio (and so no clause) is dropped.
    #[test]
    fn long_audio_windows_cover_the_buffer_without_gaps_or_overlap() {
        let n = secs(40.0);
        let samples = vec![0.3f32; n];
        let ranges = segment_ranges(&samples, SR);

        assert!(ranges.len() >= 3, "40s should split into 3+ windows");
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, n);

        let max_len = secs(MAX_SEGMENT_SECS);
        let mut prev_end = 0usize;
        for r in &ranges {
            assert_eq!(r.start, prev_end, "windows must be contiguous, no gap");
            assert!(r.end > r.start, "window must be non-empty");
            assert!(
                r.end - r.start <= max_len,
                "window {:?} exceeds the {} sample hard limit",
                r,
                max_len
            );
            prev_end = r.end;
        }
        assert_eq!(prev_end, n, "windows must cover the full buffer");
    }

    /// The cut lands on a silence gap: a long loud buffer with a clear silent
    /// band placed inside the search window should be split *inside* that band,
    /// so a window edge never slices through a word.
    #[test]
    fn cut_prefers_a_silence_gap_over_loud_audio() {
        let n = secs(20.0);
        let mut samples = vec![0.5f32; n];
        // Silence band from 11.0s..11.4s — inside the first window's search band
        // (0.6*12s = 7.2s .. 12s).
        let gap_start = secs(11.0);
        let gap_end = secs(11.4);
        for s in &mut samples[gap_start..gap_end] {
            *s = 0.0;
        }

        let ranges = segment_ranges(&samples, SR);
        assert_eq!(ranges.len(), 2, "20s splits into two windows");
        let cut = ranges[0].end;
        assert!(
            cut >= gap_start && cut <= gap_end,
            "cut {} should fall within the silence band {}..{}",
            cut,
            gap_start,
            gap_end
        );
    }
}
