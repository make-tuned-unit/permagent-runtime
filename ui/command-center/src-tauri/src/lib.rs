mod oauth;
mod pty;
mod webview;

use base64::Engine;

fn check_daemon(port: u16) -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

#[tauri::command]
fn daemon_status() -> bool {
    check_daemon(3001)
}

/// Read a file from disk and return its contents as base64 with MIME type.
/// Used by the native drag-drop bridge to convert file paths into File-like data.
#[tauri::command]
fn read_dropped_file(path: String) -> Result<(String, String, String), String> {
    eprintln!("[read_dropped_file] path={}", path);
    let file_path = std::path::Path::new(&path);
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string();

    let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        _ => "application/octet-stream",
    }
    .to_string();

    let bytes = std::fs::read(&path).map_err(|e| {
        eprintln!("[read_dropped_file] FAILED path={} err={}", path, e);
        format!("Failed to read file: {}", e)
    })?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    eprintln!("[read_dropped_file] success: file={} mime={} bytes={} b64_len={}", filename, mime_type, bytes.len(), b64.len());
    Ok((filename, mime_type, b64))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(pty::new_sessions())
        .manage(webview::new_sessions())
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            read_dropped_file,
            pty::spawn_pty_session,
            pty::write_to_pty,
            pty::resize_pty,
            pty::kill_pty,
            webview::create_browser_webview,
            webview::update_browser_bounds,
            webview::navigate_browser,
            webview::close_browser,
            webview::show_browser,
            webview::hide_browser,
            webview::get_browser_url,
            oauth::get_integration_status,
            oauth::disconnect_integration,
            oauth::start_oauth,
            oauth::start_oauth_in_browser,
        ])
        .setup(|_app| {
            if !check_daemon(3001) {
                eprintln!("Warning: permagentd not detected on port 3001");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
