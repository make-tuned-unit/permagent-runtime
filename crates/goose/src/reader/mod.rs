//! The Reader (#296) — local document/image ingestion worker.
//!
//! Third worker in the Librarian/Steward family. Its job: keep raw file bytes
//! out of the agent's (Henry's) expensive context. A dropped file is OCR'd /
//! extracted **locally**, the full text is written to the Brain (durable
//! side-channel), and only a compact [`Digest`] — a summary plus a semantic
//! `recall_query` — is returned to the caller. Henry communicates the digest
//! and answers follow-ups via Brain recall, never holding the raw document.
//!
//! Phase 1 covers images (screenshots, photos) via Apple Vision OCR. Documents
//! (PDF/DOCX) land in Phase 2.
//!
//! ## The `is_visual` decision (the make-or-break UX knob)
//!
//! OCR extracts *text*. A screenshot-of-text → digest (token saving). A photo
//! of a dog → OCR yields nothing useful, and intercepting it would break visual
//! Q&A. So [`ingest_image`] weighs OCR **volume AND confidence** (Vision returns
//! per-observation confidence): a photo with one stray high-confidence word
//! still correctly falls through (low volume), and a blurry low-confidence
//! capture falls through too. Both thresholds are **named, configurable** values
//! (`reader_ocr_min_chars`, `reader_ocr_min_confidence`) — tuning surface, not
//! hardcoded magic. When `is_visual` is true the Reader does NOT ingest; the
//! caller passes the image to the agent as before.

pub mod vision_ocr;

use crate::agents::platform_extensions::get_global_brain;
use crate::config::Config;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use spectral::{RememberOpts, Visibility};

/// `source` tag recorded on every memory the Reader writes.
pub const READER_SOURCE: &str = "permagent.reader";

// ── The is_visual thresholds (tuning surface — see module docs) ────────────
const MIN_CHARS_KEY: &str = "reader_ocr_min_chars";
const MIN_CONFIDENCE_KEY: &str = "reader_ocr_min_confidence";
/// Below this many non-whitespace OCR chars, an image is treated as visual.
const DEFAULT_MIN_CHARS: usize = 16;
/// Below this mean Vision confidence (0–1), an image is treated as visual.
const DEFAULT_MIN_CONFIDENCE: f64 = 0.45;

// ── Local-summary (Ollama) config — mirrors the Librarian's local LLM use ──
const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const SUMMARY_MODEL: &str = "qwen2.5:7b";
// The digest is what Henry SAYS to the user, so the summarizer must never
// state a specific it might get wrong. Exact facts (dates, amounts, IDs, names)
// live verbatim in the Brain and are RECALLED from there — the digest gives the
// gist only. "An Acme invoice due in July" (honest-vague) beats "due July 16"
// (confidently wrong). Extractive-not-paraphrastic; omit specifics you can't
// reproduce exactly.
const SUMMARY_SYSTEM: &str = "You write a brief, conservative gist of extracted document/screenshot text — one or two sentences on WHAT KIND of document it is and its overall purpose. Do NOT restate or paraphrase exact specifics (dates, amounts, numbers, IDs, proper names); those are preserved verbatim elsewhere and recalled from there, so paraphrasing them only risks confidently-wrong errors. Prefer honest-vague ('an invoice due in July') over precise-but-risky ('due July 16'); if you cannot describe the gist without a specific, omit the specific. Output only the gist, no preamble.";

/// Compact result of ingesting a file. This — not the raw bytes — is what the
/// agent receives. Full extracted text lives in the Brain under [`Digest::memory_key`].
#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    /// One/two-sentence local-model summary (empty when `is_visual`).
    pub summary: String,
    /// A distinctive semantic phrase Henry passes to `search_memory` to refetch
    /// the full text (recall is semantic — there is no recall-by-id).
    pub recall_query: String,
    /// Always `permagent.reader`.
    pub source: String,
    /// Approximate token count of the full extracted text (len/4).
    pub token_count: usize,
    /// Non-whitespace character count of the OCR/extracted text.
    pub char_count: usize,
    /// True when OCR found too little / too low-confidence text to be worth
    /// ingesting — the caller should pass the image to the agent to *see*.
    pub is_visual: bool,
    /// Stable content-hash key the full text is stored under (`reader:file:{sha256}`).
    pub memory_key: String,
    /// True when this exact file was already ingested (skipped OCR + write).
    pub already_ingested: bool,
}

