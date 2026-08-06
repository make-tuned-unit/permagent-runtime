//! On-device wake-word + spoken-stop detection via sherpa-onnx keyword
//! spotting.
//!
//! The engine is the KWS zipformer transducer (gigaspeech, ~3.3M params,
//! Apache-2.0) run through the same sherpa-onnx runtime that already powers
//! STT — no new inference dependency. Audio never leaves the machine: the
//! spotter consumes the mic monitor frames the client streams over the
//! existing `/voice` WebSocket and emits only "a keyword fired" events.
//!
//! Open vocabulary: phrases are sentencepiece-encoded at runtime by
//! [`super::bpe`], so the wake phrase follows the persona name ("hey henry")
//! with no per-phrase model work. Every keyword line carries an `@` tag naming
//! its semantic kind, which the spotter echoes back on detection — the
//! detection result IS the routing decision, no string matching on token
//! output.

use super::bpe::BpeVocab;
use anyhow::Context;
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};
use std::path::{Path, PathBuf};

/// Detection kinds, embedded as `@` tags in keyword lines and echoed back by
/// the spotter on detection.
pub const KIND_WAKE: &str = "wake";
pub const KIND_STOP: &str = "stop";

/// Pinned KWS model release asset (sherpa-onnx kws-models release,
/// Apache-2.0). Digest verified 2026-08 against the release's own
/// checksum.txt AND an independent local hash of the downloaded asset.
pub const KWS_MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2";
pub const KWS_MODEL_SHA256: &str =
    "f170013b4716e41b62b9bfd809687c207cef798ef9bc6534d524e17af9b6561a";
pub const KWS_MODEL_BYTES: u64 = 17_626_723;
/// DownloadManager key for the wake-word model download.
pub const KWS_DOWNLOAD_ID: &str = "kws-wake-word";
/// Directory name inside the tarball (and under the voice models dir).
pub const KWS_MODEL_DIR_NAME: &str = "sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01";

/// Spoken phrases that end the agent's turn. Multiple respellings of the same
/// intent — all map to [`KIND_STOP`].
const STOP_PHRASES: &[&str] = &["stop", "okay stop", "stop talking"];

/// Wake-word model paths.
#[derive(Clone, Debug)]
pub struct WakeWordModelPaths {
    pub model_dir: PathBuf,
}

impl WakeWordModelPaths {
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("permagent")
            .join("models")
            .join("voice");
        Self {
            model_dir: base.join(KWS_MODEL_DIR_NAME),
        }
    }

    /// Where the release tarball is downloaded before extraction.
    pub fn tarball_path(&self) -> PathBuf {
        self.model_dir
            .parent()
            .unwrap_or(&self.model_dir)
            .join(format!("{KWS_MODEL_DIR_NAME}.tar.bz2"))
    }

    pub fn models_exist(&self) -> bool {
        [
            "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            "tokens.txt",
            "bpe.model",
        ]
        .iter()
        .all(|f| self.model_dir.join(f).exists())
    }
}

/// Extract the downloaded release tarball into the voice models directory.
/// The archive's top-level directory is [`KWS_MODEL_DIR_NAME`], so unpacking
/// into the parent lands the model at the expected path. The tarball is
/// removed on success.
pub fn install_from_tarball(paths: &WakeWordModelPaths) -> anyhow::Result<()> {
    let tarball = paths.tarball_path();
    let dest = paths
        .model_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("model dir has no parent"))?;
    let file =
        std::fs::File::open(&tarball).with_context(|| format!("open {}", tarball.display()))?;
    let decoder = bzip2::read::BzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    // tar::Archive::unpack refuses path traversal outside `dest` by default.
    archive
        .unpack(dest)
        .with_context(|| format!("extract {} into {}", tarball.display(), dest.display()))?;
    if !paths.models_exist() {
        anyhow::bail!(
            "KWS tarball extracted but expected files are missing in {}",
            paths.model_dir.display()
        );
    }
    let _ = std::fs::remove_file(&tarball);
    Ok(())
}

/// A keyword detection: which semantic kind fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    Wake,
    Stop,
}

/// Shared keyword-spotting engine (one per daemon, hot-swappable after the
/// on-demand model download). Streams are created per voice connection.
pub struct WakeWordSpotter {
    spotter: KeywordSpotter,
    vocab: BpeVocab,
}

/// Format one keywords line: tokens, optional boost, and the `@kind` tag the
/// spotter echoes back on detection.
fn keyword_line(tokens: &str, boost: Option<f32>, kind: &str) -> String {
    match boost {
        Some(b) => format!("{tokens} :{b} @{kind}"),
        None => format!("{tokens} @{kind}"),
    }
}

