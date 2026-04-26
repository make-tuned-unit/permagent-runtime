use std::collections::HashMap;
use std::sync::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Serialize, Clone)]
pub struct BrowserNavigatedEvent {
    pub webview_id: String,
    pub url: String,
}

pub struct BrowserState {
    pub windows: HashMap<String, String>, // webview_id -> label
}

pub type BrowserSessions = Mutex<BrowserState>;

pub fn new_sessions() -> BrowserSessions {
    Mutex::new(BrowserState {
        windows: HashMap::new(),
    })
}

const START_PAGE: &str = "data:text/html,<html><body style='background:%230B1120;color:%23e2e8f0;font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0'><div style='text-align:center'><h2 style='color:%2300ffb4'>Permagent Browser</h2><p style='color:%2364748b'>Navigate to a URL using the address bar</p></div></body></html>";

#[tauri::command]
pub fn create_browser_webview(
    app: AppHandle,
    sessions: tauri::State<'_, BrowserSessions>,
    url: String,
) -> Result<String, String> {
    let id = format!("bw-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let parsed_url: url::Url = if url.is_empty() || url == "about:blank" {
        START_PAGE.parse().unwrap()
    } else {
        url.parse().map_err(|e: url::ParseError| format!("Invalid URL: {}", e))?
    };

    let app_clone = app.clone();
    let wv_id = id.clone();

    WebviewWindowBuilder::new(&app, &id, WebviewUrl::External(parsed_url))
        .title("Permagent Browser")
        .inner_size(900.0, 700.0)
        .resizable(true)
        .on_navigation(move |nav_url| {
            let _ = app_clone.emit(
                "browser_navigated",
                BrowserNavigatedEvent {
                    webview_id: wv_id.clone(),
                    url: nav_url.to_string(),
                },
            );
            let scheme = nav_url.scheme();
            matches!(scheme, "https" | "http" | "about" | "data" | "blob")
        })
        .build()
        .map_err(|e| format!("Failed to create browser window: {}", e))?;

    sessions
        .lock()
        .map_err(|e| e.to_string())?
        .windows
        .insert(id.clone(), id.clone());

    Ok(id)
}

#[tauri::command]
pub fn navigate_browser(
    app: AppHandle,
    webview_id: String,
    url: String,
) -> Result<(), String> {
    let win = app
        .get_webview_window(&webview_id)
        .ok_or("Browser window not found")?;
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    win.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_browser(
    app: AppHandle,
    sessions: tauri::State<'_, BrowserSessions>,
    webview_id: String,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&webview_id) {
        win.close().map_err(|e| e.to_string())?;
    }
    sessions
        .lock()
        .map_err(|e| e.to_string())?
        .windows
        .remove(&webview_id);
    Ok(())
}

#[tauri::command]
pub fn show_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    let win = app
        .get_webview_window(&webview_id)
        .ok_or("Browser window not found")?;
    win.show().map_err(|e| e.to_string())?;
    win.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hide_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(&webview_id) {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_browser_url(app: AppHandle, webview_id: String) -> Result<String, String> {
    let win = app
        .get_webview_window(&webview_id)
        .ok_or("Browser window not found")?;
    Ok(win.url().map_err(|e| e.to_string())?.to_string())
}
