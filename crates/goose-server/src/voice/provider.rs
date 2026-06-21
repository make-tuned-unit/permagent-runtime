//! Voice provider traits — the abstraction boundary between the voice loop
//! and any specific STT/TTS backend.
//!
//! Implementations live in sibling modules (sherpa_backend, future ort_backend).
//! The pronunciation lexicon seam is designed here for Phase 2 injection.

use std::collections::HashMap;

/// Audio data returned from TTS synthesis.
pub struct AudioOutput {
    /// PCM samples, mono, f32 normalized [-1, 1].
    pub samples: Vec<f32>,
    /// Sample rate in Hz (e.g. 24000 for Kokoro).
    pub sample_rate: u32,
}

/// A pronunciation override map: surface-form word (or short phrase) → phoneme
/// string. Consulted BEFORE the backend's default G2P dictionary, so proper
/// nouns and acronyms the G2P mishears ("Claude" → "Cloud", "API" → "appy")
/// are spoken correctly.
///
/// Keys are lowercased; values are in the backend's phoneme format (IPA for the
/// shipping ort+misaki Kokoro backend). A backend that doesn't support phoneme
/// injection ignores the lexicon.
#[derive(Default, Clone)]
pub struct PronunciationLexicon {
    /// lowercased word/phrase → phoneme representation (backend-specific format)
    pub entries: HashMap<String, String>,
}

impl PronunciationLexicon {
    /// Build a lexicon from `(surface, phonemes)` pairs. Surfaces are lowercased
    /// so lookup is case-insensitive.
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (S, S)>,
        S: Into<String>,
    {
        Self {
            entries: pairs
                .into_iter()
                .map(|(k, v)| (k.into().to_lowercase(), v.into()))
                .collect(),
        }
    }

    /// Look up the phoneme override for `word_or_phrase` (case-insensitive).
    pub fn get(&self, word_or_phrase: &str) -> Option<&str> {
        self.entries
            .get(&word_or_phrase.to_lowercase())
            .map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Largest key word-count, for greedy longest-phrase matching.
    pub fn max_phrase_words(&self) -> usize {
        self.entries
            .keys()
            .map(|k| k.split_whitespace().count())
            .max()
            .unwrap_or(0)
    }
}

/// Configuration for a TTS synthesis call.
pub struct TtsConfig {
    /// Voice/speaker ID (backend-specific, e.g. speaker index for Kokoro).
    pub voice_id: Option<String>,
    /// Speech speed multiplier (1.0 = normal).
    pub speed: f32,
    /// Per-call pronunciation overrides, consulted before the backend default
    /// G2P. `None` lets the backend use its own built-in technical lexicon.
    pub lexicon: Option<PronunciationLexicon>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            voice_id: None,
            speed: 1.0,
            lexicon: None,
        }
    }
}

/// Configuration for an STT transcription call.
#[derive(Default)]
#[allow(dead_code)]
pub struct SttConfig {
    /// Language hint (e.g. "en").
    pub language: Option<String>,
}

/// Speech-to-text provider.
pub trait SpeechToText: Send + Sync {
    /// Transcribe audio samples to text.
    /// `samples`: mono f32 PCM, normalized [-1, 1].
    /// `sample_rate`: sample rate in Hz (e.g. 16000).
    fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
        config: &SttConfig,
    ) -> anyhow::Result<String>;
}

/// Text-to-speech provider.
/// The lexicon seam is in TtsConfig — backends that support it (e.g. the
/// shipping ort+misaki backend) consult config.lexicon before their default
/// G2P. Backends that don't (e.g. sherpa-onnx dev backend) ignore it.
pub trait TextToSpeech: Send + Sync {
    /// Synthesize text to audio.
    fn synthesize(&self, text: &str, config: &TtsConfig) -> anyhow::Result<AudioOutput>;

    /// The native sample rate of this TTS backend.
    #[allow(dead_code)]
    fn sample_rate(&self) -> u32;

    /// All selectable voice keys (e.g. "bf_emma"), for the picker roster.
    fn list_voices(&self) -> Vec<String>;
}
