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

impl SpeechToText for SherpaMoonshineStt {
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        _config: &SttConfig,
    ) -> anyhow::Result<String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, samples);
        self.recognizer.decode(&stream);
        let result = stream
            .get_result()
            .context("Moonshine: failed to get recognition result")?;
        Ok(result.text.trim().to_string())
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
