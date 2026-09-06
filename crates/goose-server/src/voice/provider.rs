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

/// A generation-scoped update from an incremental speech recognizer.
///
/// Partials are provisional and may be replaced until the first final update.
/// The generation is carried on the event deliberately: a producer can finish
/// after a socket has closed, and the consumer must be able to reject that
/// output without relying on timing or a shared "current turn" variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamingSttEvent {
    Partial { generation: u64, text: String },
    Final { generation: u64, text: String },
}

impl StreamingSttEvent {
    pub fn partial(generation: u64, text: impl Into<String>) -> Self {
        Self::Partial {
            generation,
            text: text.into(),
        }
    }

    pub fn final_text(generation: u64, text: impl Into<String>) -> Self {
        Self::Final {
            generation,
            text: text.into(),
        }
    }

    pub fn generation(&self) -> u64 {
        match self {
            Self::Partial { generation, .. } | Self::Final { generation, .. } => *generation,
        }
    }
}

/// A provider-side incremental STT session.
///
/// Implementations own their recognizer and may return zero or more updates
/// for each audio push. They must not obtain audio by calling the batch
/// [`SpeechToText::transcribe`] method on a growing buffer: that would duplicate
/// inference and cannot safely define ownership when stop races a worker.
pub trait StreamingSttSession: Send {
    /// Feed one captured PCM chunk and return any recognizer updates ready to
    /// be consumed. Chunks are never retained by this contract after the call.
    fn push_audio(&mut self, samples: &[f32]) -> anyhow::Result<Vec<StreamingSttEvent>>;

    /// Close the input and return the recognizer's authoritative result. The
    /// consumer still applies [`StreamingSttGate`] so a faulty provider cannot
    /// emit more than one final or resurrect a cancelled generation.
    fn finish(&mut self) -> anyhow::Result<Vec<StreamingSttEvent>>;

    /// Stop work and discard any result that has not already crossed the
    /// consumer boundary. This is best effort for provider implementations;
    /// the generation gate remains the final stale-output fence.
    fn cancel(&mut self);
}

/// Optional streaming capability implemented by providers with a supported
/// online model and bounded local assets.
pub trait StreamingSpeechToText: Send + Sync {
    fn start_stream(
        &self,
        sample_rate: u32,
        config: &SttConfig,
        generation: u64,
    ) -> anyhow::Result<Box<dyn StreamingSttSession>>;
}

/// Consumer-side ordering and cancellation fence for one stream generation.
///
/// The route/capture worker can feed every provider event to this gate. Partial
/// updates are coalesced: only the newest pending partial is exposed by
/// [`take_partial`](Self::take_partial). A final clears pending partial state,
/// and all subsequent updates—including updates from another generation—are
/// ignored.
#[derive(Debug)]
pub struct StreamingSttGate {
    generation: u64,
    cancelled: bool,
    final_emitted: bool,
    pending_partial: Option<String>,
}

