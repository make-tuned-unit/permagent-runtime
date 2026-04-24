use std::net::TcpStream;
use tauri::{Listener, Manager};

mod oauth;

/// Check if permagentd is running on the given port
fn check_daemon(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

#[tauri::command]
fn daemon_status() -> bool {
    check_daemon(3001)
}

#[tauri::command]
async fn start_oauth(
    app: tauri::AppHandle,
    provider: String,
    client_id: String,
    client_secret: String,
    scopes: String,
) -> Result<String, String> {
    oauth::start_oauth_flow(app, provider, client_id, client_secret, scopes).await
}

#[tauri::command]
async fn get_integration_status(provider: String) -> Result<oauth::IntegrationStatus, String> {
    oauth::get_status(&provider)
}

#[tauri::command]
async fn disconnect_integration(
    app: tauri::AppHandle,
    provider: String,
) -> Result<(), String> {
    oauth::disconnect(&app, &provider)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            start_oauth,
            get_integration_status,
            disconnect_integration
        ])
        .setup(|app| {
            // Check if daemon is running on startup
            if !check_daemon(3001) {
                eprintln!("Warning: permagentd not detected on port 3001");
            }

            // Listen for deep link events (permagent:// protocol)
            let handle = app.handle().clone();
            app.listen("deep-link://new-url", move |event: tauri::Event| {
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
