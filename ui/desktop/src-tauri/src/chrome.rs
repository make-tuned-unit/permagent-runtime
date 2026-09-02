//! Window chrome — where the traffic lights sit, and keeping them there.
//!
//! The shell runs `titleBarStyle: "Overlay"` + `hiddenTitle: true` with
//! `decorations: true`, so our own HTML runs edge-to-edge under a transparent
//! system titlebar while macOS keeps drawing the window's corner radius and
//! drop shadow for free. The window controls are then repositioned into the
//! sidebar's titlebar band with `trafficLightPosition`.
//!
//! THREE MEASURED FACTS DECIDE EVERYTHING IN THIS FILE. All three come from
//! the A1a spike (worktree `a1a-chrome-spike`, commit f31d9909), which drove a
//! harness pinned to this app's exact lock — tauri 2.11.0 / tao 0.35.0 /
//! wry 0.55.0, same `features = ["unstable"]` — and read its answers back out
//! of AppKit instead of eyeballing screenshots.
//!
//! 1. **Only the CONFIG path works.** `trafficLightPosition` set in
//!    `tauri.conf.json` is applied by tao and holds. The Rust
//!    `WebviewWindowBuilder::traffic_light_position` API is silently dropped
//!    while `unstable` is on (wry `mod.rs` takes the `add_child` branch, which
//!    never installs wry's own `WryWebViewParent` re-inset), and `unstable` is
//!    not droppable — `Window::add_child`, which the in-app browser is built
//!    on, is gated behind it. So the config is the single source of truth for
//!    the main window, and `reinset()` below is the source of truth for every
//!    window we open at runtime.
//!
//! 2. **`set_title` permanently un-does the inset.** Setting a window's title
//!    rebuilds the titlebar container view, and the buttons snap back to
//!    AppKit's natural (9, 9) and stay there (tauri#13044). Two live call
//!    sites do this to us: `daemon.rs` surfaces daemon failures in the title,
//!    and `browser.rs` mirrors a popup's document title. Without the re-inset
//!    below, a daemon hiccup permanently wrecks the titlebar of a running app.
//!
//! 3. **The geometry.** Buttons measure 14x14 with 23pt between their origins,
//!    and the button's own origin inside the rebuilt container is (9, 9). The
//!    container is sized `button_height + y` tall and pinned to the top of the
//!    window, which makes the *visible* top inset `y - 9` — that is why our
//!    y is 22 for a 13pt inset, and why `trafficLightPosition.y` must never be
//!    read as "distance from the top of the window".
//!
//! Vibrancy and `transparent: true` are deliberately absent: the same spike
//! measured `transparent: true` at +6 points of whole-GPU utilisation on a
//! *static* page on this M4 (0.12% -> 6.1%), for an always-on desktop agent.
//! Nothing here needs them — an overlay titlebar over an opaque window is the
//! whole Tahoe impression at no GPU cost.

/// Left inset of the close button, in points. Config and code must agree; the
/// test at the bottom of this file enforces that.
///
/// 12 rather than the 20 the research doc guessed at, because the sidebar rail
/// is now full-height and the window controls have to fit *inside* it: the
/// three buttons span `x + 60`, and the collapsed rail is 76pt wide
/// (`shell.rail.collapsed` in `tokens.ts`). 12 leaves 4pt of clearance; 20
/// would hang the zoom button over the rail's edge onto the content pane.
pub const TRAFFIC_LIGHT_X: f64 = 12.0;

/// The `y` that `trafficLightPosition` takes. NOT the visible top inset —
/// see fact 3 above. 22 yields a 13pt inset, which centres the 14pt buttons in
/// the 40pt titlebar band (`shell.titlebar` in `tokens.ts`).
pub const TRAFFIC_LIGHT_Y: f64 = 22.0;

/// Measured button size, in points (A1a: `size=14.0x14.0` on every button).
pub const BUTTON_SIZE: f64 = 14.0;

/// Measured distance between button origins (A1a: x = 9 / 32 / 55).
pub const BUTTON_SPACING: f64 = 23.0;

