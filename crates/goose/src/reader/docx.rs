//! Minimal DOCX text extraction for the Reader (#296 follow-up).
//!
//! A `.docx` is a zip archive; the document body lives in `word/document.xml`
//! as OOXML WordprocessingML. This does not attempt full OOXML fidelity —
//! tables, headers/footers, embedded objects, styling are all ignored — it
//! walks `word/document.xml` and pulls the literal run text (`<w:t>`),
//! turning each closed `<w:p>` into a paragraph break and each `<w:br>`/
//! `<w:tab>` into a line break / tab within one. That is enough to make a
//! dropped `.docx` recallable text, which is the whole point — see
//! [`super::extract_document_text`].

use std::io::{Cursor, Read};

/// Extract plain text from a `.docx` file's raw bytes.
pub fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("not a valid .docx (zip open failed): {e}"))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| format!("not a valid .docx (missing word/document.xml): {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("could not read word/document.xml: {e}"))?;
    Ok(xml_runs_to_text(&xml))
}

/// Walk WordprocessingML and pull run text, honoring paragraph/line breaks.
/// Deliberately minimal — see the module doc.
// string_slice: every byte index below is either `i` (only ever advanced to
// a byte matching ASCII `<`, which cannot be a UTF-8 continuation byte) or a
// `find()` result on `&xml[i..]` (a byte offset that is always a char
// boundary), so every slice here is char-boundary safe by construction.
#[allow(clippy::string_slice)]
fn xml_runs_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut in_run_text = false;
    let bytes = xml.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let Some(rel_end) = xml[i..].find('>') else {
                break;
            };
            let tag = &xml[i + 1..i + rel_end];
            let closing = tag.starts_with('/');
            let name = tag
                .trim_start_matches('/')
                .trim_end_matches('/')
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("");
            match name {
                "w:t" => {
                    // Both `<w:t>` and a bodyless `<w:t/>` land here; the
                    // latter simply never contributes text before its (absent)
                    // closing tag, since `in_run_text` goes false again below
                    // on the very next `</w:t>` match — but a truly
                    // self-closing run has none, so guard on `closing` only:
                    // opening sets true, closing sets false.
                    in_run_text = !closing;
                }
                "w:p" if closing => out.push('\n'),
                "w:br" | "w:cr" => out.push('\n'),
                "w:tab" => out.push('\t'),
                _ => {}
            }
            i += rel_end + 1;
        } else if in_run_text {
            let Some(rel_next) = xml[i..].find('<') else {
                out.push_str(&unescape_xml(&xml[i..]));
                break;
            };
            out.push_str(&unescape_xml(&xml[i..i + rel_next]));
            i += rel_next;
        } else {
            let Some(rel_next) = xml[i..].find('<') else {
                break;
            };
            i += rel_next;
        }
    }
    out
}

/// The five predefined XML entities. Good enough for run text — docx never
/// needs the full numeric-reference table for the plain words a paragraph
/// carries.
fn unescape_xml(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal in-memory `.docx` (just `word/document.xml` — the only
    /// part our extractor reads) with the given paragraphs.
    fn build_test_docx(paragraphs: &[&str]) -> Vec<u8> {
        let body: String = paragraphs
            .iter()
            .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
            .collect();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>{body}</w:body>
</w:document>"#
        );

        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extracts_a_known_paragraph() {
        let bytes = build_test_docx(&["The quarterly report ships Friday."]);
        let text = extract_docx_text(&bytes).unwrap();
        assert!(
            text.contains("The quarterly report ships Friday."),
            "got: {text:?}"
        );
    }

    #[test]
    // string_slice: both indices are `find()` results, always char-boundary safe.
    #[allow(clippy::string_slice)]
    fn preserves_paragraph_breaks() {
        let bytes = build_test_docx(&["First paragraph.", "Second paragraph."]);
        let text = extract_docx_text(&bytes).unwrap();
        let first_at = text.find("First paragraph.").expect("first paragraph");
        let second_at = text.find("Second paragraph.").expect("second paragraph");
        assert!(first_at < second_at);
        assert!(
            text[first_at..second_at].contains('\n'),
            "paragraphs must be separated by a break, got: {text:?}"
        );
    }

    #[test]
    fn line_break_inside_a_run_becomes_a_newline() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body><w:p><w:r><w:t>line one</w:t><w:br/><w:t>line two</w:t></w:r></w:p></w:body>
</w:document>"#;
        let text = xml_runs_to_text(xml);
        assert_eq!(text.trim(), "line one\nline two");
    }

    #[test]
    fn decodes_xml_entities_in_run_text() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body><w:p><w:r><w:t>Tom &amp; Jerry &lt;3</w:t></w:r></w:p></w:body>
</w:document>"#;
        let text = xml_runs_to_text(xml);
        assert_eq!(text.trim(), "Tom & Jerry <3");
    }

    #[test]
    fn not_a_zip_is_a_readable_error() {
        let err = extract_docx_text(b"not a zip file at all").unwrap_err();
        assert!(err.contains("not a valid .docx"), "got: {err}");
    }
}
