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

pub mod garble;
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
// Dispatched through `crate::mesh::pool::generate` (Workload::Batch) — the
// pool engine's scheduler + fallback ladder when PERMAGENT_MESH_ENGINE is on,
// plain `resolve_route(Batch)` (config::ollama_host() under the hood) when off.
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

        // #339: retire the prior version of this document, if any. The Reader
        // keys memories by content-hash (`reader:file:{sha256}`), so an UPDATED
        // file lands under a NEW key while the OLD memory persists — stale, yet
        // still recall-able, so the agent could surface a version the user
        // already replaced. We track the last content-key per document identity
        // (the dropped filename) and hard-delete the superseded memory via
        // `Brain::forget` on re-ingest.
        retire_prior_version(&brain, filename, &key).await;
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

/// Retire the previously-ingested version of a document on re-ingest (#339).
///
/// Records `doc_identity` (the dropped filename) → `new_key` in a
/// Permagent-owned index table and returns the prior content-key it replaced.
/// If a different prior key existed, its (now stale) memory is hard-deleted via
/// [`SafeBrain::forget`](crate::brain_handle::SafeBrain::forget). Best-effort:
/// any index/forget failure is logged, never surfaced to the caller — a stale
/// leftover is a quality issue, not an ingest failure.
///
/// Identity is the filename: re-dropping an updated file under the same name
/// retires the old text. (Two unrelated files sharing a name will also retire
/// the earlier one — an accepted limitation of filename-as-identity; the
/// content the agent already saw lives in the conversation, not this cache.)
async fn retire_prior_version(
    brain: &crate::brain_handle::SafeBrain,
    doc_identity: &str,
    new_key: &str,
) {
    let identity = doc_identity.to_string();
    let new_key_owned = new_key.to_string();

    let prior =
        tokio::task::spawn_blocking(move || doc_index::swap_content_key(&identity, &new_key_owned))
            .await;

    let prior = match prior {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            tracing::warn!("reader: doc-index update failed for {doc_identity}: {e}");
            return;
        }
        Err(e) => {
            tracing::warn!("reader: doc-index task panicked: {e}");
            return;
        }
    };

    if let Some(old_key) = prior {
        if old_key != new_key {
            match brain.forget(&old_key).await {
                Ok(report) => tracing::info!(
                    doc = %doc_identity,
                    old_key = %old_key,
                    new_key = %new_key,
                    memory_rows = report.store.memory_rows,
                    fingerprints = report.store.fingerprints,
                    "reader: retired stale prior document version"
                ),
                Err(e) => tracing::warn!(
                    old_key = %old_key,
                    "reader: forget of stale prior version failed: {e}"
                ),
            }
        }
    }
}

/// Permagent-owned index mapping a document identity (filename) to the
/// content-hash key its latest ingest was stored under. Lives in the Brain DB
/// as a `_pm_`-prefixed table (same convention as cleanup's migration ledger);
/// it never participates in recall.
mod doc_index {
    use anyhow::Result;

    /// How long an index connection waits for SQLite's write lock before
    /// giving up — mirrors the cleanup path's contention handling.
    const BUSY_TIMEOUT_MS: u64 = 30_000;

