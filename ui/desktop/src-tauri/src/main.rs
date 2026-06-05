#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

/// Enable media capture (microphone getUserMedia) on a Tauri webview window.
/// WKWebView does not expose navigator.mediaDevices by default; we need to set
/// the private `_mediaCaptureEnabled` preference via the ObjC runtime.
#[cfg(target_os = "macos")]
fn enable_media_capture(window: &tauri::WebviewWindow) {
    // Use the with_webview API to access the underlying WKWebView directly.
    let _ = window.with_webview(|webview| {
        #[cfg(target_os = "macos")]
        unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            use objc2_foundation::NSString;

            let wk_webview: *mut AnyObject = webview.inner() as *mut _ as *mut AnyObject;
            if wk_webview.is_null() { return; }

            let config: *mut AnyObject = msg_send![wk_webview, configuration];
            if config.is_null() { return; }

            let prefs: *mut AnyObject = msg_send![config, preferences];
            if prefs.is_null() { return; }

            // setValue:forKey: with NSNumber(YES) for "_mediaCaptureEnabled"
            let yes: *mut AnyObject = msg_send![
                objc2::class!(NSNumber), numberWithBool: true
            ];
            let key = NSString::from_str("_mediaCaptureEnabled");
            let _: () = msg_send![prefs, setValue: yes forKey: &*key];
        }
    });
}


mod activity;
mod browser;
mod daemon;
mod files;
mod menu;
mod terminal;

/// Tauri command to enable media capture on the calling webview window.
/// Called from JS after dynamically creating windows (e.g. the chat window).
#[tauri::command]
fn enable_media_capture_cmd(window: tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    enable_media_capture(&window);
    let _ = window; // suppress unused on non-macOS
}

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(terminal::PtySessions::new())
        .manage(browser::BrowserSessions::new())
        .invoke_handler(tauri::generate_handler![
            terminal::spawn_pty_session,
            terminal::write_to_pty,
            terminal::resize_pty,
            terminal::kill_pty,
            browser::create_browser_webview,
            browser::navigate_browser,
            browser::update_browser_bounds,
            browser::hide_browser,
            browser::close_browser,
            browser::zoom_browser,
            browser::get_page_content,
            files::read_dropped_file,
            daemon::get_daemon_token,
            activity::emit_activity,
            enable_media_capture_cmd,
        ])
        .setup(|app| {
            daemon::start_daemon(app.handle())?;

            // Enable microphone capture (getUserMedia) in all webviews.
            // WKWebView requires mediaCaptureEnabled = true on its preferences
            // for navigator.mediaDevices to be available.
            #[cfg(target_os = "macos")]
            for (_label, window) in app.webview_windows() {
                enable_media_capture(&window);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Only stop daemon when the main window closes, not the chat window
            if let tauri::WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    let handle = window.app_handle().clone();
                    daemon::stop_daemon(&handle);
                }
            }
        });

    let builder = menu::attach_menu(builder);

    builder
        .run(tauri::generate_context!())
        .expect("error while running Permagent");
}