impl WakeWordSpotter {
    /// Load the spotter from `model_dir`. Writes the generated stop-phrase
    /// keywords file next to the model (the engine requires a non-empty
    /// config-level keywords file; per-connection wake phrases are added at
    /// stream creation).
    pub fn new(model_dir: &Path, num_threads: i32) -> anyhow::Result<Self> {
        let vocab = BpeVocab::load(&model_dir.join("bpe.model"))?;

        let mut stop_lines = Vec::new();
        for phrase in STOP_PHRASES {
            match vocab.encode_phrase(phrase) {
                Some(tokens) => stop_lines.push(keyword_line(&tokens, None, KIND_STOP)),
                None => tracing::warn!(
                    target: "permagentd::voice",
                    "stop phrase {phrase:?} not encodable — skipped"
                ),
            }
        }
        if stop_lines.is_empty() {
            anyhow::bail!("no stop phrase could be encoded from the KWS vocabulary");
        }
        let keywords_path = model_dir.join("keywords-permagent.txt");
        std::fs::write(&keywords_path, stop_lines.join("\n") + "\n")
            .with_context(|| format!("write {}", keywords_path.display()))?;

        let prefix = "epoch-12-avg-2-chunk-16-left-64.int8.onnx";
        let mut config = KeywordSpotterConfig::default();
        config.model_config.transducer.encoder = Some(
            model_dir
                .join(format!("encoder-{prefix}"))
                .to_string_lossy()
                .into(),
        );
        config.model_config.transducer.decoder = Some(
            model_dir
                .join(format!("decoder-{prefix}"))
                .to_string_lossy()
                .into(),
        );
        config.model_config.transducer.joiner = Some(
            model_dir
                .join(format!("joiner-{prefix}"))
                .to_string_lossy()
                .into(),
        );
        config.model_config.tokens = Some(model_dir.join("tokens.txt").to_string_lossy().into());
        config.model_config.num_threads = num_threads;
        config.keywords_file = Some(keywords_path.to_string_lossy().into());

        let spotter = KeywordSpotter::create(&config).ok_or_else(|| {
            anyhow::anyhow!("failed to create keyword spotter (bad config or model files)")
        })?;
        Ok(Self { spotter, vocab })
    }

    /// Open a detection session for one voice connection. `wake_phrases` are
    /// encoded at runtime (open vocabulary); un-encodable phrases are dropped.
    /// The session detects the union of the wake phrases and the built-in stop
    /// phrases. Errors if NO wake phrase survives encoding — the caller falls
    /// back to VAD-only hands-free rather than silently listening for nothing.
    pub fn create_session(&self, wake_phrases: &[String]) -> anyhow::Result<WakeSession> {
        let mut lines = Vec::new();
        for phrase in wake_phrases {
            match self.vocab.encode_phrase(phrase) {
                // Boost the wake phrase: it opens the mic, so favor recall.
                Some(tokens) => lines.push(keyword_line(&tokens, Some(2.0), KIND_WAKE)),
                None => tracing::warn!(
                    target: "permagentd::voice",
                    "wake phrase {phrase:?} not encodable — skipped"
                ),
            }
        }
        if lines.is_empty() {
            anyhow::bail!("no wake phrase could be encoded: {wake_phrases:?}");
        }
        // Per-stream keywords are '/'-separated; the engine merges them with
        // the config-level (stop) keywords file.
        let stream = self.spotter.create_stream_with_keywords(&lines.join("/"));
        Ok(WakeSession { stream })
    }

    /// Feed mic samples into a session; returns the first detection, if any.
    /// Resets the detector state after a hit so spotting continues.
    pub fn accept(
        &self,
        session: &WakeSession,
        sample_rate: u32,
        samples: &[f32],
    ) -> Option<Detection> {
        session.stream.accept_waveform(sample_rate as i32, samples);
        let mut hit = None;
        while self.spotter.is_ready(&session.stream) {
            self.spotter.decode(&session.stream);
            if let Some(result) = self.spotter.get_result(&session.stream) {
                if !result.keyword.is_empty() {
                    self.spotter.reset(&session.stream);
                    // First hit wins; keep decoding to drain buffered audio.
                    hit.get_or_insert(match result.keyword.as_str() {
                        KIND_STOP => Detection::Stop,
                        _ => Detection::Wake,
                    });
                }
            }
        }
        hit
    }
}

/// Per-connection keyword-spotting stream.
pub struct WakeSession {
    stream: OnlineStream,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_line_formats() {
        assert_eq!(
            keyword_line("\u{2581}HE Y", Some(2.0), KIND_WAKE),
            "\u{2581}HE Y :2 @wake"
        );
        assert_eq!(
            keyword_line("\u{2581}ST O P", None, KIND_STOP),
            "\u{2581}ST O P @stop"
        );
    }

