//! Minimal sentencepiece reader + encoder for wake-word keywords.
//!
//! The KWS zipformer model ships a sentencepiece `bpe.model`, and sherpa-onnx's
//! keyword spotter (at our pinned 1.13.x) accepts only PRE-TOKENIZED keyword
//! lines — it does no text→token encoding of its own (`EncodeKeywords` looks
//! every whitespace-separated token up in tokens.txt and fails on anything
//! else). Open-vocabulary phrases therefore need us to run the sentencepiece
//! segmentation ourselves.
//!
//! Rather than pull in a sentencepiece binding for one 500-piece vocabulary,
//! this module hand-parses the model protobuf (three fields of one message)
//! and runs the exact segmentation algorithm the model calls for. The shipped
//! `bpe.model` is a UNIGRAM model (trainer_spec.model_type == 1) despite its
//! filename, so encoding is a Viterbi search maximizing summed piece
//! log-probabilities — not BPE merge-ranking, which segments "HELLO WORLD"
//! visibly differently (`▁W OR L D` vs the correct `▁WORLD`). Verified against
//! all reference pairs the model distributes (keywords_raw.txt →
//! keywords.txt): the output is byte-identical.

use std::collections::HashMap;
use std::path::Path;

/// Sentencepiece word-boundary marker (U+2581), prefixed to each word.
const WORD_BOUNDARY: char = '\u{2581}';

/// Piece vocabulary: piece string → log-probability score.
/// Only NORMAL pieces participate in segmentation (control tokens like
/// `<blk>`/`<unk>` are excluded).
pub struct BpeVocab {
    pieces: HashMap<String, f32>,
    /// Longest piece length in chars — bounds the Viterbi inner loop.
    max_piece_chars: usize,
}

impl BpeVocab {
    /// Parse a sentencepiece `.model` protobuf from raw bytes.
    ///
    /// Wire format walked here (all other fields skipped by wire type):
    /// `ModelProto { repeated SentencePiece pieces = 1; }`
    /// `SentencePiece { string piece = 1; float score = 2; Type type = 3; }`
    /// where `type` defaults to 1 (NORMAL).
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let mut pieces = HashMap::new();
        let mut i = 0usize;
        while i < data.len() {
            let (tag, next) = read_varint(data, i)?;
            i = next;
            let (field, wire) = (tag >> 3, tag & 7);
            match wire {
                2 => {
                    let (len, next) = read_varint(data, i)?;
                    i = next;
                    let end = i
                        .checked_add(len as usize)
                        .filter(|&e| e <= data.len())
                        .ok_or_else(|| anyhow::anyhow!("bpe.model: truncated field"))?;
                    if field == 1 {
                        if let Some((piece, score)) = parse_sentence_piece(&data[i..end])? {
                            pieces.insert(piece, score);
                        }
                    }
                    i = end;
                }
                0 => i = read_varint(data, i)?.1,
                5 => i += 4,
                1 => i += 8,
                w => anyhow::bail!("bpe.model: unsupported wire type {w}"),
            }
        }
        if pieces.is_empty() {
            anyhow::bail!("bpe.model contained no usable pieces");
        }
        let max_piece_chars = pieces.keys().map(|p| p.chars().count()).max().unwrap_or(1);
        Ok(Self {
            pieces,
            max_piece_chars,
        })
    }

    /// Parse a sentencepiece `.model` file from disk.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes =
            std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        Self::from_bytes(&bytes)
    }

    /// Encode a phrase into a space-separated token line for the keyword
    /// spotter, e.g. `"hey henry"` → `"▁HE Y ▁HE N RY"`.
    ///
    /// Input is uppercased and stripped to A–Z/'/space (the gigaspeech vocab is
    /// uppercase English). Returns `None` when any resulting word cannot be
    /// segmented from the vocabulary — the caller should treat that phrase as
    /// unusable rather than pass partial tokens to the spotter (an OOV token in
    /// a keywords file is a hard error in sherpa-onnx).
    pub fn encode_phrase(&self, phrase: &str) -> Option<String> {
        let cleaned: String = phrase
            .to_uppercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphabetic() || c == '\'' {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let mut out: Vec<String> = Vec::new();
        let mut any = false;
        for word in cleaned.split_whitespace() {
            any = true;
            out.extend(self.encode_word(word)?);
        }
        if !any {
            return None;
        }
        Some(out.join(" "))
    }

    /// Viterbi segmentation of one `▁`-prefixed word: the piece sequence with
    /// the maximum summed score, exactly sentencepiece's unigram encode.
    fn encode_word(&self, word: &str) -> Option<Vec<String>> {
        let chars: Vec<char> = std::iter::once(WORD_BOUNDARY).chain(word.chars()).collect();
        let n = chars.len();
        // best[e] = (score, start) of the best segmentation of chars[..e].
        let mut best: Vec<Option<(f32, usize)>> = vec![None; n + 1];
        best[0] = Some((0.0, 0));
        for e in 1..=n {
            let lo = e.saturating_sub(self.max_piece_chars);
            for b in lo..e {
                let Some((prefix_score, _)) = best[b] else {
                    continue;
                };
                let piece: String = chars[b..e].iter().collect();
                let Some(&score) = self.pieces.get(&piece) else {
                    continue;
                };
                let cand = prefix_score + score;
                if best[e].is_none_or(|(s, _)| cand > s) {
                    best[e] = Some((cand, b));
                }
            }
        }
        best[n]?;
        let mut rev = Vec::new();
        let mut e = n;
        while e > 0 {
            let (_, b) = best[e]?;
            rev.push(chars[b..e].iter().collect::<String>());
            e = b;
        }
        rev.reverse();
        Some(rev)
    }
}

