use std::net::TcpStream;
use tauri::Manager;

/// Check if permagentd is running on the given port
fn check_daemon(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

#[tauri::command]
fn daemon_status() -> bool {
    check_daemon(3001)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![daemon_status])
        .setup(|app| {
            // Check if daemon is running on startup
            if !check_daemon(3001) {
                eprintln!("Warning: permagentd not detected on port 3001");
            }

            // Listen for deep link events (permagent:// protocol)
            let handle = app.handle().clone();
            app.listen("deep-link://new-url", move |event| {
                let payload = event.payload();
                if payload.contains("oauth/callback") {
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.eval(&format!(
                            "window.__PERMAGENT_OAUTH_CALLBACK__ = {};",
                            serde_json::json!(payload)
                        ));
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
