//! PDF text-layer extraction for the Reader (#296, #468).
//!
//! Primary path: lopdf's **encoding-aware** extraction (`extract_text_chunks`),
//! which tracks the current font (`Tf`) and decodes each shown string through
//! that font's encoding — ToUnicode CMap, `/Differences`, or base encoding —
//! exactly what the PDF spec requires. This fixes #468: subsetted /
//! custom-encoded fonts render fine but their raw char codes are shifted junk
//! ("All Rights Reserved" → "$OO5LJKWV5HVHUYHG"); decoding through the CMap
//! recovers the real text. It also handles multi-stream `Contents` arrays,
//! which the old walk silently skipped.
//!
//! Fallback path (per page, only when the encoding-aware path yields nothing):
//! the legacy raw content-stream walk (Tj / TJ operators, negative-offset word
//! spacing) lifted from the computercontroller `pdf_tool`. Its output is NOT
//! trusted — the caller runs [`super::garble::assess`] on the final text and
//! refuses garbled results rather than feeding them to the agent.
//!
//! Scanned / image-only PDFs yield little text; the caller detects that.

use std::time::{Duration, Instant};

use lopdf::{content::Content as PdfContent, Document, Object, ObjectId};

/// Maximum number of PDF pages whose text layer will be inspected.
pub(super) const MAX_PDF_PAGES: usize = 2_000;
/// Maximum UTF-8 size of text returned from a PDF.
pub(super) const MAX_PDF_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// PDFs with more indirect objects than this are rejected before page extraction.
pub(super) const MAX_PDF_OBJECTS: usize = 100_000;
/// Wall-clock budget checked between page extractions.
pub(super) const PDF_EXTRACTION_DEADLINE: Duration = Duration::from_secs(10);

const TRUNCATION_MARKER: &str =
    "\n[PDF extraction truncated: document too large/complex to fully extract]";

#[derive(Clone, Copy)]
struct ExtractionLimits {
    max_pages: usize,
    max_text_bytes: usize,
    max_objects: usize,
    deadline: Duration,
}

const DEFAULT_LIMITS: ExtractionLimits = ExtractionLimits {
    max_pages: MAX_PDF_PAGES,
    max_text_bytes: MAX_PDF_TEXT_BYTES,
    max_objects: MAX_PDF_OBJECTS,
    deadline: PDF_EXTRACTION_DEADLINE,
};

/// Extract the text layer from a PDF's bytes. Returns an empty string for an
/// image-only / scanned PDF (no text layer) — the caller treats sparse output
/// as "needs OCR". The caller MUST also gate the result through
/// [`super::garble::assess`] before treating it as the document's content.
pub fn extract_pdf_text(bytes: &[u8]) -> String {
    extract_pdf_text_with_limits(bytes, DEFAULT_LIMITS)
}

fn extract_pdf_text_with_limits(bytes: &[u8], limits: ExtractionLimits) -> String {
    let started = Instant::now();
    let Ok(doc) = Document::load_mem(bytes) else {
        return String::new();
    };

    if doc.objects.len() > limits.max_objects || started.elapsed() >= limits.deadline {
        return truncation_marker(limits.max_text_bytes);
    }

    let mut text = String::new();
    let pages = doc.get_pages();
    let page_limit_hit = pages.len() > limits.max_pages;
    let mut truncated = false;

    for (page_num, page_id) in pages.into_iter().take(limits.max_pages) {
        if started.elapsed() >= limits.deadline {
            truncated = true;
            break;
        }

        let page_text = extract_page_text(&doc, page_num, page_id);
        if !page_text.trim().is_empty() {
            let separator_bytes = usize::from(!text.is_empty());
            if text.len() + separator_bytes + page_text.trim_end().len() > limits.max_text_bytes {
                if separator_bytes == 1 {
                    push_bounded(&mut text, "\n", limits.max_text_bytes);
                }
                push_bounded(&mut text, page_text.trim_end(), limits.max_text_bytes);
                truncated = true;
                break;
            }
            if separator_bytes == 1 {
                text.push('\n');
            }
            text.push_str(page_text.trim_end());
        }

        if started.elapsed() >= limits.deadline {
            truncated = true;
            break;
        }
    }

    truncated |= page_limit_hit;
    if truncated {
        append_truncation_marker(&mut text, limits.max_text_bytes);
    }
    text.trim().to_string()
}

