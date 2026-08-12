//! Standalone Kokoro TTS via ort + misaki-rs (GPL-clean shipping backend).
//!
//! This is the SHIPPING TTS backend. It loads the Kokoro ONNX model directly
//! via the `ort` crate, with `misaki-rs` (default-features=false, no espeak)
//! for G2P phonemization. No sherpa-onnx dependency in this path.
//!
//! Measured: 0.24x realtime on Apple Silicon (CPU provider, release build).

use super::provider::{AudioOutput, PronunciationLexicon, TextToSpeech, TtsConfig};
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

/// Built-in pronunciation overrides for technical terms misaki's G2P mishears.
///
/// Values are Kokoro IPA (the same alphabet `build_vocab` tokenizes); `ˈ`/`ˌ`
/// mark primary/secondary stress and `ː` marks length. Acronyms are spelled out
/// letter-by-letter so they aren't read as words ("API" → "appy", "URL" →
/// "earl"). Edit a value here to retune a term — no audio rebuild required.
fn technical_lexicon() -> PronunciationLexicon {
    PronunciationLexicon::from_pairs([
        // Product names — the headline fix: "Claude" must not become "Cloud".
        ("claude code", "klˈɔːd kˈəʊd"),
        ("claude", "klˈɔːd"),
        ("dropdown", "drˈɒpdaʊn"),
        // Coined product names — misaki/G2P spells these letter-by-letter or
        // mis-stresses them. Pronounce as words (#516).
        ("permagent", "pˈɜːməʤɛnt"),       // Per-ma-jent
        ("permagentd", "pˈɜːməʤɛnt dˈiː"), // the daemon: "Permagent-D"
        ("spectral", "spˈɛktrəl"),         // SPEK-truhl
        ("kinrows", "kˈɪnrəʊz"),           // KIN-rohz
        // ── Corrupt misaki dictionary entries ──
        // These words ARE in misaki's dictionary, so the OOV compound splitter
        // never sees them — they must be overridden by hand. Each value below
        // is the G2P output for the correct respelling.
        //
        // "coworking" ships as kˈaʊɜːkɪŋ — the vowel of COW and no /w/ at all,
        // i.e. "cow-erking". Its parts are both right (co → kˈəʊ,
        // working → wˈɜːkɪŋ), which is exactly the respelling fix.
        ("coworking", "kˈəʊwˈɜːkɪŋ"),
        ("coworkings", "kˈəʊwˈɜːkɪŋz"),
        ("co-working", "kˈəʊwˈɜːkɪŋ"),
        // "repo" ships as ɹˈiːpQ — a literal capital Q, which is not a phoneme.
        ("repo", "ɹˈiːpəʊ"),
        ("repos", "ɹˈiːpəʊz"),
        // ── Seeds: OOV with no safe compound split ──
        // Verified against the gb dictionaries: none of these decompose, so
        // without an entry each is spelled out letter by letter.
        ("sqlite", "ˌɛskjuːˌɛlˈaɪt"), // "S-Q-L-ite", how it is said aloud
        ("xterm", "ˈɛkstɜːm"),        // "ex-term"
        ("kubernetes", "kˌuːbənˈɛtiːz"),
        ("agritech", "ˈaɡrɪtɛk"),
        ("kuzu", "kˈuːzuː"),
        ("neon", "nˈiːɒn"),
        ("proptech", "prˈɒptɛk"), // resolves by split too; pinned for demos
        // Acronyms, spelled out letter-by-letter.
        ("api", "ˌeɪpˌiːˈaɪ"),
        ("url", "jˌuːˌɑːrˈɛl"),
        ("cli", "sˌiːˌɛlˈaɪ"),
        ("uuid", "jˌuːjˌuːˌaɪdˈiː"),
    ])
}

/// Combine the built-in technical pronunciations with per-call user entries.
/// User pronunciations win for matching keys without hiding unrelated seeds.
fn effective_lexicon(
    seeded: &PronunciationLexicon,
    user: Option<&PronunciationLexicon>,
) -> PronunciationLexicon {
    let mut effective = seeded.clone();
    if let Some(user) = user {
        effective.entries.extend(user.entries.clone());
    }
    effective
}