/// The button's own origin inside the titlebar container view, which AppKit
/// sets and we do not touch (A1a: `raw superview frame x=.. y=9.00` in every
/// single probe, before and after re-insetting).
pub const BUTTON_ORIGIN_IN_CONTAINER: f64 = 9.0;

/// Visible distance from the top of the window to the top of the buttons, for
/// a given `trafficLightPosition.y`.
///
/// This and `traffic_light_span` are the *specification* of the chrome rather
/// than steps in the runtime path — `apply_inset` sets frames, it does not need
/// to know what they come out to. They are exercised by this file's tests, and
/// mirrored in `ui/command-center/src/styles/tokens.ts`, where the CSS derives
/// the titlebar band and the rail's collapsed width from the same arithmetic.
/// Deleting them as "unused" would delete the reason the numbers are what they
/// are, which is exactly the drift the tests exist to catch.
#[allow(dead_code)]
///
/// Container height is `BUTTON_SIZE + y` and it is pinned to the window's top
/// edge; the button sits `BUTTON_ORIGIN_IN_CONTAINER` up from the container's
/// bottom. A1a measured y=24 -> 15pt and y=9 (the snap-back) -> 9pt... which
/// is the one case this formula does *not* describe, because the snap-back is
/// AppKit rebuilding the container at its own natural height rather than ours.
pub fn top_inset(y: f64) -> f64 {
    y - BUTTON_ORIGIN_IN_CONTAINER
}

/// Height the titlebar container view is given, for a given `y`.
pub fn container_height(button_height: f64, y: f64) -> f64 {
    button_height + y
}

/// Where that container's origin goes, in AppKit's bottom-left window space.
pub fn container_origin_y(window_height: f64, button_height: f64, y: f64) -> f64 {
    window_height - container_height(button_height, y)
}

/// X origin of the `index`-th window button (0 = close, 1 = miniaturize,
/// 2 = zoom).
pub fn button_x(x: f64, index: usize, spacing: f64) -> f64 {
    x + index as f64 * spacing
}

/// Total width the three window buttons occupy, measured from the window's
/// left edge to the right edge of the zoom button. The rail has to be at least
/// this wide or the controls hang off it. Specification, not runtime — see
/// `top_inset`.
#[allow(dead_code)]
pub fn traffic_light_span(x: f64) -> f64 {
    button_x(x, 2, BUTTON_SPACING) + BUTTON_SIZE
}

/// Which windows we own the chrome of, and therefore have to re-inset.
///
/// `None` means "leave AppKit's default alone", and it is the answer for two
/// windows on purpose:
///
///   * The browser's sign-in popups (`browser-popup-*`) are the one surface
///     where the user must be able to read the *site's* own title to see what
///     they are signing in to, so they keep an ordinary system titlebar.
///   * The `chat` window keeps one too, for now — it is created with plain
///     `decorations` and its React root (`ChatApp.tsx`, lane R4's file) does
///     not yet reserve a titlebar band. Insetting a window whose content does
///     not know about it would push the buttons down into a titlebar that is
///     still being drawn. It joins this list in the same change that gives
///     ChatApp the band; the request is in this lane's PR body.
///
/// The popup check has to come first — a popup label (`browser-popup-3`) is
/// also a `browser-` prefix match.
pub fn inset_for_label(label: &str) -> Option<(f64, f64)> {
    if label.starts_with("browser-popup-") {
        return None;
    }
    // `main` is fixed; detached panes are `<kind>-<uuid>` minted by
    // `lib/paneWindows.ts` and wear the same chrome (`PaneWindowApp.tsx`).
    if label == "main" || label.starts_with("browser-") || label.starts_with("terminal-") {
        return Some((TRAFFIC_LIGHT_X, TRAFFIC_LIGHT_Y));
    }
    None
}