fn push_bounded(output: &mut String, value: &str, max_bytes: usize) {
    let remaining = max_bytes.saturating_sub(output.len());
    let end = char_boundary_at_or_before(value, remaining.min(value.len()));
    // `end` is a validated char boundary, so `.get(..end)` always yields Some;
    // use it (not raw slicing) to satisfy clippy::string_slice and stay panic-free.
    if let Some(slice) = value.get(..end) {
        output.push_str(slice);
    }
}

fn char_boundary_at_or_before(value: &str, mut index: usize) -> usize {
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn append_truncation_marker(output: &mut String, max_bytes: usize) {
    if max_bytes == 0 {
        output.clear();
        return;
    }

    let marker = TRUNCATION_MARKER.trim_start();
    if marker.len() >= max_bytes {
        output.clear();
        push_bounded(output, marker, max_bytes);
        return;
    }

    let separator = usize::from(!output.is_empty());
    output.truncate(char_boundary_at_or_before(
        output,
        max_bytes.saturating_sub(marker.len() + separator),
    ));
    if separator == 1 {
        output.push('\n');
    }
    output.push_str(marker);
}

fn truncation_marker(max_bytes: usize) -> String {
    let mut text = String::new();
    append_truncation_marker(&mut text, max_bytes);
    text
}

#[cfg(test)]
pub(super) fn extract_pdf_text_for_test(
    bytes: &[u8],
    max_pages: usize,
    max_text_bytes: usize,
    deadline: Duration,
) -> String {
    extract_pdf_text_with_limits(
        bytes,
        ExtractionLimits {
            max_pages,
            max_text_bytes,
            max_objects: MAX_PDF_OBJECTS,
            deadline,
        },
    )
}

/// Encoding-aware extraction for one page, falling back to the raw walk only
/// when the decoded path produces nothing (e.g. a malformed font dictionary
/// lopdf refuses to decode at all).
fn extract_page_text(doc: &Document, page_num: u32, page_id: ObjectId) -> String {
    let decoded: String = doc
        .extract_text_chunks(&[page_num])
        .into_iter()
        .filter_map(|chunk| chunk.ok())
        .collect();
    if !decoded.trim().is_empty() {
        return decoded;
    }
    extract_page_text_raw(doc, page_id)
}

/// Legacy raw content-stream walk (pre-#468 behavior): reads string operands
/// of Tj/TJ without applying font encodings. Kept only as a last resort for
/// PDFs whose fonts lopdf cannot decode — its output can be char-shifted junk,
/// which is why the caller garble-gates the final text.
fn extract_page_text_raw(doc: &Document, page_id: ObjectId) -> String {
    let mut text = String::new();

    let Ok(page_obj) = doc.get_object(page_id) else {
        return text;
    };
    let Ok(page_dict) = page_obj.as_dict() else {
        return text;
    };
    let Ok(contents) = page_dict.get(b"Contents").and_then(|c| c.as_reference()) else {
        return text;
    };
    let Ok(content_obj) = doc.get_object(contents) else {
        return text;
    };
    let Ok(stream) = content_obj.as_stream() else {
        return text;
    };
    let Ok(content_data) = stream.get_plain_content() else {
        return text;
    };
    let Ok(content) = PdfContent::decode(&content_data) else {
        return text;
    };

    for operation in content.operations {
        match operation.operator.as_ref() {
            // Show text.
            "Tj" => {
                for operand in operation.operands {
                    if let Object::String(ref bytes, _) = operand {
                        if let Ok(s) = std::str::from_utf8(bytes) {
                            text.push_str(s);
                        }
                    }
                }
                text.push(' ');
            }
            // Show text with positioning; large negative offsets ≈ spaces.
            "TJ" => {
                if let Some(Object::Array(ref arr)) = operation.operands.first() {
                    let mut last_was_text = false;
                    for element in arr {
                        match element {
                            Object::String(ref bytes, _) => {
                                if let Ok(s) = std::str::from_utf8(bytes) {
                                    if last_was_text {
                                        text.push(' ');
                                    }
                                    text.push_str(s);
                                    last_was_text = true;
                                }
                            }
                            Object::Integer(offset) => {
                                if *offset < -100 {
                                    text.push(' ');
                                    last_was_text = false;
                                }
                            }
                            Object::Real(offset) if *offset < -100.0 => {
                                text.push(' ');
                                last_was_text = false;
                            }
                            _ => {}
                        }
                    }
                    text.push(' ');
                }
            }
            _ => {}
        }
    }

    text
}
