use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewBuilder, WebviewUrl};

struct BrowserWebview {
    _label: String,
}

pub struct BrowserSessions(Mutex<HashMap<String, BrowserWebview>>);

impl BrowserSessions {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

static WEBVIEW_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[derive(Clone, Serialize)]
struct BrowserNavigatedPayload {
    webview_id: String,
    url: String,
}

#[derive(Clone, Serialize)]
struct BrowserTitleChangedPayload {
    webview_id: String,
    title: String,
}

#[tauri::command]
pub async fn create_browser_webview(
    app: AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    let id = WEBVIEW_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let label = format!("browser-{id}");
    let parsed_url: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview_url = WebviewUrl::External(parsed_url);

    let window = app
        .get_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;

    let nav_id = label.clone();
    let nav_app = app.clone();
    let title_id = label.clone();
    let title_app = app.clone();
    let builder = WebviewBuilder::new(&label, webview_url)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15")
        .on_navigation(move |nav_url: &url::Url| {
            let _ = nav_app.emit(
                "browser_navigated",
                BrowserNavigatedPayload {
                    webview_id: nav_id.clone(),
                    url: nav_url.to_string(),
                },
            );
            true
        })
        .on_page_title_changed(move |title| {
            let _ = title_app.emit(
                "browser_title_changed",
                BrowserTitleChangedPayload {
                    webview_id: title_id.clone(),
                    title,
                },
            );
        });

    let position = tauri::Position::Logical(tauri::LogicalPosition::new(x, y));
    let size = tauri::Size::Logical(tauri::LogicalSize::new(width, height));

    window
        .add_child(builder, position, size)
        .map_err(|e| format!("Failed to create webview: {e}"))?;

    app.state::<BrowserSessions>()
        .0
        .lock()
        .unwrap()
        .insert(
            label.clone(),
            BrowserWebview {
                _label: label.clone(),
            },
        );

    Ok(label)
}

#[tauri::command]
pub async fn navigate_browser(
    app: AppHandle,
    webview_id: String,
    url: String,
) -> Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .navigate(parsed)
        .map_err(|e| format!("Navigation failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn update_browser_bounds(
    app: AppHandle,
    webview_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)))
        .map_err(|e| format!("Set position failed: {e}"))?;
    webview
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
        .map_err(|e| format!("Set size failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn hide_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    if let Some(webview) = app.get_webview(&webview_id) {
        webview
            .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                -10000.0, -10000.0,
            )))
            .map_err(|e| format!("Hide failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn close_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    app.state::<BrowserSessions>()
        .0
        .lock()
        .unwrap()
        .remove(&webview_id);
    if let Some(webview) = app.get_webview(&webview_id) {
        webview.close().map_err(|e| format!("Close failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn zoom_browser(
    app: AppHandle,
    webview_id: String,
    zoom_level: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let js = format!("document.body.style.zoom = '{:.0}%'", zoom_level * 100.0);
    webview
        .eval(&js)
        .map_err(|e| format!("Zoom failed: {e}"))?;
    Ok(())
}
