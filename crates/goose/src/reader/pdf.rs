//! PDF text-layer extraction for the Reader (#296).
//!
//! Lifts the same lopdf content-stream walk used by the computercontroller
//! `pdf_tool` (Tj / TJ operators, negative-offset word spacing), but operates
//! on bytes (`Document::load_mem`) and returns the plain text so the Reader can
//! route it to the Brain instead of into the agent's context. PDFs with a text
//! layer — the overwhelming majority of real documents — extract cleanly here;
//! scanned / image-only PDFs yield little text (the caller detects that).

use lopdf::{content::Content as PdfContent, Document, Object};

/// Extract the text layer from a PDF's bytes. Returns an empty string for an
/// image-only / scanned PDF (no text layer) — the caller treats sparse output
/// as "needs OCR".
pub fn extract_pdf_text(bytes: &[u8]) -> String {
    let Ok(doc) = Document::load_mem(bytes) else {
        return String::new();
    };

    let mut text = String::new();
    for (_page_num, page_id) in doc.get_pages() {
        let Ok(page_obj) = doc.get_object(page_id) else {
            continue;
        };
        let Ok(page_dict) = page_obj.as_dict() else {
            continue;
        };
        let Ok(contents) = page_dict.get(b"Contents").and_then(|c| c.as_reference()) else {
            continue;
        };
        let Ok(content_obj) = doc.get_object(contents) else {
            continue;
        };
        let Ok(stream) = content_obj.as_stream() else {
            continue;
        };
        let Ok(content_data) = stream.get_plain_content() else {
            continue;
        };
        let Ok(content) = PdfContent::decode(&content_data) else {
            continue;
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
        text.push('\n');
    }

    text.trim().to_string()
}
