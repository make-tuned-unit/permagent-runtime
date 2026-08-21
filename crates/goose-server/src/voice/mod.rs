//! Voice provider abstraction and implementations.
//!
//! The provider trait is deliberately decoupled from any specific backend
//! (sherpa-onnx, standalone ort+misaki, cloud APIs) so the TTS and STT
//! backends can be swapped via config, not code. This is load-bearing:
//! the shipping TTS backend will differ from the development backend.

pub mod bpe;
pub mod compound;
pub mod kws;
pub mod loudness;
pub mod oov_log;
pub mod ort_kokoro_backend;
pub mod proper_noun_corrector;
pub mod prosody;
pub mod provider;
pub mod sherpa_backend;
pub mod speakable;
pub mod spoken_verdict;
pub mod user_lexicon;

pub use provider::{SpeechToText, TextToSpeech};
