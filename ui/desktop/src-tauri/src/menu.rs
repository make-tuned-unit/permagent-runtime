use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

pub fn attach_menu(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        .menu(|app| {
            let app_menu = SubmenuBuilder::new(app, "Permagent")
                .item(&PredefinedMenuItem::about(
                    app,
                    Some("About Permagent"),
                    None,
                )?)
                .separator()
                .item(
                    &MenuItemBuilder::with_id("settings", "Settings...")
                        .accelerator("CmdOrCtrl+,")
                        .build(app)?,
                )
                .separator()
                .item(&PredefinedMenuItem::hide(app, Some("Hide Permagent"))?)
                .item(&PredefinedMenuItem::hide_others(app, Some("Hide Others"))?)
                .item(&PredefinedMenuItem::show_all(app, Some("Show All"))?)
                .separator()
                .item(&PredefinedMenuItem::quit(app, Some("Quit Permagent"))?)
                .build()?;

            let file_menu = SubmenuBuilder::new(app, "File")
                .item(
                    &MenuItemBuilder::with_id("new_chat", "New Chat")
                        .accelerator("CmdOrCtrl+N")
                        .build(app)?,
                )
                .item(&PredefinedMenuItem::close_window(
                    app,
                    Some("Close Window"),
                )?)
                .build()?;

            let edit_menu = SubmenuBuilder::new(app, "Edit")
                .item(&PredefinedMenuItem::undo(app, None)?)
                .item(&PredefinedMenuItem::redo(app, None)?)
                .separator()
                .item(&PredefinedMenuItem::cut(app, None)?)
                .item(&PredefinedMenuItem::copy(app, None)?)
                .item(&PredefinedMenuItem::paste(app, None)?)
                .item(&PredefinedMenuItem::select_all(app, None)?)
                .build()?;

            let view_menu = SubmenuBuilder::new(app, "View")
                .item(
                    &MenuItemBuilder::with_id("toggle_sidebar", "Toggle Sidebar")
                        .accelerator("CmdOrCtrl+/")
                        .build(app)?,
                )
                .separator()
                .item(
                    &MenuItemBuilder::with_id("reload", "Reload")
                        .accelerator("CmdOrCtrl+R")
                        .build(app)?,
                )
                .item(
                    &MenuItemBuilder::with_id("force_reload", "Force Reload")
                        .accelerator("CmdOrCtrl+Shift+R")
                        .build(app)?,
                )
                .build()?;

            let window_menu = SubmenuBuilder::new(app, "Window")
                .item(&PredefinedMenuItem::minimize(app, None)?)
                .item(&PredefinedMenuItem::maximize(app, Some("Zoom"))?)
                .build()?;

            let help_menu = SubmenuBuilder::new(app, "Help")
                .item(&MenuItemBuilder::with_id("docs", "Permagent Documentation").build(app)?)
                .build()?;

            MenuBuilder::new(app)
                .item(&app_menu)
                .item(&file_menu)
                .item(&edit_menu)
                .item(&view_menu)
                .item(&window_menu)
                .item(&help_menu)
                .build()
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "settings" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.eval("window.location.hash = '#/settings'");
                    }
                }
                "new_chat" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.eval("window.__PERMAGENT_MENU?.('new_chat')");
                    }
                }
                "toggle_sidebar" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.eval("window.__PERMAGENT_MENU?.('toggle_sidebar')");
                    }
                }
                "reload" | "force_reload" => {
                    // Park-and-close native browser children BEFORE the shell
                    // forgets their ids. `window.location.reload()` does not
                    // destroy child WKWebViews — they keep compositing over
                    // the new page, which is the stuck-overlay state
                    // (reported 2026-08-21). Await the sweep so the overlay
                    // is gone before the next paint, then reload.
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::browser::reap_orphan_browsers(
                            handle.clone(),
                            Vec::new(),
                            Some("main".into()),
                        )
                        .await;
                        if let Some(w) = handle.get_webview_window("main") {
                            let _ = w.eval("window.location.reload()");
                        }
                    });
                }
                "docs" => {
                    // Deprecated in tauri-plugin-shell in favor of
                    // tauri-plugin-opener. The migration is deferred (#566): the
                    // opener plugin needs a new dep + a capability entry that can
                    // only be runtime-verified on a machine that stages the app's
                    // sidecars — this call keeps working meanwhile.
                    #[allow(deprecated)]
                    let _ = app.shell().open("https://permagent.ai/docs", None);
                }
                _ => {}
            }
        })
}