    fn open() -> Result<rusqlite::Connection> {
        let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))?;
        Ok(conn)
    }

    fn ensure_table(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _pm_reader_doc_index ( \
                 doc_identity TEXT PRIMARY KEY, \
                 content_key  TEXT NOT NULL, \
                 updated_at   TEXT NOT NULL DEFAULT (datetime('now')) \
             );",
        )?;
        Ok(())
    }

    /// Record `doc_identity → new_key`, returning the content-key it replaced
    /// (if the document had been ingested before). Production entrypoint.
    pub(super) fn swap_content_key(doc_identity: &str, new_key: &str) -> Result<Option<String>> {
        let conn = open()?;
        swap_content_key_on_conn(&conn, doc_identity, new_key)
    }

    /// Core of [`swap_content_key`] against an arbitrary connection. Exposed for
    /// testing without touching `Paths::brain_dir()`.
    pub(super) fn swap_content_key_on_conn(
        conn: &rusqlite::Connection,
        doc_identity: &str,
        new_key: &str,
    ) -> Result<Option<String>> {
        ensure_table(conn)?;

        let prior: Option<String> = conn
            .query_row(
                "SELECT content_key FROM _pm_reader_doc_index WHERE doc_identity = ?1",
                [doc_identity],
                |r| r.get(0),
            )
            .ok();

        conn.execute(
            "INSERT INTO _pm_reader_doc_index (doc_identity, content_key, updated_at) \
             VALUES (?1, ?2, datetime('now')) \
             ON CONFLICT(doc_identity) DO UPDATE SET \
                 content_key = excluded.content_key, \
                 updated_at  = excluded.updated_at",
            rusqlite::params![doc_identity, new_key],
        )?;

        Ok(prior)
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
        } else if let garble::TextQuality::Garbled { reason } = garble::assess(&text) {
            // #468 safety gate: extraction produced char-shifted / mojibake
            // junk (typically a subsetted font with no usable ToUnicode CMap).
            // Fail loud and ingest NOTHING — a garbled "success" would be
            // summarized by the local model and confidently relayed by the
            // agent as if it were the document's real content.
            tracing::warn!(
                filename,
                reason = %reason,
                "reader: PDF extraction returned garbled text; refusing to ingest"
            );
            anyhow::bail!(
                "couldn't read this PDF cleanly — text extraction returned unreadable text \
                 (likely a font-encoding issue); refusing to ingest garbled content"
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
    let prompt = format!(
        "Write a brief, conservative gist (one or two sentences, NO exact dates/amounts/numbers/names — keep specifics vague or omit them) of this extracted text:\n\n{}",
        truncate(text, 6000)
    );
    // Dispatch through the mesh pool engine: with `PERMAGENT_MESH_ENGINE` on,
    // this rides the full ladder (trusted healthy peer → local → escalation
    // hint); with it off, a single attempt against `resolve_route(Batch)`
    // with the same 60s budget this function always used. The wire body is
    // built by the engine's inference-only choke-point.
    crate::mesh::pool::generate(crate::mesh::pool::GenerateRequest {
        model: SUMMARY_MODEL.to_string(),
        prompt,
        system: Some(SUMMARY_SYSTEM.to_string()),
        options: Some(serde_json::json!({ "temperature": 0.2, "num_predict": 120 })),
        keep_alive: None,
        timeout: None,
        workload: crate::mesh::Workload::Batch,
    })
    .await
    .map(|resp| resp.text)
    .map_err(|e| e.message)
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

    /// #339: first ingest of a document records its content-key and has no
    /// prior version to retire; re-ingesting the SAME bytes (same key) reports
    /// the prior key equal to the new one, so no forget fires.
    #[test]
    fn doc_index_first_ingest_and_identical_reingest() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let k1 = content_key(b"v1 contents");

        let prior = doc_index::swap_content_key_on_conn(&conn, "report.pdf", &k1).unwrap();
        assert_eq!(prior, None, "first ingest has no prior version");

        let prior2 = doc_index::swap_content_key_on_conn(&conn, "report.pdf", &k1).unwrap();
        assert_eq!(
            prior2,
            Some(k1.clone()),
            "identical re-drop returns the same key (== new_key ⇒ caller skips forget)"
        );
    }

    /// #339: re-ingesting an UPDATED document (new bytes ⇒ new key) returns the
    /// superseded key so the caller can `forget` it, and the index now points
    /// at the new key.
    #[test]
    fn doc_index_updated_reingest_returns_stale_key() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let old = content_key(b"v1 contents");
        let new = content_key(b"v2 contents — edited");
        assert_ne!(old, new);

        doc_index::swap_content_key_on_conn(&conn, "report.pdf", &old).unwrap();
        let prior = doc_index::swap_content_key_on_conn(&conn, "report.pdf", &new).unwrap();

        assert_eq!(
            prior,
            Some(old),
            "updated re-ingest surfaces the stale prior key to be forgotten"
        );

        let current: String = conn
            .query_row(
                "SELECT content_key FROM _pm_reader_doc_index WHERE doc_identity = 'report.pdf'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(current, new, "index advances to the new content key");
    }

    /// Distinct document identities are tracked independently.
    #[test]
    fn doc_index_keys_are_per_identity() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let a = content_key(b"a");
        let b = content_key(b"b");

        assert_eq!(
            doc_index::swap_content_key_on_conn(&conn, "a.txt", &a).unwrap(),
            None
        );
        assert_eq!(
            doc_index::swap_content_key_on_conn(&conn, "b.txt", &b).unwrap(),
            None,
            "a second document does not see the first's key"
        );
    }

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

    // ── #468 fixtures: in-memory PDFs built with lopdf ─────────────────────

    /// A paragraph long enough to clear the sparse-text check and give the
    /// garble detector real signal.
    const PDF_PARAGRAPH: &str = "Wealthie Family Office overview. All rights \
        reserved. Our platform integrates brokerage services with education \
        savings plans, offering families three revenue streams and a \
        partnership model that reaches schools across the province. The \
        integration is expected to launch in September and will provide \
        access to registered accounts for every student in the program.";

    /// Build a one-page PDF showing `text` via a Tj operator. With
    /// `with_font`, the page declares a standard Type1 font (Helvetica, no
    /// /Encoding → StandardEncoding), so lopdf's encoding-aware extraction
    /// decodes it. Without it, the encoding-aware path finds no font and the
    /// legacy raw walk reads the bytes as-is — the #468 failure shape.
    fn build_test_pdf(text: &str, with_font: bool) -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let mut operations = vec![Operation::new("BT", vec![])];
        if with_font {
            operations.push(Operation::new("Tf", vec!["F1".into(), 12.into()]));
        }
        operations.push(Operation::new("Td", vec![50.into(), 700.into()]));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(text.as_bytes().to_vec())],
        ));
        operations.push(Operation::new("ET", vec![]));
        let content = Content { operations };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode content"),
        ));

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });

        let mut pages_dict = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        if with_font {
            let font_id = doc.add_object(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            });
            pages_dict.set(
                "Resources",
                dictionary! { "Font" => dictionary! { "F1" => font_id } },
            );
        }
        doc.objects
            .insert(pages_id, lopdf::Object::Dictionary(pages_dict));

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save pdf");
        bytes
    }

    #[test]
    fn pdf_with_standard_font_extracts_readable_text() {
        // Encoding-aware path: font present → decoded through its encoding.
        let bytes = build_test_pdf(PDF_PARAGRAPH, true);
        let text = pdf::extract_pdf_text(&bytes);
        assert!(
            text.contains("Wealthie") && text.contains("All rights"),
            "expected decoded text, got: {text:.80}"
        );
        assert_eq!(garble::assess(&text), garble::TextQuality::Readable);
    }

    #[test]
    fn fontless_pdf_falls_back_to_raw_walk() {
        // No font resources → encoding-aware path yields nothing → the legacy
        // raw walk still recovers whatever bytes the stream shows.
        let bytes = build_test_pdf(PDF_PARAGRAPH, false);
        let text = pdf::extract_pdf_text(&bytes);
        assert!(
            text.contains("Wealthie"),
            "raw fallback should surface the stream bytes, got: {text:.80}"
        );
    }

    #[tokio::test]
    async fn garbled_pdf_is_refused_by_document_pipeline() {
        // The #468 shape end-to-end: a PDF whose shown bytes are char-shifted
        // (glyph codes without a ToUnicode map). Extraction "succeeds" but the
        // garble gate must refuse to ingest — never summarize noise.
        let shifted = garble::caesar_shift(&PDF_PARAGRAPH.to_uppercase(), 3);
        let bytes = build_test_pdf(&shifted, false);

        // Sanity: extraction really does return the shifted junk…
        let raw = pdf::extract_pdf_text(&bytes);
        assert!(raw.contains("DOO ULJKWV"), "fixture sanity: {raw:.80}");

        // …and the pipeline turns that into an explicit failure.
        let err = extract_document_text(&bytes, "family_office.pdf", "application/pdf")
            .await
            .expect_err("garbled extraction must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("couldn't read this PDF cleanly"),
            "error must be the honest extraction-failed state, got: {msg}"
        );
        assert!(
            msg.contains("font-encoding"),
            "error should name the likely cause, got: {msg}"
        );
    }

    #[tokio::test]
    async fn readable_pdf_passes_document_pipeline() {
        let bytes = build_test_pdf(PDF_PARAGRAPH, true);
        let text = extract_document_text(&bytes, "family_office.pdf", "application/pdf")
            .await
            .expect("readable PDF must extract");
        assert!(text.contains("Wealthie"));
    }
}
