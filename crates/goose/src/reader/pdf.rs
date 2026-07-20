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

use lopdf::{content::Content as PdfContent, Document, Object, ObjectId};

/// Extract the text layer from a PDF's bytes. Returns an empty string for an
/// image-only / scanned PDF (no text layer) — the caller treats sparse output
/// as "needs OCR". The caller MUST also gate the result through
/// [`super::garble::assess`] before treating it as the document's content.
pub fn extract_pdf_text(bytes: &[u8]) -> String {
    let Ok(doc) = Document::load_mem(bytes) else {
        return String::new();
    };

    let mut text = String::new();
    for (page_num, page_id) in doc.get_pages() {
        let page_text = extract_page_text(&doc, page_num, page_id);
        if !page_text.trim().is_empty() {
            text.push_str(page_text.trim_end());
            text.push('\n');
        }
    }

    text.trim().to_string()
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
                            Object::Real(offset) => {
                                if *offset < -100.0 {
                                    text.push(' ');
                                    last_was_text = false;
                                }
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
