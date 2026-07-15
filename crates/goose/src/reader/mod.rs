//! The Reader (#296) — local document/image ingestion worker.
//!
//! Third worker in the Librarian/Steward family. Its job: keep raw file bytes
//! out of the agent's (Henry's) expensive context. A dropped file is OCR'd /
//! extracted **locally**, the full text is written to the Brain (durable
//! side-channel), and only a compact [`Digest`] — a summary plus a semantic
//! `recall_query` — is returned to the caller. Henry communicates the digest
//! and answers follow-ups via Brain recall, never holding the raw document.
//!
//! Phase 1 covers images (screenshots, photos) via Apple Vision OCR. Phase 2
//! adds documents: PDF text-layer extraction (lopdf) and plain-text/code
//! passthrough. Scanned (image-only) PDFs and docx are follow-ups.
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

pub mod pdf;
pub mod vision_ocr;

use crate::agents::platform_extensions::get_global_brain;
use crate::config::Config;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use spectral::{RememberOpts, Visibility};

/// `source` tag recorded on every memory the Reader writes.
pub const READER_SOURCE: &str = "permagent.reader";

/// Self-knowledge descriptor for the Reader surface. Co-located here; aggregated
/// by `crate::agents::self_knowledge`. Static — the ingest route is always-on
/// (registered unconditionally, no enable flag), so it renders editorially with
/// no live status claim.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "reader",
        display_name: "Reader",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "Intercepts dropped files and ingests them locally — OCR for images, text extraction for PDFs and documents",
        why_it_matters:
            "You can discuss a file the user dropped without them pasting its contents, and ingestion costs no model tokens",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        // Reader is Static (no live brief state), so its lesson confirms BY PROXY
        // via MemoryRecallable — the Reader writes ingested text to the Brain.
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Drop a file in",
                body: "Ask them to drag a screenshot, PDF, or document straight onto the chat — no upload dialog, no copy-paste, no file paths.",
                open_surface: None,
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Show the magic",
                body: "When they drop it, the Reader OCRs or extracts the text locally and stores it in your Brain — you receive only a compact digest, so even a huge file costs you almost no context. Tell them what you can now see in it.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::MemoryRecallable(
                    "a distinctive phrase from the file they just dropped",
                )),
            },
        ],
    };

// ── The is_visual thresholds (tuning surface — see module docs) ────────────
const MIN_CHARS_KEY: &str = "reader_ocr_min_chars";
const MIN_CONFIDENCE_KEY: &str = "reader_ocr_min_confidence";
/// Below this many non-whitespace OCR chars, an image is treated as visual.
const DEFAULT_MIN_CHARS: usize = 16;
/// Below this mean Vision confidence (0–1), an image is treated as visual.
const DEFAULT_MIN_CONFIDENCE: f64 = 0.45;

// ── Local-summary (Ollama) config — mirrors the Librarian's local LLM use ──
// Endpoint resolved from `crate::config::ollama_host()` (batch-tier configurable).
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
    format!("reader:file:{}", hex::encode(hasher.finalize()))
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

/// Idempotency / skip-work pre-check: if these exact bytes were already
/// ingested, rebuild the digest from the stored memory (no OCR/extract, no
/// re-summarize). Shared by the image and document paths.
async fn already_ingested_digest(key: &str, filename: &str) -> Option<Digest> {
    let brain = get_global_brain()?;
    let mem = brain.get_memory_by_key(key).await.ok()??;
    let char_count = mem.content.chars().filter(|c| !c.is_whitespace()).count();
    let summary = truncate(&mem.content, 240);
    Some(Digest {
        recall_query: recall_query_for(filename, &summary),
        summary,
        source: READER_SOURCE.to_string(),
        token_count: mem.content.len() / 4,
        char_count,
        is_visual: false,
        memory_key: key.to_string(),
        already_ingested: true,
    })
}

/// Summarize extracted text, write the full text to the Brain under `key`, and
/// build the compact digest. Shared tail of the image and document paths.
async fn finalize_text_ingest(key: String, full_text: String, filename: &str) -> Digest {
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

    let char_count = full_text.chars().filter(|c| !c.is_whitespace()).count();
    Digest {
        recall_query: recall_query_for(filename, &summary),
        summary,
        source: READER_SOURCE.to_string(),
        token_count: full_text.len() / 4,
        char_count,
        is_visual: false,
        memory_key: key,
        already_ingested: false,
    }
}