fn min_chars() -> usize {
    Config::global()
        .get_param::<usize>(MIN_CHARS_KEY)
        .unwrap_or(DEFAULT_MIN_CHARS)
}

fn min_confidence() -> f64 {
    Config::global()
        .get_param::<f64>(MIN_CONFIDENCE_KEY)
        .unwrap_or(DEFAULT_MIN_CONFIDENCE)
}

/// Stable content-hash key for a file's bytes — the idempotency mechanism.
/// Re-dropping identical bytes yields the same key, so `remember_with` is a
/// no-op (`WriteOutcome::NoOp`) and the pre-check skips the work entirely.
pub fn content_key(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("reader:file:{:x}", hasher.finalize())
}

/// Weigh OCR volume AND confidence to classify an image as visual vs textual.
/// Returns `(is_visual, non_ws_char_count, mean_confidence)`.
fn decide_visual(lines: &[vision_ocr::OcrLine]) -> (bool, usize, f64) {
    let chars: usize = lines
        .iter()
        .flat_map(|l| l.text.chars())
        .filter(|c| !c.is_whitespace())
        .count();
    let mean_conf = if lines.is_empty() {
        0.0
    } else {
        lines.iter().map(|l| l.confidence as f64).sum::<f64>() / lines.len() as f64
    };
    let is_visual = chars < min_chars() || mean_conf < min_confidence();
    (is_visual, chars, mean_conf)
}