    /// The pinned model URL must satisfy the DownloadManager's strict policy
    /// and the digest must be well-formed, or the wake-word download endpoint
    /// is dead on arrival.
    #[test]
    fn kws_url_and_digest_pass_download_policy() {
        permagent::download_manager::validate_download_url(KWS_MODEL_URL)
            .expect("KWS model URL must be allowlisted");
        assert_eq!(KWS_MODEL_SHA256.len(), 64);
        assert!(KWS_MODEL_SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    /// End-to-end against the real model when installed (skips otherwise):
    /// the spotter must detect a runtime-encoded open-vocabulary phrase in the
    /// model's own test audio, and stay silent on audio without it.
    #[test]
    fn detects_keyword_in_reference_audio() {
        let paths = WakeWordModelPaths::default_paths();
        if !paths.models_exist() {
            eprintln!("skipping: KWS model not installed");
            return;
        }
        let spotter = WakeWordSpotter::new(&paths.model_dir, 2).expect("spotter loads");

        // test_wavs/0.wav transcript contains "LIGHT UP" phrases (the model's
        // own reference keywords); use one as an open-vocab "wake" phrase.
        let wav_path = paths.model_dir.join("test_wavs/0.wav");
        if !wav_path.exists() {
            eprintln!("skipping: reference wav not present");
            return;
        }
        let mut reader = hound::WavReader::open(&wav_path).expect("open wav");
        let sr = reader.spec().sample_rate;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        let session = spotter
            .create_session(&["light up".to_string()])
            .expect("session with open-vocab phrase");
        let mut detected = false;
        for chunk in samples.chunks(1600) {
            if spotter.accept(&session, sr, chunk) == Some(Detection::Wake) {
                detected = true;
                break;
            }
        }
        assert!(detected, "expected 'light up' detection in reference audio");
    }

    /// Full detection matrix on real synthesized speech (installed Kokoro
    /// TTS): the stop phrase rides the CONFIG-LEVEL keywords file and the wake
    /// phrase the PER-STREAM keywords — two different engine paths — while
    /// ordinary speech must trip neither. Skips when either model set is
    /// absent (CI).
    #[test]
    fn detects_wake_and_stop_in_synthesized_audio() {
        use crate::voice::provider::{TextToSpeech, TtsConfig};

        let paths = WakeWordModelPaths::default_paths();
        let kokoro = crate::voice::ort_kokoro_backend::OrtKokoroModelPaths::default_paths();
        if !paths.models_exist() || !kokoro.models_exist() {
            eprintln!("skipping: KWS or Kokoro models not installed");
            return;
        }
        let tts = crate::voice::ort_kokoro_backend::OrtKokoroTts::new(
            &kokoro.model_path,
            &kokoro.voices_path,
            "bm_lewis",
        )
        .expect("kokoro loads");

        // Linear resample to the spotter's 16k and pad with trailing silence
        // so the final decode frames flush.
        let to_16k = |audio: &crate::voice::provider::AudioOutput| -> Vec<f32> {
            let src_rate = audio.sample_rate as f32;
            let n_out = (audio.samples.len() as f32 * 16_000.0 / src_rate) as usize;
            let last = audio.samples.len() - 1;
            let mut samples: Vec<f32> = (0..n_out)
                .map(|i| {
                    let pos = i as f32 * src_rate / 16_000.0;
                    let j = (pos as usize).min(last);
                    let frac = pos - j as f32;
                    let a = audio.samples[j];
                    let b = audio.samples[(j + 1).min(last)];
                    a + (b - a) * frac
                })
                .collect();
            samples.extend(std::iter::repeat_n(0.0f32, 8_000));
            samples
        };

        let spotter = WakeWordSpotter::new(&paths.model_dir, 2).expect("spotter loads");

        // (utterance, wake phrase for the session, expected detection)
        let cases: &[(&str, Option<Detection>)] = &[
            // The stop phrase rides the config-level keywords file.
            ("Stop.", Some(Detection::Stop)),
            // The shipping default wake phrase rides the per-stream keywords.
            ("Hey Henry.", Some(Detection::Wake)),
            // Ordinary speech must NOT trip either keyword.
            ("What a lovely morning outside.", None),
        ];
        for (utterance, expected) in cases {
            let audio = tts
                .synthesize(utterance, &TtsConfig::default())
                .expect("synthesize");
            let samples = to_16k(&audio);
            let session = spotter
                .create_session(&["hey henry".to_string()])
                .expect("session");
            let mut hit = None;
            for chunk in samples.chunks(1600) {
                if let Some(d) = spotter.accept(&session, 16_000, chunk) {
                    hit = Some(d);
                    break;
                }
            }
            assert_eq!(hit, *expected, "unexpected detection for {utterance:?}");
        }
    }
}