/// Ingest a dropped image: OCR locally, decide visual-vs-textual, and for
/// textual images summarize + write the full text to the Brain. Returns the
/// compact [`Digest`]. Idempotent on identical bytes.
pub async fn ingest_image(image_bytes: &[u8], filename: &str) -> anyhow::Result<Digest> {
    let key = content_key(image_bytes);

    if let Some(d) = already_ingested_digest(&key, filename).await {
        return Ok(d);
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

    Ok(finalize_text_ingest(key, joined_text(&lines), filename).await)
}

/// Ingest a dropped document (PDF / text / code / markdown / …): extract text
/// locally, summarize, and write the full text to the Brain. Returns the
/// compact [`Digest`] — never the raw bytes. Idempotent on identical bytes.
///
/// Documents are never "visual" — there is nothing for the agent to *see*, only
/// text to read — so `is_visual` is always false. Phase 2 handles text-bearing
/// PDFs and plain-text/code; scanned (image-only) PDFs and docx are follow-ups.
pub async fn ingest_document(bytes: &[u8], filename: &str, mime: &str) -> anyhow::Result<Digest> {
    let key = content_key(bytes);

    if let Some(d) = already_ingested_digest(&key, filename).await {
        return Ok(d);
    }

    let full_text = extract_document_text(bytes, filename, mime).await?;
    if full_text.trim().is_empty() {
        anyhow::bail!("no extractable text in {filename} (mime={mime})");
    }

    Ok(finalize_text_ingest(key, full_text, filename).await)
}

/// Extract plain text from a document's bytes by type.
async fn extract_document_text(bytes: &[u8], filename: &str, mime: &str) -> anyhow::Result<String> {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if mime == "application/pdf" || ext == "pdf" {
        let owned = bytes.to_vec();
        let text = tokio::task::spawn_blocking(move || pdf::extract_pdf_text(&owned))
            .await
            .map_err(|e| anyhow::anyhow!("pdf task panicked: {e}"))?;
        if text.chars().filter(|c| !c.is_whitespace()).count() < min_chars() {
            // Image-only / scanned PDF: no text layer. Per-page Vision OCR is a
            // Phase 2.1 follow-up; for now we surface what (little) we found.
            tracing::warn!(
                filename,
                "reader: PDF has little/no text layer (likely scanned); per-page OCR is a follow-up"
            );
        }
        Ok(text)
    } else if is_texty(mime, &ext) {
        Ok(String::from_utf8_lossy(bytes).to_string())
    } else {
        anyhow::bail!("unsupported document type: mime={mime} ext={ext}")
    }
}

/// Whether a non-PDF document is plain-text-like (safe to read as UTF-8).
fn is_texty(mime: &str, ext: &str) -> bool {
    if mime.starts_with("text/") {
        return true;
    }
    matches!(
        mime,
        "application/json"
            | "application/xml"
            | "application/x-yaml"
            | "application/yaml"
            | "application/toml"
            | "application/javascript"
            | "application/x-sh"
    ) || matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "csv"
            | "tsv"
            | "log"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "ini"
            | "cfg"
            | "conf"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "css"
            | "scss"
    )
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
        .post(format!("{}/api/generate", crate::config::ollama_host()))
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

    #[test]
    fn is_texty_covers_text_code_and_data() {
        assert!(is_texty("text/plain", "txt"));
        assert!(is_texty("text/markdown", "md"));
        assert!(is_texty("application/json", "json"));
        // Recognized by extension even with a generic/empty mime.
        assert!(is_texty("application/octet-stream", "rs"));
        assert!(is_texty("", "py"));
        // Not text.
        assert!(!is_texty("application/pdf", "pdf"));
        assert!(!is_texty("image/png", "png"));
        assert!(!is_texty("application/octet-stream", "bin"));
    }

    #[tokio::test]
    async fn document_text_passthrough_reads_utf8() {
        let text = extract_document_text(b"hello reader\nsecond line", "notes.txt", "text/plain")
            .await
            .expect("text passthrough");
        assert_eq!(text, "hello reader\nsecond line");
    }

    #[tokio::test]
    async fn document_unsupported_type_errors() {
        let err = extract_document_text(
            &[0xFF, 0xD8, 0xFF],
            "mystery.bin",
            "application/octet-stream",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("unsupported document type"));
    }

    #[test]
    fn pdf_extract_on_garbage_is_empty_not_panic() {
        assert_eq!(pdf::extract_pdf_text(b"not a pdf at all"), "");
        assert_eq!(pdf::extract_pdf_text(&[]), "");
    }
}