/// Put this window's traffic lights back where they belong, if it is one of
/// ours. Safe to call from any thread and safe to call redundantly — it reads
/// the current geometry first and does nothing when the inset is already
/// correct, so wiring it to a firehose event like `Resized` costs two AppKit
/// property reads.
pub fn ensure_inset(window: &tauri::WebviewWindow) {
    let Some((x, y)) = inset_for_label(window.label()) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        // AppKit view geometry is main-thread-only. Window events and command
        // handlers already arrive there; a background thread (the daemon
        // supervisor, notably) does not.
        if objc2::MainThreadMarker::new().is_some() {
            apply_inset(window, x, y);
        } else {
            let w = window.clone();
            let _ = window.run_on_main_thread(move || apply_inset(&w, x, y));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (x, y);
    }
}

/// Reapply the inset the way tao/wry do it internally, against the live
/// `NSWindow`. Verbatim in shape from `wry`'s `inset_traffic_lights`
/// (`wkwebview/class/wry_web_view_parent.rs`) — the code that never runs for
/// us because `unstable` puts the webview on the `add_child` path — plus an
/// early-out so this is cheap to call often.
#[cfg(target_os = "macos")]
fn apply_inset(window: &tauri::WebviewWindow, x: f64, y: f64) {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    let Ok(ptr) = window.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    unsafe {
        let ns_window = &*(ptr as *mut NSWindow);
        let (Some(close), Some(miniaturize)) = (
            ns_window.standardWindowButton(NSWindowButton::CloseButton),
            ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        ) else {
            // A window without standard buttons is not one we can inset.
            return;
        };
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);

        // close -> its containing bar -> the titlebar container view.
        let Some(bar) = close.superview() else { return };
        let Some(container) = bar.superview() else {
            return;
        };

        let close_rect = NSView::frame(&close);
        let mut container_rect = NSView::frame(&container);
        let target_height = container_height(close_rect.size.height, y);

        // Already correct — the common case once the window is up, and what
        // makes this safe to call from a resize stream.
        if (close_rect.origin.x - x).abs() < 0.5
            && (container_rect.size.height - target_height).abs() < 0.5
        {
            return;
        }

        container_rect.size.height = target_height;
        container_rect.origin.y =
            container_origin_y(ns_window.frame().size.height, close_rect.size.height, y);
        container.setFrame(container_rect);

        // Read the spacing off the live buttons rather than trusting our
        // measured constant: it is AppKit's to choose, and it has changed
        // across macOS releases (the constant exists for the layout arithmetic
        // the rail depends on, not to drive this loop).
        let live_spacing = NSView::frame(&miniaturize).origin.x - close_rect.origin.x;
        let spacing = if live_spacing > 0.0 {
            live_spacing
        } else {
            // Buttons already stacked on top of each other (a half-applied
            // inset from a previous run, or a hidden miniaturize button): fall
            // back to the measured constant rather than collapsing all three
            // onto x.
            BUTTON_SPACING
        };

        let mut buttons = vec![close, miniaturize];
        if let Some(zoom) = zoom {
            buttons.push(zoom);
        }
        for (i, button) in buttons.into_iter().enumerate() {
            let mut rect = NSView::frame(&button);
            rect.origin.x = button_x(x, i, spacing);
            button.setFrameOrigin(rect.origin);
        }
    }
}

