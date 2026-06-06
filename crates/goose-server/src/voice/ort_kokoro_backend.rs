//! Standalone Kokoro TTS via ort + misaki-rs (GPL-clean shipping backend).
//!
//! This is the SHIPPING TTS backend. It loads the Kokoro ONNX model directly
//! via the `ort` crate, with `misaki-rs` (default-features=false, no espeak)
//! for G2P phonemization. No sherpa-onnx dependency in this path.
//!
//! Measured: 0.24x realtime on Apple Silicon (CPU provider, release build).

use super::provider::{AudioOutput, TextToSpeech, TtsConfig};
use anyhow::{bail, Context};
use misaki_rs::{Language, G2P};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SAMPLE_RATE: u32 = 24000;
const MAX_PHONEME_LENGTH: usize = 510;

/// The Kokoro phoneme vocabulary: IPA character → token ID.
/// Extracted from kokoro-onnx Python DEFAULT_VOCAB (authoritative source).
fn build_vocab() -> HashMap<char, i64> {
    [
        (';', 1),
        (':', 2),
        (',', 3),
        ('.', 4),
        ('!', 5),
        ('?', 6),
        ('\u{2014}', 9),  // —
        ('\u{2026}', 10), // …
        ('"', 11),
        ('(', 12),
        (')', 13),
        ('\u{201C}', 14), // "
        ('\u{201D}', 15), // "
        (' ', 16),
        ('\u{0303}', 17), // ̃
        ('\u{02A3}', 18), // ʣ
        ('\u{02A5}', 19), // ʥ
        ('\u{02A6}', 20), // ʦ
        ('\u{02A8}', 21), // ʨ
        ('\u{1D5D}', 22), // ᵝ
        ('\u{AB67}', 23), // ꭧ
        ('A', 24),
        ('I', 25),
        ('O', 31),
        ('Q', 33),
        ('S', 35),
        ('T', 36),
        ('W', 39),
        ('Y', 41),
        ('\u{1D4A}', 42), // ᵊ
        ('a', 43),
        ('b', 44),
        ('c', 45),
        ('d', 46),
        ('e', 47),
        ('f', 48),
        ('h', 50),
        ('i', 51),
        ('j', 52),
        ('k', 53),
        ('l', 54),
        ('m', 55),
        ('n', 56),
        ('o', 57),
        ('p', 58),
        ('q', 59),
        ('r', 60),
        ('s', 61),
        ('t', 62),
        ('u', 63),
        ('v', 64),
        ('w', 65),
        ('x', 66),
        ('y', 67),
        ('z', 68),
        ('\u{0251}', 69),  // ɑ
        ('\u{0250}', 70),  // ɐ
        ('\u{0252}', 71),  // ɒ
        ('\u{00E6}', 72),  // æ
        ('\u{03B2}', 75),  // β
        ('\u{0254}', 76),  // ɔ
        ('\u{0255}', 77),  // ɕ
        ('\u{00E7}', 78),  // ç
        ('\u{0256}', 80),  // ɖ
        ('\u{00F0}', 81),  // ð
        ('\u{02A4}', 82),  // ʤ
        ('\u{0259}', 83),  // ə
        ('\u{025A}', 85),  // ɚ
        ('\u{025B}', 86),  // ɛ
        ('\u{025C}', 87),  // ɜ
        ('\u{025F}', 90),  // ɟ
        ('\u{0261}', 92),  // ɡ
        ('\u{0265}', 99),  // ɥ
        ('\u{0268}', 101), // ɨ
        ('\u{026A}', 102), // ɪ
        ('\u{029D}', 103), // ʝ
        ('\u{026F}', 110), // ɯ
        ('\u{0270}', 111), // ɰ
        ('\u{014B}', 112), // ŋ
        ('\u{0273}', 113), // ɳ
        ('\u{0272}', 114), // ɲ
        ('\u{0274}', 115), // ɴ
        ('\u{00F8}', 116), // ø
        ('\u{0278}', 118), // ɸ
        ('\u{03B8}', 119), // θ
        ('\u{0153}', 120), // œ
        ('\u{0279}', 123), // ɹ
        ('\u{027E}', 125), // ɾ
        ('\u{027B}', 126), // ɻ
        ('\u{0281}', 128), // ʁ
        ('\u{027D}', 129), // ɽ
        ('\u{0282}', 130), // ʂ
        ('\u{0283}', 131), // ʃ
        ('\u{0288}', 132), // ʈ
        ('\u{02A7}', 133), // ʧ
        ('\u{028A}', 135), // ʊ
        ('\u{028B}', 136), // ʋ
        ('\u{028C}', 138), // ʌ
        ('\u{0263}', 139), // ɣ
        ('\u{0264}', 140), // ɤ
        ('\u{03C7}', 142), // χ
        ('\u{028E}', 143), // ʎ
        ('\u{0292}', 147), // ʒ
        ('\u{0294}', 148), // ʔ
        ('\u{02C8}', 156), // ˈ
        ('\u{02CC}', 157), // ˌ
        ('\u{02D0}', 158), // ː
        ('\u{02B0}', 162), // ʰ
        ('\u{02B2}', 164), // ʲ
        ('\u{2193}', 169), // ↓
        ('\u{2192}', 171), // →
        ('\u{2197}', 172), // ↗
        ('\u{2198}', 173), // ↘
        ('\u{1D7B}', 177), // ᵻ
    ]
    .into_iter()
    .collect()
}