fn joined_text(lines: &[vision_ocr::OcrLine]) -> String {
    lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ingest a dropped image: OCR locally, decide visual-vs-textual, and for
/// textual images summarize + write the full text to the Brain. Returns the
/// compact [`Digest`]. Idempotent on identical bytes.
pub async fn ingest_image(image_bytes: &[u8], filename: &str) -> anyhow::Result<Digest> {
    let key = content_key(image_bytes);

    // Idempotency / skip-work pre-check: identical bytes already ingested?
    if let Some(brain) = get_global_brain() {
        if let Ok(Some(mem)) = brain.get_memory_by_key(&key).await {
            let char_count = mem.content.chars().filter(|c| !c.is_whitespace()).count();
            let summary = truncate(&mem.content, 240);
            return Ok(Digest {
                recall_query: recall_query_for(filename, &summary),
                summary,
                source: READER_SOURCE.to_string(),
                token_count: mem.content.len() / 4,
                char_count,
                is_visual: false,
                memory_key: key,
                already_ingested: true,
            });
        }
    }

    // OCR runs off the async executor (objc2/Vision is blocking native work).
    let bytes = image_bytes.to_vec();
    let lines = tokio::task::spawn_blocking(move || vision_ocr::recognize(&bytes))
        .await
        .map_err(|e| anyhow::anyhow!("vision task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("vision OCR failed: {e}"))?;

    let (is_visual, char_count, mean_conf) = decide_visual(&lines);
    tracing::info!(
        filename,
        char_count,
        mean_conf,
        is_visual,
        "reader: OCR complete"
    );

    if is_visual {
        // Too little / too low-confidence text → a visual image. Do not ingest;
        // the caller passes the bytes to the agent so visual Q&A still works.
        return Ok(Digest {
            summary: String::new(),
            recall_query: String::new(),
            source: READER_SOURCE.to_string(),
            token_count: 0,
            char_count,
            is_visual: true,
            memory_key: key,
            already_ingested: false,
        });
    }

    let full_text = joined_text(&lines);
    let summary = summarize(&full_text).await;

    if let Some(brain) = get_global_brain() {
        let opts = RememberOpts {
            source: Some(READER_SOURCE.to_string()),
            visibility: Visibility::Private,
            ..Default::default()
        };
        if let Err(e) = brain.remember_with(&key, &full_text, opts).await {
            tracing::warn!("reader: brain write failed for {key}: {e}");
        }
    } else {
        tracing::warn!("reader: Brain not ready; digest returned but full text not persisted");
    }

    Ok(Digest {
        recall_query: recall_query_for(filename, &summary),
        summary,
        source: READER_SOURCE.to_string(),
        token_count: full_text.len() / 4,
        char_count,
        is_visual: false,
        memory_key: key,
        already_ingested: false,
    })
}

/// Local-model summary via Ollama; degrades to a truncation if Ollama is down
/// so the feature still works (Henry just gets a cruder digest).
async fn summarize(text: &str) -> String {
    match ollama_summary(text).await {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        Ok(_) => truncate(text, 240),
        Err(e) => {
            tracing::debug!("reader: Ollama summary unavailable ({e}); using truncation");
            truncate(text, 240)
        }
    }
}

async fn ollama_summary(text: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let prompt = format!(
        "Write a brief, conservative gist (one or two sentences, NO exact dates/amounts/numbers/names — keep specifics vague or omit them) of this extracted text:\n\n{}",
        truncate(text, 6000)
    );
    let body = serde_json::json!({
        "model": SUMMARY_MODEL,
        "prompt": prompt,
        "system": SUMMARY_SYSTEM,
        "stream": false,
        "options": { "temperature": 0.2, "num_predict": 120 },
    });
    let resp = client
        .post(format!("{OLLAMA_BASE_URL}/api/generate"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ollama error: {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(v.get("response")
        .and_then(|r| r.as_str())
        .unwrap_or_default()
        .to_string())
}

/// A distinctive semantic phrase for `search_memory` — filename stem plus the
/// leading words of the summary, which the stored full text will match on.
fn recall_query_for(filename: &str, summary: &str) -> String {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let head = summary
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{stem} {head}").trim().to_string()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_chars {
        t.to_string()
    } else {
        let s: String = t.chars().take(max_chars).collect();
        format!("{s}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_key_is_stable_and_namespaced() {
        let a = content_key(b"hello world");
        let b = content_key(b"hello world");
        let c = content_key(b"different");
        assert_eq!(a, b, "identical bytes → identical key (idempotency)");
        assert_ne!(a, c, "different bytes → different key");
        assert!(a.starts_with("reader:file:"));
    }

    #[test]
    fn visual_when_text_is_sparse() {
        let lines = vec![vision_ocr::OcrLine {
            text: "dog".to_string(),
            confidence: 0.99,
        }];
        let (is_visual, chars, _conf) = decide_visual(&lines);
        assert!(is_visual, "a single short high-confidence word → visual");
        assert_eq!(chars, 3);
    }

    #[test]
    fn visual_when_confidence_is_low() {
        // Plenty of characters, but low confidence (incidental/garbled text).
        let lines = vec![vision_ocr::OcrLine {
            text: "qwertyuiop asdfghjkl zxcvbnm".to_string(),
            confidence: 0.10,
        }];
        let (is_visual, _chars, conf) = decide_visual(&lines);
        assert!(is_visual, "low mean confidence → visual");
        assert!(conf < DEFAULT_MIN_CONFIDENCE);
    }

    #[test]
    fn textual_when_dense_and_confident() {
        let lines = vec![
            vision_ocr::OcrLine {
                text: "Invoice #4823 — Acme Corp".to_string(),
                confidence: 0.97,
            },
            vision_ocr::OcrLine {
                text: "Total due: $1,240.00 by 2026-07-01".to_string(),
                confidence: 0.95,
            },
        ];
        let (is_visual, chars, conf) = decide_visual(&lines);
        assert!(!is_visual, "dense high-confidence text → ingest as digest");
        assert!(chars >= DEFAULT_MIN_CHARS);
        assert!(conf >= DEFAULT_MIN_CONFIDENCE);
    }

    #[test]
    fn recall_query_uses_filename_stem() {
        let q = recall_query_for(
            "/tmp/Quarterly_Report.png",
            "Q2 revenue rose 12% year over year",
        );
        assert!(q.contains("Quarterly_Report"));
        assert!(q.contains("Q2"));
    }
}