/// A planned phoneme segment: either a verbatim override pulled from the
/// lexicon, or raw text still to be run through misaki G2P.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    /// Override phonemes (already in IPA), used verbatim.
    Override(String),
    /// Source text to be phonemized by G2P.
    Text(String),
}

/// Lowercase a token with leading/trailing ASCII punctuation stripped, for
/// case- and punctuation-insensitive lexicon matching.
fn match_key(token: &str) -> String {
    token
        .trim_matches(|c: char| c.is_ascii_punctuation())
        .to_lowercase()
}

/// Split `sentence` into segments, replacing whole-word (and multi-word) lexicon
/// hits with `Override` phonemes and leaving everything else as `Text` for G2P.
/// Greedy longest-phrase match so "claude code" wins over "claude".
///
/// Pure (no G2P) so it can be unit-tested without the model. When the sentence
/// contains no lexicon term the result is a single `Text` segment — the caller's
/// common path stays byte-identical to plain G2P.
fn plan_segments(sentence: &str, lexicon: &PronunciationLexicon) -> Vec<Segment> {
    if lexicon.is_empty() {
        return vec![Segment::Text(sentence.to_string())];
    }
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let max_phrase = lexicon.max_phrase_words().max(1);

    let mut segments: Vec<Segment> = Vec::new();
    let mut pending: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        // Try the longest phrase first, down to a single word.
        let mut matched = None;
        let upper = max_phrase.min(words.len() - i);
        for len in (1..=upper).rev() {
            let key = words[i..i + len]
                .iter()
                .map(|w| match_key(w))
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(phonemes) = lexicon.get(&key) {
                matched = Some((len, phonemes.to_string()));
                break;
            }
        }
        if let Some((len, phonemes)) = matched {
            if !pending.is_empty() {
                segments.push(Segment::Text(pending.join(" ")));
                pending.clear();
            }
            // Preserve any trailing punctuation on the last matched word so
            // sentence-final pauses survive ("...Claude Code." keeps the ".").
            let last = words[i + len - 1];
            let trailing: String = last
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_punctuation())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            segments.push(Segment::Override(format!("{phonemes}{trailing}")));
            i += len;
        } else {
            pending.push(words[i]);
            i += 1;
        }
    }
    if !pending.is_empty() {
        segments.push(Segment::Text(pending.join(" ")));
    }
    segments
}

/// misaki's own dictionaries, as the compound splitter's oracle. `Lexicon`
/// exposes `golds`/`silvers` publicly, so this borrows the already-loaded maps
/// — no second copy of 390k entries.
struct MisakiDict<'a>(&'a G2P);

impl crate::voice::compound::WordDict for MisakiDict<'_> {
    fn in_gold(&self, word: &str) -> bool {
        self.0.lexicon.golds.contains_key(word)
    }
    fn known(&self, word: &str) -> bool {
        // is_known() also covers misaki's own casing/symbol special cases, so a
        // word it can already handle is never needlessly split.
        self.0.lexicon.golds.contains_key(word)
            || self.0.lexicon.silvers.contains_key(word)
            || self.0.lexicon.is_known(word, "")
    }
}