/// Split text into sentences for chunked TTS synthesis.
/// Splits on sentence-ending punctuation (.!?) followed by whitespace.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if (ch == '.' || ch == '!' || ch == '?') && current.len() > 5 {
            // Look for sentence boundary (punct followed by space or end)
            sentences.push(current.trim().to_string());
            current = String::new();
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }
    if sentences.is_empty() {
        sentences.push(text.to_string());
    }
    sentences
}

fn phonemes_to_tokens(phonemes: &str, vocab: &HashMap<char, i64>) -> Vec<i64> {
    let mut tokens: Vec<i64> = Vec::new();
    tokens.push(0); // start pad
    for ch in phonemes.chars() {
        // Skip zero-width joiner (U+200D) inserted by misaki-rs in diphthongs
        if ch == '\u{200D}' {
            continue;
        }
        if let Some(&id) = vocab.get(&ch) {
            tokens.push(id);
        }
        // Skip chars not in vocab (matches Python behavior)
    }
    tokens.push(0); // end pad
    if tokens.len() > MAX_PHONEME_LENGTH + 2 {
        tokens.truncate(MAX_PHONEME_LENGTH + 2);
    }
    tokens
}

/// Voice style vectors loaded from voices-v1.0.bin (NPZ format).
struct VoiceStyles {
    /// voice_name → style tensor [510, 1, 256]
    styles: HashMap<String, Vec<f32>>,
    style_dim: usize,
}

impl VoiceStyles {
    fn load(path: &Path) -> anyhow::Result<Self> {
        // The voices file is a raw f32 binary with a known layout per voice.
        // For the kokoro-onnx NPZ format, we'd use npyz. But the sherpa-onnx
        // voices.bin is raw floats. We support both by trying NPZ first.
        //
        // For the shipping path, we use the kokoro-onnx voices-v1.0.bin (NPZ).
        // This is downloaded separately from the sherpa-onnx model files.
        //
        // Fallback: if the file can't be parsed as NPZ, treat it as a single
        // flat f32 array for a default voice.
        let data = std::fs::read(path).context("Failed to read voices file")?;

        // NPZ files start with PK (zip magic bytes)
        if data.len() >= 2 && data[0] == 0x50 && data[1] == 0x4B {
            Self::load_npz(path)
        } else {
            // Raw f32 binary — treat as single default voice
            let floats: Vec<f32> = data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            let mut styles = HashMap::new();
            styles.insert("default".to_string(), floats);
            Ok(Self {
                styles,
                style_dim: 256,
            })
        }
    }

    fn load_npz(path: &Path) -> anyhow::Result<Self> {
        // NPZ is a zip of .npy files. Each .npy has a header + raw data.
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file).context("Failed to open voices NPZ")?;

        let mut styles = HashMap::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().trim_end_matches(".npy").to_string();

            // Read .npy: skip header, read f32 data
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)?;

            // Parse minimal .npy header: magic \x93NUMPY + version + header_len
            if buf.len() < 10 || &buf[0..6] != b"\x93NUMPY" {
                continue;
            }
            let header_len = u16::from_le_bytes([buf[8], buf[9]]) as usize;
            let data_start = 10 + header_len;
            if data_start >= buf.len() {
                continue;
            }

            let floats: Vec<f32> = buf[data_start..]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            styles.insert(name, floats);
        }

        Ok(Self {
            styles,
            style_dim: 256,
        })
    }

    /// Get the style vector for a voice at a given style index.
    /// NPZ shape: [510, 1, 256] flattened → index by style_index * 256.
    fn get_style(&self, voice: &str, style_index: usize) -> anyhow::Result<Vec<f32>> {
        let data = self
            .styles
            .get(voice)
            .ok_or_else(|| anyhow::anyhow!("Voice '{}' not found", voice))?;

        let offset = style_index * self.style_dim;
        if offset + self.style_dim > data.len() {
            bail!(
                "Style vector out of range: voice={}, style_index={}, offset={}, data_len={}, style_dim={}",
                voice, style_index, offset, data.len(), self.style_dim
            );
        }
        Ok(data[offset..offset + self.style_dim].to_vec())
    }

    fn voice_names(&self) -> Vec<&str> {
        self.styles.keys().map(|s| s.as_str()).collect()
    }
}

/// Standalone Kokoro TTS backend: ort + misaki-rs, GPL-clean.
///
/// The ONNX session requires `&mut self` for `run()`, so we wrap it in a Mutex.
/// Model is loaded ONCE at startup and reused across utterances.
pub struct OrtKokoroTts {
    session: Mutex<ort::session::Session>,
    g2p: Mutex<G2P>,
    vocab: HashMap<char, i64>,
    voices: VoiceStyles,
    default_voice: String,
}

