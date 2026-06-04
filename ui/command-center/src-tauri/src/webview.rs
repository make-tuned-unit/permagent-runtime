use std::collections::HashMap;
use std::sync::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewBuilder, WebviewUrl};
use tauri::webview::NewWindowResponse;

#[derive(Serialize, Clone)]
pub struct BrowserNavigatedEvent {
    pub webview_id: String,
    pub url: String,
}

#[derive(Serialize, Clone)]
struct BrowserNewWindowRequestEvent {
    source_webview_id: String,
    url: String,
}

pub struct BrowserState {
    pub webviews: HashMap<String, String>, // webview_id -> label
}

pub type BrowserSessions = Mutex<BrowserState>;

pub fn new_sessions() -> BrowserSessions {
    Mutex::new(BrowserState {
        webviews: HashMap::new(),
    })
}

const START_PAGE: &str = "data:text/html,<html><body style='background:%230B1120;color:%23e2e8f0;font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0'><div style='text-align:center'><h2 style='color:%2300ffb4'>Permagent Browser</h2><p style='color:%2364748b'>Navigate to a URL using the address bar</p></div></body></html>";

/// Create a child webview embedded inside the main window at the given position/size.
#[tauri::command]
pub fn create_browser_webview(
    app: AppHandle,
    sessions: tauri::State<'_, BrowserSessions>,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    let id = format!("bw-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let parsed_url: url::Url = if url.is_empty() || url == "about:blank" {
        START_PAGE.parse().unwrap()
    } else {
        url.parse().map_err(|e: url::ParseError| format!("Invalid URL: {}", e))?
    };

    let app_clone = app.clone();
    let wv_id = id.clone();

    let app_new_window = app.clone();
    let wv_id_new_window = id.clone();

    let builder = WebviewBuilder::new(&id, WebviewUrl::External(parsed_url))
        .auto_resize()
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
        .on_new_window(move |url, _features| {
            let _ = app_new_window.emit(
                "browser_new_window_request",
                BrowserNewWindowRequestEvent {
                    source_webview_id: wv_id_new_window.clone(),
                    url: url.to_string(),
                },
            );
            NewWindowResponse::Deny
        });

    // Attach the child webview to the main application window.
    let main_window = app
        .get_window("main")
        .ok_or("Main window not found")?;

    main_window
        .add_child(
            builder,
            tauri::LogicalPosition::new(x, y),
            tauri::LogicalSize::new(width, height),
        )
        .map_err(|e| format!("Failed to create child webview: {}", e))?;

    sessions
        .lock()
        .map_err(|e| e.to_string())?
        .webviews
        .insert(id.clone(), id.clone());

    Ok(id)
}

/// Reposition / resize an existing child webview (called on container resize).
#[tauri::command]
pub fn update_browser_bounds(
    app: AppHandle,
    webview_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or("Webview not found")?;
    webview
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    webview
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn navigate_browser(
    app: AppHandle,
    webview_id: String,
    url: String,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or("Browser webview not found")?;
    let parsed: url::Url = url
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;
    webview.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_browser(
    app: AppHandle,
    sessions: tauri::State<'_, BrowserSessions>,
    webview_id: String,
) -> Result<(), String> {
    if let Some(webview) = app.get_webview(&webview_id) {
        webview.close().map_err(|e| e.to_string())?;
    }
    sessions
        .lock()
        .map_err(|e| e.to_string())?
        .webviews
        .remove(&webview_id);
    Ok(())
}

/// Show a child webview by positioning it back into view.
/// The React side follows up with `update_browser_bounds` to set exact coords.
#[tauri::command]
pub fn show_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    let _webview = app
        .get_webview(&webview_id)
        .ok_or("Webview not found")?;
    // Position will be set by the subsequent update_browser_bounds call.
    Ok(())
}

/// Hide a child webview by moving it offscreen.
#[tauri::command]
pub fn hide_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    if let Some(webview) = app.get_webview(&webview_id) {
        webview
            .set_position(tauri::LogicalPosition::new(-10000.0_f64, -10000.0_f64))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_browser_url(app: AppHandle, webview_id: String) -> Result<String, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or("Browser webview not found")?;
    Ok(webview.url().map_err(|e| e.to_string())?.to_string())
}
