//! sherpa-onnx development backend for STT (Moonshine) and TTS (Kokoro).
//!
//! This is the Phase 1 / internal development backend. The shipping TTS backend
//! will be standalone Kokoro via ort + misaki-rs (GPL-clean). The provider
//! abstraction ensures the swap is a config change, not a refactor.

use super::provider::{AudioOutput, SpeechToText, SttConfig, TextToSpeech, TtsConfig};
use anyhow::Context;
use sherpa_onnx::{
    GenerationConfig, OfflineMoonshineModelConfig, OfflineModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
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

        let recognizer = OfflineRecognizer::create(&config)
            .context("Failed to create Moonshine recognizer")?;
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

/// sherpa-onnx Kokoro TTS backend.
pub struct SherpaKokoroTts {
    tts: OfflineTts,
    native_sample_rate: u32,
}

impl SherpaKokoroTts {
    /// Create a new Kokoro TTS engine.
    /// `model_dir` should contain: model.onnx, voices.bin, tokens.txt,
    /// espeak-ng-data/, dict/, lexicon-us-en.txt
    pub fn new(model_dir: &Path, num_threads: i32) -> anyhow::Result<Self> {
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: Some(model_dir.join("model.onnx").to_string_lossy().into()),
                    voices: Some(model_dir.join("voices.bin").to_string_lossy().into()),
                    tokens: Some(model_dir.join("tokens.txt").to_string_lossy().into()),
                    data_dir: Some(model_dir.join("espeak-ng-data").to_string_lossy().into()),
                    dict_dir: Some(model_dir.join("dict").to_string_lossy().into()),
                    lexicon: Some(model_dir.join("lexicon-us-en.txt").to_string_lossy().into()),
                    lang: Some("en".into()),
                    ..Default::default()
                },
                num_threads,
                ..Default::default()
            },
            ..Default::default()
        };

        let tts = OfflineTts::create(&config).context("Failed to create Kokoro TTS engine")?;
        let native_sample_rate = tts.sample_rate() as u32;
        Ok(Self {
            tts,
            native_sample_rate,
        })
    }
}

impl TextToSpeech for SherpaKokoroTts {
    fn synthesize(&self, text: &str, config: &TtsConfig) -> anyhow::Result<AudioOutput> {
        // Note: the pronunciation lexicon seam is in config.lexicon but the
        // sherpa-onnx dev backend ignores it — sherpa-onnx handles G2P internally.
        // The shipping ort+misaki backend will use the lexicon.
        let sid = config
            .voice_id
            .as_ref()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);

        let gen_config = GenerationConfig {
            speed: config.speed,
            sid,
            ..Default::default()
        };

        let audio = self
            .tts
            .generate_with_config(text, &gen_config, None::<fn(&[f32], f32) -> bool>)
            .context("Kokoro TTS synthesis failed")?;

        Ok(AudioOutput {
            samples: audio.samples().to_vec(),
            sample_rate: audio.sample_rate() as u32,
        })
    }

    fn sample_rate(&self) -> u32 {
        self.native_sample_rate
    }
}

/// Paths configuration for voice models.
#[derive(Clone, Debug)]
pub struct VoiceModelPaths {
    pub stt_model_dir: PathBuf,
    pub tts_model_dir: PathBuf,
}

impl VoiceModelPaths {
    /// Default model paths under ~/.permagent/models/voice/
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("permagent")
            .join("models")
            .join("voice");
        Self {
            stt_model_dir: base.join("sherpa-onnx-moonshine-tiny-en-int8"),
            tts_model_dir: base.join("kokoro-multi-lang-v1_0"),
        }
    }

    pub fn models_exist(&self) -> bool {
        self.stt_model_dir.join("tokens.txt").exists()
            && self.tts_model_dir.join("model.onnx").exists()
    }
}

pub type VoiceProviderPair = (Box<dyn SpeechToText>, Box<dyn TextToSpeech>);

/// Create the STT and TTS providers from model paths.
/// Returns None if models are not downloaded yet.
pub fn create_providers(
    paths: &VoiceModelPaths,
    num_threads: i32,
) -> anyhow::Result<Option<VoiceProviderPair>> {
    if !paths.models_exist() {
        tracing::info!(
            target: "permagentd::voice",
            "Voice models not found at {} / {} — voice disabled",
            paths.stt_model_dir.display(),
            paths.tts_model_dir.display()
        );
        return Ok(None);
    }

    tracing::info!(target: "permagentd::voice", "Loading voice models...");

    let stt = SherpaMoonshineStt::new(&paths.stt_model_dir, num_threads)
        .context("Failed to load Moonshine STT")?;

    let tts = SherpaKokoroTts::new(&paths.tts_model_dir, num_threads)
        .context("Failed to load Kokoro TTS")?;

    tracing::info!(
        target: "permagentd::voice",
        "Voice models loaded (STT: Moonshine, TTS: Kokoro @ {}Hz)",
        tts.sample_rate()
    );

    Ok(Some((Box::new(stt), Box::new(tts))))
}