/// One `SentencePiece` submessage → `(piece, score)`, or `None` for
/// non-NORMAL pieces (control/unknown/unused).
fn parse_sentence_piece(data: &[u8]) -> anyhow::Result<Option<(String, f32)>> {
    let mut piece: Option<String> = None;
    let mut score = 0.0f32;
    let mut ptype = 1u64; // proto3 default: NORMAL
    let mut i = 0usize;
    while i < data.len() {
        let (tag, next) = read_varint(data, i)?;
        i = next;
        let (field, wire) = (tag >> 3, tag & 7);
        match wire {
            2 => {
                let (len, next) = read_varint(data, i)?;
                i = next;
                let end = i
                    .checked_add(len as usize)
                    .filter(|&e| e <= data.len())
                    .ok_or_else(|| anyhow::anyhow!("bpe.model: truncated piece"))?;
                if field == 1 {
                    piece = Some(String::from_utf8_lossy(&data[i..end]).into_owned());
                }
                i = end;
            }
            5 => {
                let end = i + 4;
                if end > data.len() {
                    anyhow::bail!("bpe.model: truncated float");
                }
                if field == 2 {
                    score = f32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
                }
                i = end;
            }
            0 => {
                let (v, next) = read_varint(data, i)?;
                i = next;
                if field == 3 {
                    ptype = v;
                }
            }
            1 => i += 8,
            w => anyhow::bail!("bpe.model: unsupported wire type {w} in piece"),
        }
    }
    Ok(match (piece, ptype) {
        (Some(p), 1) => Some((p, score)),
        _ => None,
    })
}

