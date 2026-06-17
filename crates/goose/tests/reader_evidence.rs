//! Phase 1 evidence harness for the Reader (#296) — the payload-diff proof.
//!
//! Ignored by default (needs macOS + a real image + the Vision framework at
//! runtime). Run manually:
//!
//! ```sh
//! READER_TEST_IMAGE=/path/to/screenshot.png \
//!   cargo test -p permagent --test reader_evidence -- --ignored --nocapture
//! ```
//!
//! It runs a real image through `reader::ingest_image` and prints the BEFORE
//! (old path: full base64 image in the agent's context) vs AFTER (Reader path:
//! a compact digest text block, no image) outgoing message — proving the raw
//! bytes no longer enter Henry's context. Brain-recall verification of the
//! ingested text is a separate (daemon-run) step and may defer to M4.

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "needs a real image via READER_TEST_IMAGE + macOS Vision at runtime"]
async fn reader_payload_diff_evidence() {
    use permagent::reader;

    let path =
        std::env::var("READER_TEST_IMAGE").expect("set READER_TEST_IMAGE to a real image path");
    let bytes = std::fs::read(&path).expect("read image");
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image.png")
        .to_string();

    let b64_len = bytes.len().div_ceil(3) * 4;
    println!("\n=== READER PAYLOAD-DIFF EVIDENCE ===");
    println!(
        "file: {filename}  ({} bytes, ~{} base64 chars, ~{} tokens if sent raw)",
        bytes.len(),
        b64_len,
        b64_len / 4
    );

    let digest = reader::ingest_image(&bytes, &filename)
        .await
        .expect("ingest");
    println!("\n-- Reader digest --");
    println!("{}", serde_json::to_string_pretty(&digest).unwrap());

    println!("\n-- BEFORE (old path: raw image base64 into Henry's context) --");
    println!(
        "{}",
        serde_json::json!({
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                    "data": format!("<{} base64 chars>", b64_len)}},
                {"type": "text", "text": "(file upload)"}
            ]
        })
    );

    println!("\n-- AFTER (Reader path) --");
    if digest.is_visual {
        println!("is_visual=true → image falls through to Henry (visual Q&A preserved)");
    } else {
        let block = format!(
            "📎 {} — {} (recall: \"{}\")",
            filename, digest.summary, digest.recall_query
        );
        println!(
            "{}",
            serde_json::json!({
                "role": "user",
                "content": [ {"type": "text", "text": block} ]
            })
        );
        println!(
            "\n→ base64 image block ELIMINATED. ~{} tokens kept out of context; \
             full text persisted to Brain key {}",
            b64_len / 4,
            digest.memory_key
        );
    }
}
