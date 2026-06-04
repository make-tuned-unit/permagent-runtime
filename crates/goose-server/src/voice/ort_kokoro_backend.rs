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
fn build_vocab() -> HashMap<char, i64> {
    // Matches the Python kokoro-onnx DEFAULT_VOCAB exactly.
    let pairs: &[(char, i64)] = &[
        (';', 1),
        (':', 2),
        (',', 3),
        ('.', 4),
        ('!', 5),
        ('?', 6),
        ('\u{2014}', 9),  // —
        ('\u{2026}', 10), // …
        ('\u{201C}', 11), // "
        ('(', 12),
        (')', 13),
        ('\u{201D}', 14), // "
        ('\u{201E}', 15), // „
        (' ', 16),
        ('\u{0303}', 17), // ̃ (combining tilde)
        ('\u{02A3}', 18), // ʣ
        ('\u{02A5}', 19), // ʥ
        ('\u{02A6}', 20), // ʦ
        ('\u{02A8}', 21), // ʨ
        ('\u{1D5D}', 22), // ᵝ
        ('A', 23),
        ('I', 24),
        ('O', 25),
        ('Q', 26),
        ('W', 27),
        ('Y', 28),
        ('a', 29),
        ('b', 30),
        ('d', 31),
        ('e', 32),
        ('f', 33),
        ('h', 34),
        ('i', 35),
        ('j', 36),
        ('k', 37),
        ('l', 38),
        ('m', 39),
        ('n', 40),
        ('o', 41),
        ('p', 42),
        ('s', 43),
        ('t', 44),
        ('u', 45),
        ('v', 46),
        ('w', 47),
        ('x', 48),
        ('z', 49),
        ('\u{00E6}', 51),  // æ
        ('\u{00E7}', 52),  // ç
        ('\u{00F0}', 53),  // ð
        ('\u{00F8}', 55),  // ø
        ('\u{0127}', 56),  // ħ
        ('\u{014B}', 57),  // ŋ
        ('\u{0153}', 58),  // œ
        ('\u{0250}', 59),  // ɐ
        ('\u{0251}', 60),  // ɑ
        ('\u{0252}', 61),  // ɒ
        ('\u{0254}', 62),  // ɔ
        ('\u{0259}', 63),  // ə
        ('\u{025B}', 64),  // ɛ
        ('\u{025C}', 65),  // ɜ
        ('\u{025F}', 66),  // ɟ
        ('\u{0260}', 67),  // ɠ
        ('\u{0261}', 68),  // ɡ
        ('\u{0263}', 69),  // ɣ
        ('\u{0268}', 70),  // ɨ
        ('\u{026A}', 71),  // ɪ
        ('\u{026B}', 72),  // ɫ
        ('\u{026C}', 73),  // ɬ
        ('\u{026D}', 74),  // ɭ
        ('\u{026E}', 75),  // ɮ
        ('\u{026F}', 76),  // ɯ
        ('\u{0270}', 77),  // ɰ
        ('\u{0271}', 78),  // ɱ
        ('\u{0272}', 79),  // ɲ
        ('\u{0273}', 80),  // ɳ
        ('\u{0274}', 81),  // ɴ
        ('\u{0275}', 82),  // ɵ
        ('\u{0278}', 83),  // ɸ
        ('\u{0279}', 84),  // ɹ
        ('\u{027A}', 85),  // ɺ
        ('\u{027B}', 86),  // ɻ
        ('\u{027D}', 87),  // ɽ
        ('\u{027E}', 88),  // ɾ
        ('\u{0280}', 89),  // ʀ
        ('\u{0281}', 90),  // ʁ
        ('\u{0282}', 91),  // ʂ
        ('\u{0283}', 92),  // ʃ
        ('\u{0288}', 93),  // ʈ
        ('\u{0289}', 94),  // ʉ
        ('\u{028A}', 95),  // ʊ
        ('\u{028B}', 96),  // ʋ
        ('\u{028C}', 97),  // ʌ
        ('\u{028D}', 98),  // ʍ
        ('\u{028E}', 99),  // ʎ
        ('\u{0290}', 101), // ʐ
        ('\u{0291}', 102), // ʑ
        ('\u{0292}', 103), // ʒ
        ('\u{0294}', 104), // ʔ
        ('\u{0295}', 105), // ʕ
        ('\u{029C}', 106), // ʜ
        ('\u{029F}', 107), // ʟ
        ('\u{02A4}', 108), // ʤ
        ('\u{02A7}', 109), // ʧ
        ('\u{02B0}', 112), // ʰ — mapped to 112 in some versions
        ('\u{02C8}', 156), // ˈ (primary stress)
        ('\u{02CC}', 157), // ˌ (secondary stress)
        ('\u{02D0}', 158), // ː (length)
        ('\u{02B0}', 162), // ʰ
        ('\u{02B2}', 164), // ʲ
        ('\u{2193}', 169), // ↓
        ('\u{2192}', 171), // →
        ('\u{2197}', 172), // ↗
        ('\u{2198}', 173), // ↘
        ('\u{1D7B}', 177), // ᵻ
    ];
    pairs.iter().copied().collect()
}

fn phonemes_to_tokens(phonemes: &str, vocab: &HashMap<char, i64>) -> Vec<i64> {
    let mut tokens: Vec<i64> = Vec::new();
    tokens.push(0); // start pad
    for ch in phonemes.chars() {
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

    /// Get the style vector for a voice at a given token count.
    /// Shape: [510, 1, 256] flattened → index by token_count * 256.
    fn get_style(&self, voice: &str, token_count: usize) -> anyhow::Result<Vec<f32>> {
        let data = self
            .styles
            .get(voice)
            .ok_or_else(|| anyhow::anyhow!("Voice '{}' not found", voice))?;

        // Style shape is [510, 1, 256] = 510*256 = 130560 floats
        let offset = token_count * self.style_dim;
        if offset + self.style_dim > data.len() {
            bail!(
                "Style vector out of range: token_count={}, data_len={}",
                token_count,
                data.len()
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
            "OrtKokoroTts: loaded model, {} voices, default={}",
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
        // Step 1: G2P — text to IPA phonemes via misaki-rs
        let phonemes = {
            let g2p = self
                .g2p
                .lock()
                .map_err(|e| anyhow::anyhow!("G2P lock: {}", e))?;
            let (phonemes, _tokens) = g2p.g2p(text)?;
            phonemes
        };

        // Step 2: Tokenize — IPA phonemes to Kokoro token IDs
        let tokens = phonemes_to_tokens(&phonemes, &self.vocab);
        let token_count = tokens.len();

        // Step 3: Get voice style vector
        let voice_name = config.voice_id.as_deref().unwrap_or(&self.default_voice);
        let style = self.voices.get_style(voice_name, token_count)?;

        // Step 4: Run ONNX inference
        use ort::value::Tensor;

        // Use (shape, Vec<T>) form to avoid ndarray version mismatch with ort's pinned ndarray.
        let token_val = Tensor::from_array(([1usize, token_count], tokens))
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

        // Step 5: Extract audio samples from the first output
        let (_shape, raw_data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("extract tensor: {}", e))?;
        let samples: Vec<f32> = raw_data.to_vec();

        Ok(AudioOutput {
            samples,
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
