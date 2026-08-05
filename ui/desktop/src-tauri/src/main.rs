#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

mod activity;
mod browser;
mod daemon;
mod files;
mod menu;
mod system_audio;
mod terminal;

/// Enable media capture (microphone getUserMedia) on a Tauri webview window.
/// WKWebView does not expose navigator.mediaDevices by default; we set the
/// private `_mediaCaptureEnabled` preference via the ObjC runtime.
///
/// Uses objc2::exception::catch to handle ObjC NSExceptions (not just Rust
/// panics). A failure degrades gracefully — mic capture unavailable, app runs.
#[cfg(target_os = "macos")]
fn enable_media_capture(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| apply_media_capture(&webview));
}

/// Set `_mediaCaptureEnabled` on an already-resolved platform webview.
///
/// Split out from `enable_media_capture` so it can be applied to a CHILD
/// `Webview` as well as a `WebviewWindow`. The in-app browser (browser.rs) is a
/// child webview hosting third-party pages, and a child cannot enable itself:
/// `enable_media_capture_cmd` is invoked from JS, and the JS running in a
/// browser webview is Zoom's or Google Meet's, which has no Tauri bridge. So
/// the browser webview had no media capture at all and `getUserMedia` failed —
/// no video call could run in the app (reported 2026-08-04).
#[cfg(target_os = "macos")]
pub(crate) fn apply_media_capture(webview: &tauri::webview::PlatformWebview) {
    {
        // Wrap in both ObjC exception catch AND Rust catch_unwind.
        // objc2::exception::catch handles NSException.
        // catch_unwind handles Rust panics.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                use objc2::msg_send;
                use objc2::runtime::AnyObject;
                use objc2_foundation::{NSNumber, NSString};

                let wk_webview: *mut AnyObject = webview.inner() as *mut _ as *mut AnyObject;
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
    }
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

/// Re-order the chat window directly above the main window WITHOUT stealing
/// focus or coupling the two (the way `parent`/NSWindow.addChildWindow would).
///
/// The in-app browser is a native child webview of the MAIN window, so it rides
/// the main window's z-order and paints over any overlapping part of the chat
/// window the moment the main window comes forward — clipping the chat at the
/// browser's edge (#461/#406). We fix that by calling
/// `[chatWindow orderWindow:NSWindowAbove relativeTo:mainWindow.windowNumber]`
/// whenever the main window gains focus. Unlike `addChildWindow`, this is a
/// one-shot relative ordering: the chat window stays an INDEPENDENT top-level
/// window — it can enter fullscreen and tile into split-screen, and dragging
/// the main window does not move it. It also never floats above OTHER apps
/// (that was the global-`alwaysOnTop` overshoot of #461) — it is ordered only
/// relative to our own main window.
///
/// Guarded so it never fights macOS Spaces: if the chat window is fullscreen
/// (its own Space) or not visible, we skip the reorder entirely.
#[cfg(target_os = "macos")]
fn reorder_chat_above_main(chat: &tauri::WebviewWindow, main: &tauri::WebviewWindow) {
    // Skip when the chat window is on its own Space (fullscreen) or hidden —
    // reordering across Spaces is meaningless and risks flicker/focus fights.
    if chat.is_fullscreen().unwrap_or(false) || !chat.is_visible().unwrap_or(true) {
        return;
    }

    let chat_ns = match chat.ns_window() {
        Ok(p) if !p.is_null() => p,
        _ => return,
    };
    let main_ns = match main.ns_window() {
        Ok(p) if !p.is_null() => p,
        _ => return,
    };

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;

            let chat_obj = chat_ns as *mut AnyObject;
            let main_obj = main_ns as *mut AnyObject;

            // NSInteger windowNumber of the main window.
            let main_num: isize = msg_send![main_obj, windowNumber];
            // NSWindowAbove == 1 (NSWindowOrderingMode). Orders chat just above
            // main without making it key/main, so focus is NOT stolen.
            const NS_WINDOW_ABOVE: isize = 1;
            let _: () = msg_send![chat_obj, orderWindow: NS_WINDOW_ABOVE, relativeTo: main_num];
        }));
    }));
}

