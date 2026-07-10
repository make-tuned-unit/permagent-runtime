use base64::Engine;
use std::path::Path;

fn mime_from_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "md" => "text/markdown",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/x-toml",
        "rs" => "text/x-rust",
        "ts" | "tsx" => "text/typescript",
        "js" | "jsx" => "text/javascript",
        "py" => "text/x-python",
        _ => "application/octet-stream",
    }
}

#[tauri::command]
pub async fn read_dropped_file(path: String) -> Result<(String, String, String), String> {
    let p = Path::new(&path);

    let filename = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mime_type = mime_from_ext(&ext).to_string();

    let data = std::fs::read(&path).map_err(|e| format!("Failed to read file {}: {}", path, e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);

    Ok((filename, mime_type, b64))
}