/// Phonemize `sentence` to a single IPA string, consulting `lexicon` for
/// overrides before falling back to misaki G2P for the rest.
///
/// Out-of-vocabulary compounds are decomposed BEFORE G2P (see voice::compound):
/// without its espeak fallback misaki spells unknown words letter by letter, so
/// "proptech" arrived as "P-R-O-P-T-E-C-H". Rewriting the text — rather than
/// splicing phonemes together by hand — keeps misaki's tagging and stress
/// assignment in charge of the result.
///
/// Words that survive as unresolved are returned so the caller can log what
/// speech is still guessing at, instead of that only surfacing mid-demo.
fn phonemize(
    g2p: &G2P,
    sentence: &str,
    lexicon: &PronunciationLexicon,
) -> anyhow::Result<(String, Vec<String>)> {
    let mut out = String::new();
    let mut unresolved: Vec<String> = Vec::new();
    for seg in plan_segments(sentence, lexicon) {
        if !out.is_empty() {
            out.push(' ');
        }
        match seg {
            // A lexicon hit is authoritative — never second-guess a taught word.
            Segment::Override(p) => out.push_str(&p),
            Segment::Text(t) => {
                let (expanded, mut oov) =
                    crate::voice::compound::expand_compounds(&MisakiDict(g2p), &t);
                unresolved.append(&mut oov);
                let (phonemes, _tokens) = g2p.g2p(&expanded)?;
                out.push_str(&phonemes);
            }
        }
    }
    Ok((out, unresolved))
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
    /// Built-in technical-term pronunciation overrides, consulted before G2P
    /// unless a per-call `TtsConfig.lexicon` is supplied.
    lexicon: PronunciationLexicon,
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

        // 0b: fail LOUD if a repacked voices-v1.0.bin is missing the keys the
        // product depends on (the default voice + the seeded British female).
        // A silently-thinned pack would otherwise surface as a runtime "voice
        // not found" deep in the picker — assert it up front instead.
        let names: Vec<&str> = voices.voice_names();
        for required in [default_voice, "bf_emma"] {
            if !names.contains(&required) {
                tracing::error!(
                    target: "permagentd::voice",
                    "VOICE PACK INTEGRITY: required voice '{}' missing from voices-v1.0.bin ({} voices present) — \
                     the pack is incomplete or repacked; voice selection/preview will be degraded",
                    required,
                    names.len()
                );
            }
        }

        Ok(Self {
            session: Mutex::new(session),
            g2p: Mutex::new(g2p),
            vocab: build_vocab(),
            voices,
            default_voice: default_voice.to_string(),
            lexicon: technical_lexicon(),
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
        let lexicon = effective_lexicon(&self.lexicon, config.lexicon.as_ref());

        for (i, sentence) in sentences.iter().enumerate() {
            if sentence.trim().is_empty() {
                continue;
            }

            let chunk_start = std::time::Instant::now();

            // G2P (lexicon overrides consulted before misaki for each word)
            let t_g2p = std::time::Instant::now();
            let (phonemes, unresolved) = {
                let g2p = self
                    .g2p
                    .lock()
                    .map_err(|e| anyhow::anyhow!("G2P lock: {}", e))?;
                phonemize(&g2p, sentence, &lexicon)?
            };
            // Words speech is still guessing at. Logged, not silent: an
            // unresolved word WILL be spelled out letter by letter, and the
            // only alternative to recording it here is discovering it live.
            if !unresolved.is_empty() {
                crate::voice::oov_log::record(&unresolved);
                tracing::info!(
                    target: "permagentd::voice",
                    "PRONUNCIATION unresolved (spelled out): {}",
                    unresolved.join(", ")
                );
            }
            let g2p_ms = t_g2p.elapsed().as_millis();

            // Tokenize
            let t_tok = std::time::Instant::now();
            let tokens = phonemes_to_tokens(&phonemes, &self.vocab);
            // Captured before `tokens` is moved into the input tensor below —
            // the log used to hardcode 0 for this, so every chunk reported
            // "0 tokens" and the phoneme count was useless exactly where it
            // would diagnose a G2P failure.
            let token_count = tokens.len();
            let style_index = (tokens.len().saturating_sub(2)).min(509);
            let tok_ms = t_tok.elapsed().as_millis();

            // Style vector
            let t_style = std::time::Instant::now();
            let style = self.voices.get_style(voice_name, style_index)?;
            let style_ms = t_style.elapsed().as_millis();

            // ONNX inference
            use ort::value::Tensor;
            let t_tensor = std::time::Instant::now();
            let token_val = Tensor::from_array(([1usize, tokens.len()], tokens))
                .map_err(|e| anyhow::anyhow!("ort token tensor: {}", e))?;
            let style_val = Tensor::from_array(([1usize, style.len()], style))
                .map_err(|e| anyhow::anyhow!("ort style tensor: {}", e))?;
            let speed_val = Tensor::from_array(([1usize], vec![config.speed]))
                .map_err(|e| anyhow::anyhow!("ort speed tensor: {}", e))?;
            let tensor_ms = t_tensor.elapsed().as_millis();

            let t_lock = std::time::Instant::now();
            let mut session = self
                .session
                .lock()
                .map_err(|e| anyhow::anyhow!("Session lock: {}", e))?;
            let lock_ms = t_lock.elapsed().as_millis();

            let t_run = std::time::Instant::now();
            let outputs = session
                .run(ort::inputs![
                    "tokens" => token_val,
                    "style" => style_val,
                    "speed" => speed_val,
                ])
                .map_err(|e| anyhow::anyhow!("ort run: {}", e))?;
            let run_ms = t_run.elapsed().as_millis();

            let t_extract = std::time::Instant::now();
            let (_shape, raw_data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow::anyhow!("extract tensor: {}", e))?;
            let chunk_samples: Vec<f32> = raw_data.to_vec();
            let extract_ms = t_extract.elapsed().as_millis();

            let chunk_ms = chunk_start.elapsed().as_millis();
            let chunk_dur = chunk_samples.len() as f32 / SAMPLE_RATE as f32;
            let rtf = chunk_ms as f32 / 1000.0 / chunk_dur.max(0.01);
            tracing::info!(
                target: "permagentd::voice",
                "TTS chunk {}/{}: {}ms ({:.1}s audio, {:.2}x RTF) | \
                 g2p={}ms tok={}ms style={}ms tensor={}ms lock={}ms RUN={}ms extract={}ms | \
                 {} tokens \"{}\"",
                i + 1, sentences.len(), chunk_ms, chunk_dur, rtf,
                g2p_ms, tok_ms, style_ms, tensor_ms, lock_ms, run_ms, extract_ms,
                token_count,
                &sentence.chars().take(40).collect::<String>()
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

    fn list_voices(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .voices
            .voice_names()
            .iter()
            .map(|s| s.to_string())
            .collect();
        names.sort();
        names
    }

    /// Derive phonemes for a respelling, using the SAME G2P that speaks.
    ///
    /// Deliberately runs with an EMPTY lexicon: the point is to convert a
    /// respelling from first principles, and consulting the user lexicon here
    /// would let a previous (possibly wrong) entry for the same word feed back
    /// into its own replacement. Compound expansion still applies, so a
    /// respelling may itself contain an OOV compound.
    fn phonemize_text(&self, text: &str) -> anyhow::Result<String> {
        let g2p = self
            .g2p
            .lock()
            .map_err(|e| anyhow::anyhow!("G2P lock: {}", e))?;
        let (phonemes, unresolved) = phonemize(&g2p, text, &PronunciationLexicon::default())?;
        let phonemes = phonemes.trim().to_string();
        if phonemes.is_empty() {
            anyhow::bail!("'{text}' produced no phonemes — try a different respelling");
        }
        // A respelling is only as good as its parts being REAL words. Anything
        // unresolved here will be spelled letter by letter, so accepting it
        // would store exactly the defect this path exists to prevent: teaching
        // "permagent" as "per ma jent" yielded "per mah JAY-EE-EN-TEE", and the
        // save reported success. Reject with the offending part named, so the
        // caller can pick a real word ("gent" for "jent") and try again.
        if !unresolved.is_empty() {
            anyhow::bail!(
                "the respelling '{text}' contains {} that speech cannot pronounce and would \
                 spell out letter by letter: {}. Respell using REAL English words — e.g. \
                 'gent' not 'jent', 'purr' not 'per' — each of which is spoken as written",
                if unresolved.len() == 1 {
                    "a part"
                } else {
                    "parts"
                },
                unresolved.join(", ")
            );
        }
        // The unknown marker means misaki could not phonemize part of the
        // respelling; storing it would bake a "❓" into speech.
        if phonemes.contains(&g2p.unk) {
            anyhow::bail!(
                "'{text}' could not be pronounced — respell it using ordinary English words \
                 or syllables, e.g. 'prop tech' or 'per ma jent'"
            );
        }
        Ok(phonemes)
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Real-dictionary pronunciation tests ──
    //
    // These build an actual misaki G2P. That needs NO downloaded assets — the
    // dictionaries and tagger are embedded in the crate — so unlike the
    // synthesis test below they run in CI. `phonemize` is the whole
    // pronunciation decision, so this is where the reported bugs are pinned
    // against ground truth rather than against a hand-written fake.

    fn real_g2p() -> G2P {
        G2P::new(Language::EnglishGB)
    }

    /// Strip the zero-width joiners misaki emits inside vowel digraphs, so
    /// comparisons read the way the phonemes actually sound.
    fn clean(s: &str) -> String {
        s.replace('\u{200d}', "")
    }

    #[test]
    fn proptech_is_spoken_not_spelled() {
        let g2p = real_g2p();
        let lex = PronunciationLexicon::default();

        // Ground truth: what misaki does with the word UNTOUCHED. Without the
        // espeak fallback this is the letter-by-letter spelling that shipped.
        let (raw, _) = g2p.g2p("proptech").unwrap();
        // And what it produces for the decomposition.
        let (parts, _) = g2p.g2p("prop tech").unwrap();

        let (got, unresolved) = phonemize(&g2p, "proptech", &lex).unwrap();

        assert_eq!(
            clean(&got),
            clean(&parts),
            "expected the compound to phonemize as its parts"
        );
        assert_ne!(
            clean(&got),
            clean(&raw),
            "expected to DIFFER from the untouched (letter-spelled) form"
        );
        assert!(
            unresolved.is_empty(),
            "a resolved compound must not be reported unresolved: {unresolved:?}"
        );
    }

    #[test]
    fn common_product_compounds_resolve_against_the_real_dictionary() {
        let g2p = real_g2p();
        let lex = PronunciationLexicon::default();
        for (word, respelling) in [
            ("webhook", "web hook"),
            ("dogfood", "dog food"),
            ("changelog", "change log"),
            ("toolchain", "tool chain"),
            ("devops", "dev ops"),
        ] {
            let (got, _) = phonemize(&g2p, word, &lex).unwrap();
            let (want, _) = g2p.g2p(respelling).unwrap();
            assert_eq!(
                clean(&got),
                clean(&want),
                "{word} should sound like {respelling}"
            );
        }
    }

    /// "coworking" is IN misaki's dictionary and WRONG there — kˈaʊɜːkɪŋ, the
    /// vowel of COW with no /w/. Being "known", the compound splitter never
    /// touches it, so only the built-in override can fix it. This asserts the
    /// override is actually reached at synthesis.
    #[test]
    fn the_corrupt_coworking_entry_is_overridden() {
        let g2p = real_g2p();
        let (dictionary_form, _) = g2p.g2p("coworking").unwrap();
        assert!(
            clean(&dictionary_form).starts_with("kˈaʊ"),
            "upstream dictionary changed; re-check the override. got {}",
            clean(&dictionary_form)
        );

        let (spoken, _) = phonemize(&g2p, "coworking", &technical_lexicon()).unwrap();
        assert!(
            clean(&spoken).starts_with("kˈəʊ"),
            "coworking must be said with the vowel of COAT: got {}",
            clean(&spoken)
        );
        assert!(clean(&spoken).contains('w'), "and must contain a /w/");
    }

    /// A respelling is only as good as its parts being real words, and this is
    /// the signal `phonemize_text` rejects on. Found by live testing, not by the
    /// fake-dictionary tests: teaching "permagent" as "per ma jent" derived
    /// "pɜː mˈɑː dʒˈeɪ ˈiː ˈɛn tˈiː" — "per mah JAY-EE-EN-TEE" — because "jent"
    /// is not a word, and the save reported SUCCESS. Swapping in a real word
    /// ("gent") must resolve cleanly.
    #[test]
    fn a_respelling_built_from_non_words_is_detectable() {
        let g2p = real_g2p();
        let lex = PronunciationLexicon::default();

        let (_, bad) = phonemize(&g2p, "per ma jent", &lex).unwrap();
        assert!(
            bad.contains(&"jent".to_string()),
            "'jent' is not a word and must be reported so the save can be refused: {bad:?}"
        );

        let (_, good) = phonemize(&g2p, "per ma gent", &lex).unwrap();
        assert!(
            good.is_empty(),
            "a respelling of real words must resolve cleanly: {good:?}"
        );
    }

    /// A word with no safe split is reported so it reaches the review queue
    /// instead of being silently spelled out.
    #[test]
    fn an_unsplittable_unknown_word_is_reported() {
        let g2p = real_g2p();
        let (_, unresolved) = phonemize(&g2p, "zzqxwv", &PronunciationLexicon::default()).unwrap();
        assert_eq!(unresolved, vec!["zzqxwv".to_string()]);
    }

    /// A taught word wins over everything: dictionary, corruption and splitter.
    #[test]
    fn a_taught_pronunciation_is_authoritative() {
        let g2p = real_g2p();
        let lex = PronunciationLexicon::from_pairs([("proptech", "ZZZ")]);
        let (got, _) = phonemize(&g2p, "proptech.", &lex).unwrap();
        assert_eq!(
            got, "ZZZ.",
            "override applies and keeps sentence punctuation"
        );
    }

    /// An ordinary sentence must be untouched by any of this.
    #[test]
    fn plain_english_is_unaffected() {
        let g2p = real_g2p();
        let sentence = "I will open the project and read the notes aloud.";
        let (got, unresolved) =
            phonemize(&g2p, sentence, &PronunciationLexicon::default()).unwrap();
        let (want, _) = g2p.g2p(sentence).unwrap();
        assert_eq!(clean(&got), clean(&want));
        assert!(unresolved.is_empty(), "{unresolved:?}");
    }

    /// Proves `voice_id` actually routes through to Kokoro synthesis: two
    /// distinct non-default voices produce different waveforms for the same
    /// text, and a bogus key is rejected (so a silent fallback to the default
    /// voice can't masquerade as success). Loads the real ~325MB model, so
    /// `#[ignore]`d — run with the assets present:
    ///   cargo test -p permagent-daemon --lib ort_kokoro -- --ignored --nocapture
    #[test]
    #[ignore]
    fn voice_id_routes_through_to_kokoro() {
        let paths = OrtKokoroModelPaths::default_paths();
        assert!(
            paths.models_exist(),
            "voice models absent at {:?} — cannot run synthesis test",
            paths.model_path
        );
        let tts = OrtKokoroTts::new(&paths.model_path, &paths.voices_path, "bm_lewis")
            .expect("load Kokoro");

        let text = "Hello there, this is a voice routing test.";
        let cfg = |v: &str| TtsConfig {
            voice_id: Some(v.to_string()),
            speed: 1.0,
            lexicon: None,
        };

        // Two non-default, non-seed British females (default=bm_lewis, seed=bf_emma).
        let a = tts
            .synthesize(text, &cfg("bf_isabella"))
            .expect("synth bf_isabella");
        let b = tts
            .synthesize(text, &cfg("bf_alice"))
            .expect("synth bf_alice");

        assert!(!a.samples.is_empty(), "produced no audio");
        assert_eq!(a.sample_rate, 24_000, "unexpected Kokoro sample rate");
        assert_ne!(
            a.samples, b.samples,
            "voice_id not routing through — identical audio for different voices"
        );

        // A key absent from the pack must error, not silently fall back.
        assert!(
            tts.synthesize(text, &cfg("zz_not_a_voice")).is_err(),
            "bogus voice_id should be rejected"
        );
    }

    // ── Pronunciation lexicon (model-free) ──

    #[test]
    fn claude_code_resolves_in_lexicon() {
        let lex = technical_lexicon();
        // Case-insensitive lookup of the headline term.
        assert!(lex.get("Claude Code").is_some(), "Claude Code must resolve");
        assert_eq!(lex.get("Claude Code"), lex.get("claude code"));
        assert!(lex.get("claude").is_some());
    }

    #[test]
    fn user_lexicon_overlays_seeded_pronunciations() {
        let seeded = technical_lexicon();
        let unrelated_user =
            PronunciationLexicon::from_pairs([("unrelated", "user-unrelated-ipa")]);
        let effective = effective_lexicon(&seeded, Some(&unrelated_user));

        assert_eq!(
            effective.get("spectral"),
            seeded.get("spectral"),
            "an unrelated user entry must not hide seeded pronunciations"
        );

        let overriding_user =
            PronunciationLexicon::from_pairs([("permagent", "user-permagent-ipa")]);
        let effective = effective_lexicon(&seeded, Some(&overriding_user));
        assert_eq!(
            effective.get("permagent"),
            Some("user-permagent-ipa"),
            "a user pronunciation must override the matching seed"
        );
    }

    #[test]
    fn plan_substitutes_claude_code_phrase() {
        let lex = technical_lexicon();
        let segs = plan_segments("open Claude Code now", &lex);
        // Expect: Text("open") , Override(claude code) , Text("now")
        let claude_code = lex.get("claude code").unwrap().to_string();
        assert_eq!(
            segs,
            vec![
                Segment::Text("open".into()),
                Segment::Override(claude_code),
                Segment::Text("now".into()),
            ],
        );
    }

    #[test]
    fn plan_prefers_longest_phrase() {
        // "claude code" (2 words) wins over "claude" (1 word).
        let lex = technical_lexicon();
        let segs = plan_segments("Claude Code", &lex);
        assert_eq!(
            segs,
            vec![Segment::Override(lex.get("claude code").unwrap().into())],
        );
    }

    #[test]
    fn plan_preserves_trailing_punctuation() {
        let lex = technical_lexicon();
        let segs = plan_segments("use the API.", &lex);
        let api = lex.get("api").unwrap();
        assert_eq!(
            segs,
            vec![
                Segment::Text("use the".into()),
                Segment::Override(format!("{api}.")),
            ],
        );
    }

    #[test]
    fn plan_no_hit_is_single_text_segment() {
        // The common path: no lexicon term → one Text segment, so the caller's
        // behavior is identical to plain G2P.
        let lex = technical_lexicon();
        assert_eq!(
            plan_segments("just ordinary words here", &lex),
            vec![Segment::Text("just ordinary words here".into())],
        );
    }

    #[test]
    fn permagent_resolves_as_a_word_not_spelled_out() {
        // #516: "Permagent" was spoken letter-by-letter. It must now resolve to
        // a single phoneme override (a real word), case-insensitively.
        let lex = technical_lexicon();
        let expected = lex.get("permagent").expect("permagent must resolve");
        assert!(!expected.is_empty());
        assert_eq!(lex.get("Permagent"), Some(expected), "case-insensitive");
        assert_eq!(lex.get("PERMAGENT"), Some(expected), "case-insensitive");

        // In a sentence: the term becomes one Override segment, surrounded by
        // ordinary G2P Text.
        let segs = plan_segments("welcome to Permagent today", &lex);
        assert_eq!(
            segs,
            vec![
                Segment::Text("welcome to".into()),
                Segment::Override(expected.to_string()),
                Segment::Text("today".into()),
            ],
        );

        // The other coined names resolve too.
        for name in ["permagentd", "spectral", "kinrows"] {
            assert!(lex.get(name).is_some(), "'{name}' must resolve");
        }
    }

    #[test]
    fn lexicon_matches_whole_words_only_not_substrings() {
        // Word-boundary guarantee: a longer word that merely CONTAINS a lexicon
        // key as a substring must not be mangled — only whole tokens match.
        let lex = technical_lexicon();
        // "spectrally" contains "spectral"; "permagently" contains "permagent".
        let segs = plan_segments("spectrally permagently", &lex);
        assert_eq!(
            segs,
            vec![Segment::Text("spectrally permagently".into())],
            "substrings inside longer words must stay as plain G2P text"
        );
    }

    #[test]
    fn lexicon_phonemes_tokenize_against_vocab() {
        // Every override must be expressible in the Kokoro vocab, else the
        // phonemes are silently dropped at tokenization.
        let vocab = build_vocab();
        for (surface, ipa) in technical_lexicon().entries.iter() {
            let tokens = phonemes_to_tokens(ipa, &vocab);
            // start pad + at least one real phoneme + end pad
            assert!(
                tokens.len() > 2,
                "'{surface}' phonemes '{ipa}' produced no vocab tokens"
            );
        }
    }
}
