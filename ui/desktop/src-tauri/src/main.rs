#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

mod activity;
mod browser;
mod daemon;
mod files;
mod menu;
mod terminal;

/// Enable media capture (microphone getUserMedia) on a Tauri webview window.
/// WKWebView does not expose navigator.mediaDevices by default; we set the
/// private `_mediaCaptureEnabled` preference via the ObjC runtime.
///
/// Uses objc2::exception::catch to handle ObjC NSExceptions (not just Rust
/// panics). A failure degrades gracefully — mic capture unavailable, app runs.
#[cfg(target_os = "macos")]
fn enable_media_capture(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| {
        // Wrap in both ObjC exception catch AND Rust catch_unwind.
        // objc2::exception::catch handles NSException.
        // catch_unwind handles Rust panics.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                use objc2::msg_send;
                use objc2::runtime::AnyObject;
                use objc2_foundation::{NSNumber, NSString};

                let wk_webview: *mut AnyObject =
                    webview.inner() as *mut _ as *mut AnyObject;
                if wk_webview.is_null() {
                    return;
                }

                let config: *mut AnyObject = msg_send![wk_webview, configuration];
                if config.is_null() {
                    return;
                }

                let prefs: *mut AnyObject = msg_send![config, preferences];
                if prefs.is_null() {
                    return;
                }

                // Box the boolean as NSNumber (the ObjC object type setValue:forKey: expects)
                let yes = NSNumber::new_bool(true);
                let key = NSString::from_str("_mediaCaptureEnabled");

                // Use setValue:forKey: (KVC) — prefs is an NSObject subclass
                let _: () = msg_send![prefs, setValue: &*yes, forKey: &*key];
            }))
        }));

        match result {
            Ok(Ok(())) => {
                eprintln!("enable_media_capture: success");
            }
            Ok(Err(objc_exception)) => {
                eprintln!(
                    "enable_media_capture: ObjC exception (mic capture unavailable): {:?}",
                    objc_exception
                );
            }
            Err(rust_panic) => {
                eprintln!(
                    "enable_media_capture: Rust panic (mic capture unavailable): {:?}",
                    rust_panic
                );
            }
        }
    });
}

/// Tauri command: enable media capture on the calling webview window.
/// Called from JS on mount (ChatApp.tsx) for dynamically-created windows.
#[tauri::command]
fn enable_media_capture_cmd(window: tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    enable_media_capture(&window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
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
            // Media capture is enabled per-window via enable_media_capture_cmd,
            // called from JS on mount. NOT done here — the WKWebView is not
            // fully initialized during did_finish_launching.
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
