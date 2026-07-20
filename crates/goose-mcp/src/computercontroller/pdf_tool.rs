use super::text_quality;
use lopdf::{content::Content as PdfContent, Document, Object, ObjectId};
use rmcp::model::{Content, ErrorCode, ErrorData};
use std::{fs, path::Path};

/// Encoding-aware extraction for one page (#468): lopdf's `extract_text_chunks`
/// tracks the current font (`Tf`) and decodes shown strings through that
/// font's encoding — ToUnicode CMap, `/Differences`, or base encoding. Falls
/// back to the legacy raw content-stream walk only when the decoded path
/// yields nothing; the raw walk's output is garble-gated by the caller.
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

/// Legacy raw walk (pre-#468): reads Tj/TJ string operands without applying
/// font encodings, so custom-encoded fonts come out char-shifted. Last resort
/// only — the caller runs [`text_quality::assess`] on the final text.
fn extract_page_text_raw(doc: &Document, page_id: ObjectId) -> String {
    let mut text = String::new();

    // Try to get text from page contents
    if let Ok(page_obj) = doc.get_object(page_id) {
        if let Ok(page_dict) = page_obj.as_dict() {
            // Try to get text from Contents stream
            if let Ok(contents) = page_dict.get(b"Contents").and_then(|c| c.as_reference()) {
                if let Ok(content_obj) = doc.get_object(contents) {
                    if let Ok(stream) = content_obj.as_stream() {
                        if let Ok(content_data) = stream.get_plain_content() {
                            if let Ok(content) = PdfContent::decode(&content_data) {
                                // Process each operation in the content stream
                                for operation in content.operations {
                                    match operation.operator.as_ref() {
                                        // "Tj" operator: show text
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
                                        // "TJ" operator: show text with positioning
                                        "TJ" => {
                                            if let Some(Object::Array(ref arr)) =
                                                operation.operands.first()
                                            {
                                                let mut last_was_text = false;
                                                for element in arr {
                                                    match element {
                                                        Object::String(ref bytes, _) => {
                                                            if let Ok(s) =
                                                                std::str::from_utf8(bytes)
                                                            {
                                                                if last_was_text {
                                                                    text.push(' ');
                                                                }
                                                                text.push_str(s);
                                                                last_was_text = true;
                                                            }
                                                        }
                                                        Object::Integer(offset) => {
                                                            // Large negative offsets often indicate word spacing
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
                                        _ => (), // Ignore other operators
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    text
}

pub async fn pdf_tool(
    path: &str,
    operation: &str,
    cache_dir: &Path,
) -> Result<Vec<Content>, ErrorData> {
    // Open and parse the PDF file
    let doc = Document::load(path).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to open PDF file: {}", e),
            None,
        )
    })?;

    let result = match operation {
        "extract_text" => {
            let mut text = String::new();

            // Iterate over each page in the document
            for (page_num, page_id) in doc.get_pages() {
                text.push_str(&format!("Page {}:\n", page_num));
                text.push_str(&extract_page_text(&doc, page_num, page_id));
                text.push('\n');
            }

            // #468 safety gate: a subsetted font without a usable ToUnicode
            // CMap yields char-shifted junk ("All Rights Reserved" →
            // "$OO5LJKWV5HVHUYHG"). Surfacing that as a successful extraction
            // invites the agent to confidently "summarize" noise — fail loud
            // instead, as an explicit tool error.
            if let text_quality::TextQuality::Garbled { reason } = text_quality::assess(&text) {
                return Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!(
                        "PDF text extraction FAILED for {path}: the output is unreadable ({reason}). \
                         This is typically a font-encoding problem (the PDF's embedded font has no \
                         usable ToUnicode map), so the extracted bytes are NOT the document's real \
                         text. Do not summarize, analyze, or answer questions from any partial \
                         output — tell the user the PDF could not be read cleanly."
                    ),
                    None,
                ));
            }

            if text.trim().is_empty() {
                "No text found in PDF".to_string()
            } else {
                format!("Extracted text from PDF:\n\n{}", text)
            }
        }

        "extract_images" => {
            let cache_dir = cache_dir.join("pdf_images");
            fs::create_dir_all(&cache_dir).map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to create image cache directory: {}", e),
                    None,
                )
            })?;

            let mut images = Vec::new();
            let mut image_count = 0;

            // Helper function to determine file extension based on stream dict
            fn get_image_extension(dict: &lopdf::Dictionary) -> &'static str {
                if let Ok(filter) = dict.get(b"Filter") {
                    match filter {
                        Object::Name(name) => {
                            match name.as_slice() {
                                b"DCTDecode" => ".jpg",
                                b"JBIG2Decode" => ".jbig2",
                                b"JPXDecode" => ".jp2",
                                b"CCITTFaxDecode" => ".tiff",
                                b"FlateDecode" => {
                                    // PNG-like images often use FlateDecode
                                    // Check color space to confirm
                                    if let Ok(cs) = dict.get(b"ColorSpace") {
                                        if let Ok(name) = cs.as_name() {
                                            if name == b"DeviceRGB" || name == b"DeviceGray" {
                                                return ".png";
                                            }
                                        }
                                    }
                                    ".raw"
                                }
                                _ => ".raw",
                            }
                        }
                        Object::Array(filters) => {
                            // If multiple filters, check the last one
                            if let Some(Object::Name(name)) = filters.last() {
                                match name.as_slice() {
                                    b"DCTDecode" => return ".jpg",
                                    b"JPXDecode" => return ".jp2",
                                    _ => {}
                                }
                            }
                            ".raw"
                        }
                        _ => ".raw",
                    }
                } else {
                    ".raw"
                }
            }

            // Process each page
            for (page_num, page_id) in doc.get_pages() {
                let page = doc.get_object(page_id).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get page {}: {}", page_num, e),
                        None,
                    )
                })?;

                let page_dict = page.as_dict().map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get page dict {}: {}", page_num, e),
                        None,
                    )
                })?;

                // Get page resources - handle both direct dict and reference
                let resources = match page_dict.get(b"Resources") {
                    Ok(res) => match res {
                        Object::Dictionary(dict) => Ok(dict),
                        Object::Reference(id) => doc
                            .get_object(*id)
                            .map_err(|e| {
                                ErrorData::new(
                                    ErrorCode::RESOURCE_NOT_FOUND,
                                    format!("Failed to get resource reference: {}", e),
                                    None,
                                )
                            })
                            .and_then(|obj| {
                                obj.as_dict().map_err(|e| {
                                    ErrorData::new(
                                        ErrorCode::RESOURCE_NOT_FOUND,
                                        format!("Resource reference is not a dictionary: {}", e),
                                        None,
                                    )
                                })
                            }),
                        _ => Err(ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            "Resources is neither dictionary nor reference".to_string(),
                            None,
                        )),
                    },
                    Err(e) => Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get Resources: {}", e),
                        None,
                    )),
                }?;

                // Look for XObject dictionary - handle both direct dict and reference
                let xobjects = match resources.get(b"XObject") {
                    Ok(xobj) => match xobj {
                        Object::Dictionary(dict) => Ok(dict),
                        Object::Reference(id) => doc
                            .get_object(*id)
                            .map_err(|e| {
                                ErrorData::new(
                                    ErrorCode::INTERNAL_ERROR,
                                    format!("Failed to get XObject reference: {}", e),
                                    None,
                                )
                            })
                            .and_then(|obj| {
                                obj.as_dict().map_err(|e| {
                                    ErrorData::new(
                                        ErrorCode::INTERNAL_ERROR,
                                        format!("XObject reference is not a dictionary: {}", e),
                                        None,
                                    )
                                })
                            }),
                        _ => Err(ErrorData::new(
                            ErrorCode::INTERNAL_ERROR,
                            "XObject is neither dictionary nor reference".to_string(),
                            None,
                        )),
                    },
                    Err(e) => Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to get XObject: {}", e),
                        None,
                    )),
                };

                if let Ok(xobjects) = xobjects {
                    for (name, xobject) in xobjects.iter() {
                        let xobject_id = xobject.as_reference().map_err(|_| {
                            ErrorData::new(
                                ErrorCode::INTERNAL_ERROR,
                                "Failed to get XObject reference".to_string(),
                                None,
                            )
                        })?;

                        let xobject = doc.get_object(xobject_id).map_err(|e| {
                            ErrorData::new(
                                ErrorCode::INTERNAL_ERROR,
                                format!("Failed to get XObject: {}", e),
                                None,
                            )
                        })?;

                        if let Ok(stream) = xobject.as_stream() {
                            // Check if it's an image
                            if let Ok(subtype) =
                                stream.dict.get(b"Subtype").and_then(|s| s.as_name())
                            {
                                if subtype == b"Image" {
                                    let extension = get_image_extension(&stream.dict);

                                    // Get image metadata
                                    let width = stream
                                        .dict
                                        .get(b"Width")
                                        .and_then(|w| w.as_i64())
                                        .unwrap_or(0);
                                    let height = stream
                                        .dict
                                        .get(b"Height")
                                        .and_then(|h| h.as_i64())
                                        .unwrap_or(0);
                                    let bpc = stream
                                        .dict
                                        .get(b"BitsPerComponent")
                                        .and_then(|b| b.as_i64())
                                        .unwrap_or(0);

                                    // Get the image data
                                    if let Ok(data) = stream.get_plain_content() {
                                        let image_path = cache_dir.join(format!(
                                            "page{}_obj{}_{}{}",
                                            page_num,
                                            xobject_id.0,
                                            String::from_utf8_lossy(name),
                                            extension
                                        ));

                                        fs::write(&image_path, &data).map_err(|e| {
                                            ErrorData::new(
                                                ErrorCode::INTERNAL_ERROR,
                                                format!("Failed to write image: {}", e),
                                                None,
                                            )
                                        })?;

                                        images.push(format!(
                                            "Saved image to: {} ({}x{}, {} bits per component)",
                                            image_path.display(),
                                            width,
                                            height,
                                            bpc
                                        ));
                                        image_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if images.is_empty() {
                "No images found in PDF".to_string()
            } else {
                format!("Found {} images:\n{}", image_count, images.join("\n"))
            }
        }

        _ => {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Invalid operation: {}. Valid operations are: 'extract_text', 'extract_images'",
                    operation
                ),
                None,
            ))
        }
    };

    Ok(vec![Content::text(result)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_pdf_text_extraction() {
        let test_pdf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/computercontroller/tests/data/test.pdf");
        let cache_dir = tempfile::tempdir().unwrap().keep();

        println!("Testing text extraction from: {}", test_pdf_path.display());

        let result = pdf_tool(test_pdf_path.to_str().unwrap(), "extract_text", &cache_dir).await;

        assert!(result.is_ok(), "PDF text extraction should succeed");
        let content = result.unwrap();
        assert!(!content.is_empty(), "Extracted text should not be empty");
        let text = content[0].as_text().unwrap();
        println!("Extracted text:\n{}", text.text);
        assert!(text.text.contains("Page 1"), "Should contain page marker");
        assert!(
            text.text.contains("This is a test PDF"),
            "Should contain expected test content"
        );
    }

    #[tokio::test]
    async fn test_pdf_image_extraction() {
        let test_pdf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/computercontroller/tests/data/test_image.pdf");
        let cache_dir = tempfile::tempdir().unwrap().keep();

        println!("Testing image extraction from: {}", test_pdf_path.display());

        // Now try image extraction
        let result = pdf_tool(
            test_pdf_path.to_str().unwrap(),
            "extract_images",
            &cache_dir,
        )
        .await;

        println!("Image extraction result: {:?}", result);
        assert!(result.is_ok(), "PDF image extraction should succeed");
        let content = result.unwrap();
        assert!(
            !content.is_empty(),
            "Image extraction result should not be empty"
        );
        let text = content[0].as_text().unwrap();
        println!("Extracted content: {}", text.text);

        // Should either find images or explicitly state none were found
        assert!(
            text.text.contains("Saved image to:") || text.text.contains("No images found"),
            "Should either save images or report none found"
        );

        // If we found images, verify they exist
        if text.text.contains("Saved image to:") {
            // Extract the file path from the output
            let file_path = text
                .text
                .lines()
                .find(|line| line.contains("Saved image to:"))
                .and_then(|line| line.split(": ").nth(1))
                .and_then(|path| path.split(" (").next())
                .expect("Should have a valid file path");

            println!("Verifying image file exists: {}", file_path);
            assert!(PathBuf::from(file_path).exists(), "Image file should exist");
        }
    }

    /// Build a one-page PDF whose shown bytes are char-shifted junk (the #468
    /// failure shape: glyph codes with no usable ToUnicode map). No font
    /// resources are declared, so the encoding-aware path yields nothing and
    /// the raw fallback surfaces the shifted bytes — which the garble gate
    /// must refuse.
    fn build_garbled_pdf() -> Vec<u8> {
        use lopdf::content::{Content as LoContent, Operation};
        use lopdf::{dictionary, Stream};

        let shifted = super::super::text_quality::caesar_shift(
            &"Wealthie Family Office overview. All rights reserved. Our platform \
              integrates brokerage services with education savings plans, offering \
              families three revenue streams and a partnership model that reaches \
              schools across the province and provides access to registered \
              accounts for every student in the program."
                .to_uppercase(),
            3,
        );

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = LoContent {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Td", vec![50.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(shifted.into_bytes())]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode content"),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save pdf");
        bytes
    }

    #[tokio::test]
    async fn test_garbled_pdf_extraction_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let pdf_path = dir.path().join("garbled.pdf");
        fs::write(&pdf_path, build_garbled_pdf()).unwrap();
        let cache_dir = dir.path().join("cache");

        let result = pdf_tool(pdf_path.to_str().unwrap(), "extract_text", &cache_dir).await;

        let err = result.expect_err("garbled extraction must be an explicit tool error");
        let msg = err.message.to_string();
        assert!(
            msg.contains("PDF text extraction FAILED"),
            "error must state extraction failed, got: {msg}"
        );
        assert!(
            msg.contains("Do not summarize"),
            "error must forbid summarizing partial output, got: {msg}"
        );
        // The garbled bytes themselves must NOT ride along in the error.
        assert!(
            !msg.contains("DOO ULJKWV"),
            "garbled content must not leak into the error surface: {msg}"
        );
    }

    #[tokio::test]
    async fn test_pdf_invalid_path() {
        let cache_dir = tempfile::tempdir().unwrap().keep();
        let result = pdf_tool("nonexistent.pdf", "extract_text", &cache_dir).await;

        assert!(result.is_err(), "Should fail with invalid path");
    }

    #[tokio::test]
    async fn test_pdf_invalid_operation() {
        let test_pdf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/computercontroller/tests/data/test.pdf");
        let cache_dir = tempfile::tempdir().unwrap().keep();

        let result = pdf_tool(
            test_pdf_path.to_str().unwrap(),
            "invalid_operation",
            &cache_dir,
        )
        .await;

        assert!(result.is_err(), "Should fail with invalid operation");
    }
}