impl OrtKokoroTts {
    pub fn new(model_path: &Path, voices_path: &Path, default_voice: &str) -> anyhow::Result<Self> {
        // CPU provider — measured faster than CoreML for this model (0.24x vs 0.28x).
        let session = ort::session::Session::builder()
            .map_err(|e| anyhow::anyhow!("ort session builder: {}", e))?
            .with_inter_threads(4)
            .map_err(|e| anyhow::anyhow!("ort inter_threads: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| anyhow::anyhow!("ort intra_threads: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| anyhow::anyhow!("ort load model: {}", e))?;

        let g2p = G2P::new(Language::EnglishGB);

        let voices = VoiceStyles::load(voices_path).context("Failed to load voice styles")?;

        tracing::info!(
            target: "permagentd::voice",
            "OrtKokoroTts: loaded model, {} voices, default={}, threads=inter:4/intra:4, provider=CPU",
            voices.voice_names().len(),
            default_voice
        );

        Ok(Self {
            session: Mutex::new(session),
            g2p: Mutex::new(g2p),
            vocab: build_vocab(),
            voices,
            default_voice: default_voice.to_string(),
        })
    }
}

impl TextToSpeech for OrtKokoroTts {
    fn synthesize(&self, text: &str, config: &TtsConfig) -> anyhow::Result<AudioOutput> {
        // Split text into sentences and synthesize each separately.
        // Kokoro inference time scales superlinearly with token count —
        // 300 tokens takes ~25s vs 116 tokens at ~2s. Chunking keeps
        // each piece in the fast (<2s) regime.
        let sentences = split_sentences(text);
        tracing::debug!(
            target: "permagentd::voice",
            "TTS: text_len={} sentences={}",
            text.len(),
            sentences.len()
        );

        let mut all_samples: Vec<f32> = Vec::new();
        let voice_name = config.voice_id.as_deref().unwrap_or(&self.default_voice);

        for (i, sentence) in sentences.iter().enumerate() {
            if sentence.trim().is_empty() {
                continue;
            }

            let chunk_start = std::time::Instant::now();

            // G2P
            let phonemes = {
                let g2p = self
                    .g2p
                    .lock()
                    .map_err(|e| anyhow::anyhow!("G2P lock: {}", e))?;
                let (phonemes, _tokens) = g2p.g2p(sentence)?;
                phonemes
            };

            // Tokenize
            let tokens = phonemes_to_tokens(&phonemes, &self.vocab);
            let style_index = (tokens.len().saturating_sub(2)).min(509);

            tracing::debug!(
                target: "permagentd::voice",
                "TTS chunk {}/{}: \"{}\" → {} tokens",
                i + 1, sentences.len(), &sentence[..sentence.len().min(40)], tokens.len()
            );

            // Style vector
            let style = self.voices.get_style(voice_name, style_index)?;

            // ONNX inference
            use ort::value::Tensor;
            let token_val = Tensor::from_array(([1usize, tokens.len()], tokens))
                .map_err(|e| anyhow::anyhow!("ort token tensor: {}", e))?;
            let style_val = Tensor::from_array(([1usize, style.len()], style))
                .map_err(|e| anyhow::anyhow!("ort style tensor: {}", e))?;
            let speed_val = Tensor::from_array(([1usize], vec![config.speed]))
                .map_err(|e| anyhow::anyhow!("ort speed tensor: {}", e))?;

            let mut session = self
                .session
                .lock()
                .map_err(|e| anyhow::anyhow!("Session lock: {}", e))?;

            let outputs = session
                .run(ort::inputs![
                    "tokens" => token_val,
                    "style" => style_val,
                    "speed" => speed_val,
                ])
                .map_err(|e| anyhow::anyhow!("ort run: {}", e))?;

            let (_shape, raw_data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow::anyhow!("extract tensor: {}", e))?;
            let chunk_samples: Vec<f32> = raw_data.to_vec();

            let chunk_ms = chunk_start.elapsed().as_millis();
            let chunk_dur = chunk_samples.len() as f32 / SAMPLE_RATE as f32;
            tracing::debug!(
                target: "permagentd::voice",
                "TTS chunk {}/{}: {}ms ({:.1}s audio, {:.2}x realtime)",
                i + 1, sentences.len(), chunk_ms, chunk_dur,
                chunk_ms as f32 / 1000.0 / chunk_dur.max(0.01)
            );

            all_samples.extend_from_slice(&chunk_samples);
        }

        Ok(AudioOutput {
            samples: all_samples,
            sample_rate: SAMPLE_RATE,
        })
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }
}

/// Model paths for the standalone Kokoro TTS backend.
pub struct OrtKokoroModelPaths {
    pub model_path: PathBuf,
    pub voices_path: PathBuf,
}

impl OrtKokoroModelPaths {
    pub fn default_paths() -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("permagent")
            .join("models")
            .join("voice");
        Self {
            model_path: base.join("kokoro-v1.0.onnx"),
            voices_path: base.join("voices-v1.0.bin"),
        }
    }

    pub fn models_exist(&self) -> bool {
        self.model_path.exists() && self.voices_path.exists()
    }
}