/// Re-inset on demand, for windows the frontend opens itself.
///
/// The chat and detached-pane windows are created through the JS
/// `WebviewWindow` constructor, which goes through the Rust builder — the path
/// A1a proved silently drops `trafficLightPosition` under `unstable`. So the
/// frontend sets `titleBarStyle`/`hiddenTitle` (those do survive the builder)
/// and calls this once the window exists.
#[tauri::command]
pub fn reinset_traffic_lights(app: tauri::AppHandle, window_label: String) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window(&window_label) {
        ensure_inset(&window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers A1a measured, restated as the arithmetic the shell depends
    /// on. `y` is not a top inset and this is the test that says so.
    #[test]
    fn geometry_matches_the_a1a_measurements() {
        // Spike, `unstable-baseline.log`: config {x:20, y:24} produced
        // x=20.00 y_from_top=15.00 on a 14x14 button.
        assert_eq!(top_inset(24.0), 15.0);
        assert_eq!(container_height(14.0, 24.0), 38.0);
        // Our own values: a 13pt inset centres 14pt buttons in a 40pt band.
        assert_eq!(top_inset(TRAFFIC_LIGHT_Y), 13.0);
        assert_eq!(top_inset(TRAFFIC_LIGHT_Y) + BUTTON_SIZE + 13.0, 40.0);
        // Container is pinned to the top edge of the window.
        assert_eq!(container_origin_y(800.0, 14.0, TRAFFIC_LIGHT_Y), 764.0);
    }

    #[test]
    fn buttons_are_evenly_spaced_from_x() {
        // Spike snap-back positions: x = 9 / 32 / 55.
        assert_eq!(button_x(9.0, 0, BUTTON_SPACING), 9.0);
        assert_eq!(button_x(9.0, 1, BUTTON_SPACING), 32.0);
        assert_eq!(button_x(9.0, 2, BUTTON_SPACING), 55.0);
    }

    /// The constraint that picked x = 12: with a full-height rail, the window
    /// controls have to fit inside the *collapsed* rail, which is 76pt wide
    /// (`shell.rail.collapsed`, mirrored by a fitness test in
    /// `styles/shell.test.ts`).
    #[test]
    fn traffic_lights_fit_inside_the_collapsed_rail() {
        const COLLAPSED_RAIL: f64 = 76.0;
        assert_eq!(traffic_light_span(TRAFFIC_LIGHT_X), 72.0);
        assert!(
            traffic_light_span(TRAFFIC_LIGHT_X) <= COLLAPSED_RAIL,
            "window controls would hang off the collapsed sidebar rail"
        );
        // The research doc's provisional x = 20 is what this rules out.
        assert!(traffic_light_span(20.0) > COLLAPSED_RAIL);
    }

    #[test]
    fn sign_in_popups_keep_their_system_titlebar() {
        // The prefix-order trap: a popup is also a `browser-` label.
        assert_eq!(inset_for_label("browser-popup-3"), None);
        assert_eq!(inset_for_label("browser-popup-0"), None);
        assert!(inset_for_label("browser-2f8c-uuid").is_some());
    }

    #[test]
    fn our_own_windows_are_insettable() {
        let ours = [
            "main",
            "terminal-9a1e0f0e-0000-4000-8000-000000000000",
            "browser-9a1e0f0e-0000-4000-8000-000000000000",
        ];
        for label in ours {
            assert_eq!(
                inset_for_label(label),
                Some((TRAFFIC_LIGHT_X, TRAFFIC_LIGHT_Y)),
                "{label} is one of ours and must be re-inset"
            );
        }
        assert_eq!(inset_for_label("some-plugin-window"), None);
        // Not yet: the chat window's React root has no titlebar band to inset
        // INTO. Insetting it would push the buttons into a titlebar the window
        // still draws. See the doc comment.
        assert_eq!(inset_for_label("chat"), None);
    }

    /// The one that actually protects the feature. A1a's binding verdict is
    /// that the CONFIG path is the only one that applies the position at all,
    /// so if these keys drift out of `tauri.conf.json` the traffic lights
    /// quietly return to (9, 9) and the re-inset above starts fighting a
    /// titlebar band that no longer matches the CSS.
    #[test]
    fn tauri_conf_still_declares_the_chrome() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let main = &conf["app"]["windows"][0];
        assert_eq!(main["label"], "main");
        assert_eq!(main["titleBarStyle"], "Overlay");
        assert_eq!(main["hiddenTitle"], true);
        // decorations MUST stay true: `false` forfeits the free macOS corner
        // radius and the native drop shadow (tauri#3481/#9287/#4243).
        assert_eq!(main["decorations"], true);
        assert_eq!(main["trafficLightPosition"]["x"], TRAFFIC_LIGHT_X);
        assert_eq!(main["trafficLightPosition"]["y"], TRAFFIC_LIGHT_Y);
        // No transparency: A1a measured it at ~+6 points of whole-GPU
        // utilisation at idle. If this assertion ever fails, that decision is
        // being reversed and it needs its own measurement and sign-off.
        assert!(main.get("transparent").is_none() || main["transparent"] == false);
    }
}
