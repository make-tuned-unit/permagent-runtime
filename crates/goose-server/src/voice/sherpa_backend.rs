//! sherpa-onnx STT backend (Moonshine).
//!
//! TTS has been moved to ort_kokoro_backend.rs (standalone Kokoro via ort +
//! misaki-rs, GPL-clean). This module provides only STT via sherpa-onnx.

use super::provider::{
    SpeechToText, StreamingSpeechToText, StreamingSttEvent, StreamingSttSession, SttConfig,
};
use anyhow::Context;
use sherpa_onnx::{
    OfflineModelConfig, OfflineMoonshineModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// The standard four-file layout used by sherpa-onnx streaming Zipformer
/// transducer releases. This is deliberately separate from the KWS layout:
/// KWS's transducer is trained for keyword spotting and is not an open-vocabulary
/// speech recognizer.
const ONLINE_ENCODER_FILE: &str = "encoder-epoch-99-avg-1.int8.onnx";
const ONLINE_DECODER_FILE: &str = "decoder-epoch-99-avg-1.onnx";
const ONLINE_JOINER_FILE: &str = "joiner-epoch-99-avg-1.int8.onnx";
const ONLINE_TOKENS_FILE: &str = "tokens.txt";
const ONLINE_SAMPLE_RATE: u32 = 16_000;
const MAX_ONLINE_CHUNK_SAMPLES: usize = ONLINE_SAMPLE_RATE as usize * 2;
const MAX_ONLINE_TURN_SAMPLES: usize = ONLINE_SAMPLE_RATE as usize * 60;
pub const STREAMING_STT_OPT_IN_ENV: &str = "PERMAGENT_STREAMING_STT";

pub fn streaming_stt_opted_in() -> bool {
    std::env::var(STREAMING_STT_OPT_IN_ENV).as_deref() == Ok("1")
}

/// Paths for an explicitly provisioned online Zipformer transducer.
///
/// The directory lives beside the existing Moonshine model under the normal
/// voice model root. No downloader is implied by this type; callers must
/// provision and validate the files before constructing the recognizer.
#[derive(Clone, Debug)]
pub struct OnlineTransducerModelPaths {
    pub model_dir: PathBuf,
    encoder: PathBuf,
    decoder: PathBuf,
    joiner: PathBuf,
    tokens: PathBuf,
}

impl OnlineTransducerModelPaths {
    pub fn from_dir(model_dir: &Path) -> Self {
        Self {
            model_dir: model_dir.to_path_buf(),
            encoder: model_dir.join(ONLINE_ENCODER_FILE),
            decoder: model_dir.join(ONLINE_DECODER_FILE),
            joiner: model_dir.join(ONLINE_JOINER_FILE),
            tokens: model_dir.join(ONLINE_TOKENS_FILE),
        }
    }

    /// True only when every file required by the online transducer is present
    /// as a regular file. A tokens-only Moonshine/KWS directory is rejected.
    pub fn models_exist(&self) -> bool {
        [&self.encoder, &self.decoder, &self.joiner, &self.tokens]
            .iter()
            .all(|path| path.is_file())
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        if self.models_exist() {
            return Ok(());
        }
        anyhow::bail!(
            "online STT model is incomplete in {}; expected {}, {}, {}, and {}",
            self.model_dir.display(),
            ONLINE_ENCODER_FILE,
            ONLINE_DECODER_FILE,
            ONLINE_JOINER_FILE,
            ONLINE_TOKENS_FILE,
        )
    }
}

/// Optional sherpa-onnx online Zipformer transducer backend.
///
/// This backend is not loaded by default: the shipped Moonshine assets remain
/// the batch fallback until a supported online model is explicitly provisioned.
pub struct SherpaOnlineTransducerStt {
    recognizer: Arc<OnlineRecognizer>,
    batch_fallback: Option<Arc<dyn SpeechToText>>,
}

impl SherpaOnlineTransducerStt {
    pub fn new(model_dir: &Path, num_threads: i32) -> anyhow::Result<Self> {
        let paths = OnlineTransducerModelPaths::from_dir(model_dir);
        paths.require_complete()?;

        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(paths.encoder.to_string_lossy().into_owned());
        config.model_config.transducer.decoder = Some(paths.decoder.to_string_lossy().into_owned());
        config.model_config.transducer.joiner = Some(paths.joiner.to_string_lossy().into_owned());
        config.model_config.tokens = Some(paths.tokens.to_string_lossy().into_owned());
        config.model_config.num_threads = num_threads;
        config.enable_endpoint = true;
        config.decoding_method = Some("greedy_search".to_string());

        let recognizer = OnlineRecognizer::create(&config)
            .ok_or_else(|| anyhow::anyhow!("sherpa-onnx failed to create online recognizer"))?;
        Ok(Self {
            recognizer: Arc::new(recognizer),
            batch_fallback: None,
        })
    }

    /// Keep the existing offline recognizer as the final-only fallback when
    /// online streaming is selected. This is important for valid non-16 kHz
    /// captures and for any online decode failure: opting into partials must
    /// not remove the proven batch path.
    pub fn with_batch_fallback(mut self, fallback: Arc<dyn SpeechToText>) -> Self {
        self.batch_fallback = Some(fallback);
        self
    }

    fn session(&self, sample_rate: u32, generation: u64) -> anyhow::Result<SherpaOnlineSession> {
        validate_online_sample_rate(sample_rate)?;
        Ok(SherpaOnlineSession {
            recognizer: self.recognizer.clone(),
            stream: self.recognizer.create_stream(),
            sample_rate,
            generation,
            cancelled: false,
            finished: false,
            samples_seen: 0,
            last_partial: None,
        })
    }
}

fn validate_online_sample_rate(sample_rate: u32) -> anyhow::Result<()> {
    if sample_rate == ONLINE_SAMPLE_RATE {
        return Ok(());
    }
    anyhow::bail!(
        "online STT requires {} Hz PCM, got {} Hz",
        ONLINE_SAMPLE_RATE,
        sample_rate
    )
}

struct SherpaOnlineSession {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
    sample_rate: u32,
    generation: u64,
    cancelled: bool,
    finished: bool,
    samples_seen: usize,
    last_partial: Option<String>,
}

fn validate_online_chunk(samples: &[f32], samples_seen: usize) -> anyhow::Result<()> {
    if samples.len() > MAX_ONLINE_CHUNK_SAMPLES {
        anyhow::bail!(
            "online STT audio chunk exceeds {} samples",
            MAX_ONLINE_CHUNK_SAMPLES
        );
    }
    if samples_seen
        .checked_add(samples.len())
        .filter(|total| *total <= MAX_ONLINE_TURN_SAMPLES)
        .is_none()
    {
        anyhow::bail!(
            "online STT turn exceeds {} samples",
            MAX_ONLINE_TURN_SAMPLES
        );
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        anyhow::bail!("online STT audio contains a non-finite sample");
    }
    Ok(())
}

fn transcribe_online_or_batch<F>(
    online: F,
    fallback: Option<&dyn SpeechToText>,
    samples: &[f32],
    sample_rate: u32,
    config: &SttConfig,
) -> anyhow::Result<String>
where
    F: FnOnce() -> anyhow::Result<String>,
{
    match online() {
        Ok(text) => Ok(text),
        Err(error) => match fallback {
            Some(fallback) => {
                tracing::debug!(
                    target: "permagentd::voice",
                    "online STT batch decode failed; using offline fallback: {error}"
                );
                fallback.transcribe(samples, sample_rate, config)
            }
            None => Err(error),
        },
    }
}

impl SherpaOnlineSession {
    fn decode_ready(&self) {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
    }

    fn partial_update(&mut self) -> Vec<StreamingSttEvent> {
        let Some(result) = self.recognizer.get_result(&self.stream) else {
            return Vec::new();
        };
        let text = result.text.trim().to_string();
        if text.is_empty() || self.last_partial.as_deref() == Some(text.as_str()) {
            return Vec::new();
        }
        self.last_partial = Some(text.clone());
        vec![StreamingSttEvent::partial(self.generation, text)]
    }
}

impl StreamingSttSession for SherpaOnlineSession {
    fn push_audio(&mut self, samples: &[f32]) -> anyhow::Result<Vec<StreamingSttEvent>> {
        if self.cancelled || self.finished || samples.is_empty() {
            return Ok(Vec::new());
        }
        validate_online_chunk(samples, self.samples_seen)?;
        self.samples_seen += samples.len();
        self.stream
            .accept_waveform(self.sample_rate as i32, samples);
        self.decode_ready();
        Ok(self.partial_update())
    }

    fn finish(&mut self) -> anyhow::Result<Vec<StreamingSttEvent>> {
        if self.cancelled || self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.stream.input_finished();
        self.decode_ready();
        let text = self
            .recognizer
            .get_result(&self.stream)
            .map(|result| result.text.trim().to_string())
            .unwrap_or_default();
        Ok(vec![StreamingSttEvent::final_text(self.generation, text)])
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.finished = true;
    }
}

impl StreamingSpeechToText for SherpaOnlineTransducerStt {
    fn start_stream(
        &self,
        sample_rate: u32,
        _config: &SttConfig,
        generation: u64,
    ) -> anyhow::Result<Box<dyn StreamingSttSession>> {
        Ok(Box::new(self.session(sample_rate, generation)?))
    }
}

impl SpeechToText for SherpaOnlineTransducerStt {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        _config: &SttConfig,
    ) -> anyhow::Result<String> {
        transcribe_online_or_batch(
            || {
                let mut stream = self.session(sample_rate, 0)?;
                for chunk in samples.chunks(MAX_ONLINE_CHUNK_SAMPLES) {
                    let _ = stream.push_audio(chunk)?;
                }
                let updates = stream.finish()?;
                Ok(updates
                    .into_iter()
                    .find_map(|event| match event {
                        StreamingSttEvent::Final { text, .. } => Some(text),
                        StreamingSttEvent::Partial { .. } => None,
                    })
                    .unwrap_or_default())
            },
            self.batch_fallback.as_deref(),
            samples,
            sample_rate,
            _config,
        )
    }

    fn streaming_capability(&self) -> Option<&dyn StreamingSpeechToText> {
        Some(self)
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

    /// Optional online model location under the same existing voice model
    /// store. It is intentionally not downloaded or loaded by default.
    pub fn online_stt_model_dir(&self) -> PathBuf {
        self.stt_model_dir
            .parent()
            .unwrap_or(&self.stt_model_dir)
            .join("sherpa-onnx-streaming-zipformer-en")
    }

    pub fn online_models_exist(&self) -> bool {
        OnlineTransducerModelPaths::from_dir(&self.online_stt_model_dir()).models_exist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const SR: u32 = 16_000;

    const ONLINE_FIXTURE_ENV: &str = "PERMAGENT_ONLINE_STT_FIXTURE";
    const ONLINE_FIXTURE_FILES: [(&str, &str); 4] = [
        (
            ONLINE_ENCODER_FILE,
            "3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3",
        ),
        (
            ONLINE_DECODER_FILE,
            "45a7f940ecfb53d89fa270ad11b88b961e53a317203eb24b1c8e95ed208b0f30",
        ),
        (
            ONLINE_JOINER_FILE,
            "e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e",
        ),
        (
            ONLINE_TOKENS_FILE,
            "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb",
        ),
    ];

    struct FakeBatchFallback;

    impl SpeechToText for FakeBatchFallback {
        fn transcribe(
            &self,
            _samples: &[f32],
            _sample_rate: u32,
            _config: &SttConfig,
        ) -> anyhow::Result<String> {
            Ok("offline fallback".into())
        }
    }

    fn secs(n: f32) -> usize {
        (n * SR as f32) as usize
    }

    fn sha256(path: &Path) -> String {
        let bytes = std::fs::read(path).expect("read online fixture asset");
        hex::encode(Sha256::digest(bytes))
    }

    fn fixture_root() -> PathBuf {
        std::env::var_os(ONLINE_FIXTURE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!(
                    "{ONLINE_FIXTURE_ENV} must point to the pinned local online-STT fixture when this ignored test is explicitly run"
                )
            })
    }

    fn fixture_audio(root: &Path, file: &str) -> (Vec<f32>, u32) {
        let mut reader = hound::WavReader::open(root.join(file)).expect("open WAV fixture");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "{file} must be mono");
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        assert_eq!(spec.bits_per_sample, 16);
        let samples = reader
            .samples::<i16>()
            .map(|sample| sample.expect("read WAV sample") as f32 / 32768.0)
            .collect();
        (samples, spec.sample_rate)
    }

    fn reference_text(root: &Path, file: &str) -> String {
        let contents = std::fs::read_to_string(root.join("trans.txt")).expect("read transcript");
        contents
            .lines()
            .find_map(|line| line.strip_prefix(file).map(str::trim))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| panic!("trans.txt has no reference for {file}"))
            .to_string()
    }

    fn normalized_tokens(text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|word| {
                word.chars()
                    .filter(|ch| ch.is_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase()
            })
            .filter(|word| !word.is_empty())
            .collect()
    }

    fn matches_reference_tail(actual: &[String], expected: &[String]) -> bool {
        let tail_len = expected.len().min(3);
        tail_len > 0
            && actual
                .windows(tail_len)
                .any(|window| window == &expected[expected.len() - tail_len..])
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

    #[test]
    fn online_model_layout_requires_all_transducer_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = OnlineTransducerModelPaths::from_dir(dir.path());
        assert!(!paths.models_exist(), "an empty directory is not a model");

        std::fs::write(dir.path().join(ONLINE_TOKENS_FILE), b"fixture").unwrap();
        assert!(
            !paths.models_exist(),
            "tokens alone are not an online model"
        );
        for file in [ONLINE_ENCODER_FILE, ONLINE_DECODER_FILE, ONLINE_JOINER_FILE] {
            std::fs::write(dir.path().join(file), b"fixture").unwrap();
        }
        assert!(paths.models_exist());
    }

    #[test]
    fn online_constructor_rejects_missing_assets_before_runtime_load() {
        let dir = tempfile::tempdir().unwrap();
        let error = SherpaOnlineTransducerStt::new(dir.path(), 2)
            .err()
            .expect("missing model must reject");
        assert!(error.to_string().contains("online STT model is incomplete"));
    }

    #[test]
    fn online_stream_enforces_sample_rate_chunk_and_finite_bounds() {
        assert!(validate_online_sample_rate(ONLINE_SAMPLE_RATE).is_ok());
        assert!(validate_online_sample_rate(8_000).is_err());
        assert!(validate_online_chunk(&[0.0, -0.2, 0.2], 0).is_ok());
        assert!(validate_online_chunk(&[f32::NAN], 0).is_err());
        assert!(validate_online_chunk(&vec![0.0; MAX_ONLINE_CHUNK_SAMPLES + 1], 0).is_err());
        assert!(validate_online_chunk(&[0.0], MAX_ONLINE_TURN_SAMPLES).is_err());
    }

    #[test]
    fn online_decode_error_uses_existing_batch_fallback() {
        let fallback = FakeBatchFallback;
        let text = transcribe_online_or_batch(
            || anyhow::bail!("online fixture failure"),
            Some(&fallback),
            &[0.0; 4],
            SR,
            &SttConfig::default(),
        )
        .expect("the existing offline provider remains available");
        assert_eq!(text, "offline fallback");
    }

    /// Explicit local-model gate. It never downloads assets and is excluded
    /// from normal CI; an explicit `--ignored` run without the env is a setup
    /// error rather than a silent pass.
    #[test]
    #[ignore]
    fn real_online_fixture_streams_and_batches() {
        let root = fixture_root();
        for (file, expected) in ONLINE_FIXTURE_FILES {
            let path = root.join(file);
            assert_eq!(sha256(&path), expected, "unexpected SHA-256 for {file}");
        }

        let provider = SherpaOnlineTransducerStt::new(&root, 2).expect("load online fixture");
        for file in ["0.wav", "1.wav"] {
            let expected = reference_text(&root, file);
            let expected_tokens = normalized_tokens(&expected);
            let (samples, sample_rate) = fixture_audio(&root, file);
            assert_eq!(sample_rate, SR, "{file} must be 16 kHz");

            let capability = provider
                .streaming_capability()
                .expect("online provider must expose streaming capability");
            let mut session = capability
                .start_stream(sample_rate, &SttConfig::default(), 7001)
                .expect("start online stream");
            let started = std::time::Instant::now();
            let mut partials = Vec::new();
            for chunk in samples.chunks(SR as usize / 5) {
                for event in session.push_audio(chunk).expect("push online audio") {
                    match event {
                        StreamingSttEvent::Partial { generation, text } => {
                            assert_eq!(generation, 7001);
                            if !text.is_empty() {
                                partials.push(text);
                            }
                        }
                        StreamingSttEvent::Final { .. } => {
                            panic!("online provider emitted a final before finish for {file}")
                        }
                    }
                }
            }
            assert!(
                !partials.is_empty(),
                "{file} emitted no incremental partial"
            );
            assert!(
                partials.windows(2).all(|window| window[0] != window[1]),
                "{file} emitted duplicate unchanged partials"
            );

            let finish_events = session.finish().expect("finish online stream");
            let finals: Vec<String> = finish_events
                .into_iter()
                .filter_map(|event| match event {
                    StreamingSttEvent::Final { generation, text } => {
                        assert_eq!(generation, 7001);
                        Some(text)
                    }
                    StreamingSttEvent::Partial { .. } => None,
                })
                .collect();
            assert_eq!(finals.len(), 1, "{file} must emit exactly one final");
            assert!(
                session
                    .push_audio(&[0.0; 320])
                    .expect("late audio after finish")
                    .is_empty(),
                "{file} emitted output after finish"
            );

            let actual = finals.into_iter().next().unwrap();
            let actual_tokens = normalized_tokens(&actual);
            let exact_match = actual_tokens == expected_tokens;
            let tail_match = matches_reference_tail(&actual_tokens, &expected_tokens);
            let rtf = started.elapsed().as_secs_f64()
                / (samples.len() as f64 / sample_rate as f64).max(f64::EPSILON);
            eprintln!(
                "online fixture={file} duration_s={:.3} partials={} rtf={rtf:.3} exact_reference_match={exact_match} reference_tail_match={tail_match} expected={expected:?} actual={actual:?}",
                samples.len() as f64 / sample_rate as f64,
                partials.len(),
            );
            assert!(
                !actual_tokens.is_empty(),
                "{file} final was empty; expected reference tail {expected:?}"
            );
        }

        // `transcribe` must exercise the same provider over >2 s by chunking
        // internally, rather than passing an oversized native push.
        let (long_samples, sample_rate) = fixture_audio(&root, "1.wav");
        let expected = normalized_tokens(&reference_text(&root, "1.wav"));
        let started = std::time::Instant::now();
        let batch_text = provider
            .transcribe(&long_samples, sample_rate, &SttConfig::default())
            .expect("online batch transcription");
        let batch_tokens = normalized_tokens(&batch_text);
        let batch_rtf = started.elapsed().as_secs_f64()
            / (long_samples.len() as f64 / sample_rate as f64).max(f64::EPSILON);
        let batch_tail_match = matches_reference_tail(&batch_tokens, &expected);
        eprintln!(
            "online batch fixture=1.wav duration_s={:.3} rtf={batch_rtf:.3} reference_tail_match={batch_tail_match} expected={expected:?} actual={batch_text:?}",
            long_samples.len() as f64 / sample_rate as f64,
        );
        assert!(!batch_tokens.is_empty(), "online batch result was empty");

        // Unsupported-rate capture must still reach the existing batch
        // fallback when online STT is selected.
        let fallback_provider = SherpaOnlineTransducerStt::new(&root, 2)
            .expect("load online fixture for fallback")
            .with_batch_fallback(Arc::new(FakeBatchFallback));
        assert_eq!(
            fallback_provider
                .transcribe(&long_samples, 8_000, &SttConfig::default())
                .expect("offline fallback for unsupported online rate"),
            "offline fallback"
        );
    }

    #[test]
    fn online_model_path_stays_in_existing_voice_store_without_kws_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let paths = VoiceModelPaths {
            stt_model_dir: dir.path().join("sherpa-onnx-moonshine-tiny-en-int8"),
        };
        assert_eq!(
            paths.online_stt_model_dir(),
            dir.path().join("sherpa-onnx-streaming-zipformer-en")
        );
        assert!(!paths.online_models_exist());
    }
}