fn read_varint(data: &[u8], mut i: usize) -> anyhow::Result<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let &byte = data
            .get(i)
            .ok_or_else(|| anyhow::anyhow!("bpe.model: truncated varint"))?;
        i += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((result, i));
        }
        shift += 7;
        if shift >= 64 {
            anyhow::bail!("bpe.model: varint overflow");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-build a sentencepiece model protobuf from (piece, score, type).
    fn build_model(pieces: &[(&str, f32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (piece, score, ptype) in pieces {
            let mut sub = Vec::new();
            // field 1 (piece), wire 2
            sub.push(0x0a);
            sub.push(piece.len() as u8);
            sub.extend_from_slice(piece.as_bytes());
            // field 2 (score), wire 5
            sub.push(0x15);
            sub.extend_from_slice(&score.to_le_bytes());
            // field 3 (type), wire 0
            sub.push(0x18);
            sub.push(*ptype);
            // ModelProto field 1, wire 2
            out.push(0x0a);
            out.push(sub.len() as u8);
            out.extend_from_slice(&sub);
        }
        out
    }

    #[test]
    fn parses_pieces_and_skips_control_tokens() {
        let model = build_model(&[
            ("<blk>", 0.0, 3), // CONTROL — excluded
            ("\u{2581}HI", -2.0, 1),
            ("A", -3.0, 1),
        ]);
        let vocab = BpeVocab::from_bytes(&model).unwrap();
        assert_eq!(vocab.pieces.len(), 2);
        assert!(vocab.pieces.contains_key("\u{2581}HI"));
        assert!(!vocab.pieces.contains_key("<blk>"));
    }

    /// Viterbi picks the globally best segmentation, not a greedy/merge one:
    /// with a whole-word piece scoring better than any split, the whole word
    /// wins even when no intermediate merge path to it exists.
    #[test]
    fn viterbi_prefers_best_total_score() {
        let model = build_model(&[
            ("\u{2581}WORLD", -5.0, 1),
            ("\u{2581}W", -2.0, 1),
            ("O", -1.0, 1),
            ("R", -1.0, 1),
            ("L", -1.0, 1),
            ("D", -1.0, 1),
        ]);
        let vocab = BpeVocab::from_bytes(&model).unwrap();
        // Split path: -2 + 4*(-1) = -6; whole piece: -5 → whole piece wins.
        assert_eq!(
            vocab.encode_phrase("world").as_deref(),
            Some("\u{2581}WORLD")
        );
    }

    #[test]
    fn lowercase_and_punctuation_are_normalized() {
        let model = build_model(&[
            ("\u{2581}HI", -1.0, 1),
            ("\u{2581}", -1.5, 1),
            ("A", -1.0, 1),
        ]);
        let vocab = BpeVocab::from_bytes(&model).unwrap();
        assert_eq!(
            vocab.encode_phrase("hi, a!").as_deref(),
            Some("\u{2581}HI \u{2581} A")
        );
    }

    #[test]
    fn unencodable_phrase_returns_none() {
        let model = build_model(&[("\u{2581}HI", -1.0, 1)]);
        let vocab = BpeVocab::from_bytes(&model).unwrap();
        assert_eq!(vocab.encode_phrase("zzz"), None);
        assert_eq!(vocab.encode_phrase("  ,, "), None, "no words at all");
    }

    #[test]
    fn malformed_model_is_an_error_not_a_panic() {
        assert!(BpeVocab::from_bytes(&[0x0a]).is_err()); // truncated
        assert!(BpeVocab::from_bytes(&[]).is_err()); // empty
    }

    /// Ground truth against the real gigaspeech model when it is installed:
    /// every (raw → tokenized) reference pair distributed with the model must
    /// reproduce byte-identically. Skips silently when the model isn't on disk
    /// (CI machines don't have voice assets).
    #[test]
    fn matches_sentencepiece_on_installed_model() {
        let dir = crate::voice::kws::WakeWordModelPaths::default_paths().model_dir;
        let model = dir.join("bpe.model");
        if !model.exists() {
            eprintln!("skipping: {} not installed", model.display());
            return;
        }
        let vocab = BpeVocab::load(&model).unwrap();
        let raw = std::fs::read_to_string(dir.join("keywords_raw.txt")).unwrap();
        let tokenized = std::fs::read_to_string(dir.join("keywords.txt")).unwrap();
        for (r, t) in raw.lines().zip(tokenized.lines()) {
            if r.trim().is_empty() {
                continue;
            }
            assert_eq!(
                vocab.encode_phrase(r.trim()).as_deref(),
                Some(t.trim()),
                "mismatch for {r:?}"
            );
        }
    }
}