/// Tauri command: re-assert the chat window's stacking just above the main
/// window. Invoked from the main window's focus listener (ChatLauncher) while
/// the chat window is open. No-ops cleanly when either window is absent.
#[tauri::command]
fn raise_chat_above_main(app: tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if let (Some(chat), Some(main)) = (
            app.get_webview_window("chat"),
            app.get_webview_window("main"),
        ) {
            reorder_chat_above_main(&chat, &main);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Path of the main-thread liveness heartbeat file (`~/.permagent/logs/ui-heartbeat`).
fn heartbeat_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".permagent/logs/ui-heartbeat")
}

/// Main-thread liveness heartbeat (#562).
///
/// A background thread asks the **main thread** to stamp `ui-heartbeat` every
/// few seconds. The file's mtime advances only if the AppKit main run loop
/// actually drains the request — so a stale heartbeat is a direct, crisp signal
/// that the main thread is wedged (the idle-wedge failure). This must run on the
/// main thread: a JS or async-command heartbeat executes off-main (Tokio worker)
/// and would stay fresh *through* a main-thread wedge, detecting nothing.
///
/// The external `scripts/ui-wedge-watchdog.sh` reads this file to trigger a
/// stack capture. Best-effort: any write error is ignored (the app runs on).
fn start_main_thread_heartbeat(app: &tauri::AppHandle) {
    const INTERVAL_SECS: u64 = 5;
    let handle = app.clone();
    std::thread::spawn(move || {
        if let Some(dir) = heartbeat_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(INTERVAL_SECS));
            // Runs ON the main thread; if the run loop is wedged this closure
            // never executes and the heartbeat mtime goes stale.
            let _ = handle.run_on_main_thread(|| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let _ = std::fs::write(heartbeat_path(), stamp.to_string());
            });
        }
    });
}

