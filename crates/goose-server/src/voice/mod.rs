//! Voice provider abstraction and implementations.
//!
//! The provider trait is deliberately decoupled from any specific backend
//! (sherpa-onnx, standalone ort+misaki, cloud APIs) so the TTS and STT
//! backends can be swapped via config, not code. This is load-bearing:
//! the shipping TTS backend will differ from the development backend.

pub mod provider;
pub mod sherpa_backend;

pub use provider::{SpeechToText, TextToSpeech};