impl StreamingSttGate {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            cancelled: false,
            final_emitted: false,
            pending_partial: None,
        }
    }

    /// Accept an update. Partials are buffered until `take_partial`; the first
    /// final is returned immediately and all later updates are discarded.
    pub fn accept(&mut self, event: StreamingSttEvent) -> Option<StreamingSttEvent> {
        if self.cancelled || self.final_emitted || event.generation() != self.generation {
            return None;
        }

        match event {
            StreamingSttEvent::Partial { text, .. } => {
                self.pending_partial = Some(text);
                None
            }
            StreamingSttEvent::Final { text, .. } => {
                self.pending_partial = None;
                self.final_emitted = true;
                Some(StreamingSttEvent::Final {
                    generation: self.generation,
                    text,
                })
            }
        }
    }

    /// Take the newest provisional update, if one is pending.
    pub fn take_partial(&mut self) -> Option<StreamingSttEvent> {
        if self.cancelled || self.final_emitted {
            return None;
        }
        self.pending_partial
            .take()
            .map(|text| StreamingSttEvent::Partial {
                generation: self.generation,
                text,
            })
    }

    /// Invalidate this generation before closing a socket or interrupting a
    /// turn. This is intentionally separate from provider cancellation.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.pending_partial = None;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
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

    /// Return the optional online capability. Existing and offline batch
    /// providers keep the default `None`, so the voice route can retain its
    /// final-only batch behavior until a supported streaming model is loaded.
    fn streaming_capability(&self) -> Option<&dyn StreamingSpeechToText> {
        None
    }
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

    /// Convert plain text to this backend's phoneme representation.
    ///
    /// This is what makes pronunciation teachable. Asking a language model for
    /// Kokoro-flavoured IPA asks it to author an encoding it cannot hear, so it
    /// has no way to notice being wrong — and it was: the only entry ever saved
    /// that way stored "permagent" as ipa "pʌmˈeɪdʒənt" / sounds_like
    /// "PUM-ay-jent", i.e. self-consistent and confidently wrong (the product
    /// is "PER-ma-jent"). Nothing in the system could detect that, because
    /// there is no ground truth to compare IPA against.
    ///
    /// Running a human RESPELLING ("per ma jent", "prop tech") through the same
    /// G2P that speaks removes the guesswork: the stored phonemes are by
    /// construction exactly what will be said, and the respelling itself is
    /// something a reader can sanity-check at a glance.
    ///
    /// Default: unsupported, so backends without a G2P seam (the sherpa dev
    /// backend) degrade honestly instead of silently storing nothing.
    fn phonemize_text(&self, _text: &str) -> anyhow::Result<String> {
        anyhow::bail!("this TTS backend cannot derive phonemes from text")
    }

    /// Words in `text` the backend would have to guess at (spell, or a
    /// last-resort split). Used to coach the model *before* it speaks a name
    /// the user just said, rather than discovering the spelling live.
    fn unresolved_words(&self, _text: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeStreamingProvider;

    struct FakeStreamingSession {
        updates: VecDeque<Vec<StreamingSttEvent>>,
        final_updates: Vec<StreamingSttEvent>,
        cancelled: bool,
    }

    impl StreamingSttSession for FakeStreamingSession {
        fn push_audio(&mut self, _samples: &[f32]) -> anyhow::Result<Vec<StreamingSttEvent>> {
            if self.cancelled {
                return Ok(Vec::new());
            }
            Ok(self.updates.pop_front().unwrap_or_default())
        }

        fn finish(&mut self) -> anyhow::Result<Vec<StreamingSttEvent>> {
            if self.cancelled {
                return Ok(Vec::new());
            }
            Ok(std::mem::take(&mut self.final_updates))
        }

        fn cancel(&mut self) {
            self.cancelled = true;
        }
    }

    impl StreamingSpeechToText for FakeStreamingProvider {
        fn start_stream(
            &self,
            _sample_rate: u32,
            _config: &SttConfig,
            generation: u64,
        ) -> anyhow::Result<Box<dyn StreamingSttSession>> {
            Ok(Box::new(FakeStreamingSession {
                updates: VecDeque::from([
                    vec![StreamingSttEvent::partial(generation, "hel")],
                    vec![StreamingSttEvent::partial(generation, "hello")],
                ]),
                final_updates: vec![StreamingSttEvent::final_text(generation, "hello world")],
                cancelled: false,
            }))
        }
    }

    impl SpeechToText for FakeStreamingProvider {
        fn transcribe(
            &self,
            _samples: &[f32],
            _sample_rate: u32,
            _config: &SttConfig,
        ) -> anyhow::Result<String> {
            Ok("hello world".to_string())
        }

        fn streaming_capability(&self) -> Option<&dyn StreamingSpeechToText> {
            Some(self)
        }
    }

    struct BatchOnlyProvider;

    impl SpeechToText for BatchOnlyProvider {
        fn transcribe(
            &self,
            _samples: &[f32],
            _sample_rate: u32,
            _config: &SttConfig,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn streaming_provider_coalesces_partials_and_emits_one_final() {
        let provider = FakeStreamingProvider;
        let capability = provider
            .streaming_capability()
            .expect("fake provider exposes streaming capability");
        let mut session = capability
            .start_stream(16_000, &SttConfig::default(), 41)
            .expect("stream starts");
        let mut gate = StreamingSttGate::new(41);

        for event in session.push_audio(&[0.1, 0.2]).unwrap() {
            assert!(gate.accept(event).is_none());
        }
        for event in session.push_audio(&[0.3, 0.4]).unwrap() {
            assert!(gate.accept(event).is_none());
        }
        assert_eq!(
            gate.take_partial(),
            Some(StreamingSttEvent::partial(41, "hello"))
        );
        assert!(gate.take_partial().is_none());

        let final_event = session.finish().unwrap().pop().expect("one final");
        assert_eq!(
            gate.accept(final_event.clone()),
            Some(StreamingSttEvent::final_text(41, "hello world"))
        );
        assert!(gate.accept(final_event).is_none());
        assert!(gate.take_partial().is_none());
    }

    #[test]
    fn stale_generation_and_cancelled_stream_cannot_publish_late_output() {
        let mut gate = StreamingSttGate::new(7);
        assert!(gate.accept(StreamingSttEvent::partial(6, "old")).is_none());
        assert!(gate
            .accept(StreamingSttEvent::partial(7, "current"))
            .is_none());
        gate.cancel();
        assert!(gate.take_partial().is_none());
        assert!(gate
            .accept(StreamingSttEvent::final_text(7, "late current"))
            .is_none());
        assert!(gate
            .accept(StreamingSttEvent::final_text(8, "late replacement"))
            .is_none());

        let provider = FakeStreamingProvider;
        let capability = provider.streaming_capability().unwrap();
        let mut session = capability
            .start_stream(16_000, &SttConfig::default(), 9)
            .unwrap();
        session.cancel();
        assert!(session.push_audio(&[0.5]).unwrap().is_empty());
        assert!(session.finish().unwrap().is_empty());
    }

    #[test]
    fn empty_final_is_still_one_authoritative_result() {
        let mut gate = StreamingSttGate::new(12);
        assert_eq!(
            gate.accept(StreamingSttEvent::final_text(12, "")),
            Some(StreamingSttEvent::final_text(12, ""))
        );
        assert!(gate
            .accept(StreamingSttEvent::final_text(12, "should be ignored"))
            .is_none());
    }

    #[test]
    fn batch_only_provider_keeps_explicit_offline_fallback() {
        let provider = BatchOnlyProvider;
        assert!(provider.streaming_capability().is_none());
    }
}