/// Check the hosted updater manifest for a newer signed build and, if the user
/// confirms, download + install + relaunch. Spawned detached so it never
/// blocks startup; all failures are logged and swallowed (offline, no
/// manifest, signature mismatch → the app just runs the current version).
fn check_for_updates(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_updater::UpdaterExt;
        let updater = match handle.updater() {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[updater] unavailable: {e}");
                return;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => {
                eprintln!(
                    "[updater] update {} available (current {})",
                    update.version, update.current_version
                );
                // Download + install; Tauri verifies the signature against the
                // pubkey in tauri.conf before applying. Relaunch on success.
                match update
                    .download_and_install(|_chunk, _total| {}, || {})
                    .await
                {
                    Ok(()) => {
                        eprintln!("[updater] installed {}, relaunching", update.version);
                        handle.restart();
                    }
                    Err(e) => eprintln!("[updater] install failed: {e}"),
                }
            }
            Ok(None) => eprintln!("[updater] up to date"),
            Err(e) => eprintln!("[updater] check failed (offline?): {e}"),
        }
    });
}

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        // ── Pane-window teardown safety net (native, IPC-independent) ──
        //
        // A detached pane window's JS asks for its own destruction: it
        // preventDefaults CloseRequested, redocks its tabs, then destroys
        // itself — and every step of that travels over the DYING window's own
        // IPC bridge. When that bridge stalls, the whole chain stalls with it,
        // including any "fallback" invoke, which is why bounding the JS
        // teardown did not stop the window surviving (reported twice,
        // 2026-08-04). A rescue routed through the thing being rescued is not
        // a rescue.
        //
        // So the guarantee lives here instead. On CloseRequested for a pane
        // window we let the JS handler run its redock, but arm a native timer:
        // if the window is still alive when it fires, Rust destroys it
        // directly. No webview IPC is involved, so a wedged bridge cannot
        // block it. The happy path is unchanged — the window is already gone
        // and the timer finds nothing.
        .on_window_event(|window, event| {
            if !matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                return;
            }
            // Pane windows are labelled `<kind>-<uuid>` by paneWindows.ts
            // (`browser-…` / `terminal-…`). NOT "pane-…" — checking for that
            // prefix would silently never match. The app's own windows are
            // "main" and "chat", so neither is caught here. Child browser
            // WEBVIEWS are also `browser-<n>`, but they live in a different
            // namespace and `on_window_event` never fires for them.
            let label = window.label().to_string();
            let is_pane = label.starts_with("browser-") || label.starts_with("terminal-");
            if !is_pane {
                return;
            }
            // HIDE IT NOW, natively. Measured 2026-08-04: the JS teardown is
            // genuinely wedged — the window never closes itself and the native
            // timer does all the work — so the user sat watching a dead window
            // with the redocked content flashing behind it for the whole
            // fallback delay. The close was REQUESTED; the window is going
            // away either way, so there is no honest reason to keep painting
            // it while we wait. Hiding here (not from the pane's JS) is the
            // whole point: the pane's own IPC is the thing that is stuck.
            //
            // The window object stays alive, so the JS redock already in
            // flight keeps running against it and tabs still come home.
            let _ = window.hide();

            let app = window.app_handle().clone();
            std::thread::spawn(move || {
                // Still generous enough for the JS redock (2s budget) to hand
                // the tabs back before the window object goes away. The delay
                // is now invisible — the window is already hidden — so this is
                // a correctness margin, not something the user waits through.
                std::thread::sleep(std::time::Duration::from_millis(2500));
                if let Some(w) = app.get_window(&label) {
                    eprintln!(
                        "[permagent-app] pane window {label} did not close itself — destroying natively"
                    );
                    let _ = w.destroy();
                }
            });
        })
        .manage(terminal::PtySessions::new())
        .manage(browser::BrowserSessions::new())
        .manage(system_audio::SystemAudioState::default())
        .invoke_handler(tauri::generate_handler![
            terminal::spawn_pty_session,
            terminal::write_to_pty,
            terminal::resize_pty,
            terminal::kill_pty,
            terminal::get_pty_output,
            browser::create_browser_webview,
            system_audio::start_system_audio,
            system_audio::stop_system_audio,
            system_audio::system_audio_available,
            system_audio::read_audio_chunk,
            browser::reparent_browser,
            browser::navigate_browser,
            browser::update_browser_bounds,
            browser::hide_browser,
            browser::close_browser,
            browser::destroy_pane_window,
            browser::browser_current_url,
            browser::browser_nav_state,
            browser::browser_go,
            browser::zoom_browser,
            browser::get_page_content,
            browser::get_page_snapshot,
            browser::act_on_ref,
            files::read_dropped_file,
            daemon::get_daemon_token,
            activity::emit_activity,
            enable_media_capture_cmd,
            raise_chat_above_main,
        ])
        .setup(|app| {
            daemon::start_daemon(app.handle())?;
            // Auto-update check (#326/#378): on launch, ask the hosted manifest
            // if a newer signed build exists. Non-blocking, failure-tolerant —
            // an offline or manifest-less launch is a no-op, never a crash.
            check_for_updates(app.handle().clone());
            // Main-thread liveness heartbeat (#562) — lets the external watchdog
            // detect an idle-wedge of the AppKit main run loop.
            start_main_thread_heartbeat(app.handle());
            // Media capture is enabled per-window via enable_media_capture_cmd,
            // called from JS on mount. NOT done here — the WKWebView is not
            // fully initialized during did_finish_launching.
            Ok(())
        });

    let builder = menu::attach_menu(builder);

    builder
        .build(tauri::generate_context!())
        .expect("error while running Permagent")
        .run(|app_handle, event| {
            // Kill the spawned daemon sidecar (if any) when the APP exits —
            // not when the main window closes (the chat window may still be
            // open and using it). tauri-plugin-shell's own Exit hook only
            // kills JS-spawned children, so this is the only kill path for
            // our Rust-side spawn. No-op for launchd/user-managed daemons.
            if let tauri::RunEvent::Exit = event {
                daemon::stop_daemon(app_handle);
            }
        });
}
