//! The one accessibility setting our glass cannot see for itself.
//!
//! `prefers-reduced-motion` and `prefers-contrast` both work inside WKWebView.
//! `prefers-reduced-transparency` does not, and is not on its way — WebKit
//! declined it on fingerprinting grounds. So System Settings > Accessibility >
//! Display > "Reduce transparency" is completely invisible to the CSS in our
//! window, and a user who has turned it on would otherwise get exactly the
//! blurred, translucent chrome they asked the system not to give them.
//!
//! AppKit can see it, and we are AppKit. This module reads
//! `NSWorkspace.accessibilityDisplayShouldReduceTransparency` and pushes the
//! answer into the webview twice over: on demand via the `reduce_transparency`
//! command, and thereafter as an `accessibility-reduce-transparency` event
//! every time AppKit posts an accessibility-options change — so flipping the
//! switch in System Settings re-renders our glass opaque while the window is
//! open, with no reload.
//!
//! Follows the ObjC pattern already established in `main.rs`: raw `objc2`
//! messaging against a class looked up at runtime, with every failure path
//! degrading to "no, don't reduce" rather than to a crash. An accessibility
//! preference we cannot read must never be an app that will not start.

#[cfg(target_os = "macos")]
use tauri::{AppHandle, Emitter};

/// The event the webview listens for. Payload is the new boolean.
#[cfg(target_os = "macos")]
const CHANGED_EVENT: &str = "accessibility-reduce-transparency";

/// Read the current value. `false` off macOS, and `false` on any failure.
#[tauri::command]
pub fn reduce_transparency() -> bool {
    #[cfg(target_os = "macos")]
    {
        read_reduce_transparency()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn read_reduce_transparency() -> bool {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    // Wrapped the way `apply_media_capture` is: an ObjC exception here would
    // otherwise unwind through Rust with undefined behaviour.
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            let Some(cls) = AnyClass::get(c"NSWorkspace") else {
                return false;
            };
            let workspace: *mut AnyObject = msg_send![cls, sharedWorkspace];
            if workspace.is_null() {
                return false;
            }
            msg_send![workspace, accessibilityDisplayShouldReduceTransparency]
        }))
    }));

    match caught {
        Ok(Ok(v)) => v,
        _ => false,
    }
}

/// Subscribe to accessibility-display changes for the life of the app.
///
/// Call once, from `setup`, on the main thread. No-op off macOS.
pub fn watch(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        watch_macos(app);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "macos")]
fn watch_macos(app: &AppHandle) {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    // The notification's name constant, linked from AppKit rather than
    // reconstructed as a string literal — the two happen to be spelled the
    // same today, and relying on that is the kind of thing that breaks
    // silently and leaves a listener that never fires.
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {
        static NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification: *const AnyObject;
    }

    let handle = app.clone();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        objc2::exception::catch(std::panic::AssertUnwindSafe(move || unsafe {
            let Some(cls) = AnyClass::get(c"NSWorkspace") else {
                return;
            };
            let workspace: *mut AnyObject = msg_send![cls, sharedWorkspace];
            if workspace.is_null() {
                return;
            }
            // NSWorkspace has its OWN notification center. These notifications
            // are not posted to the default NSNotificationCenter, and asking
            // the wrong one is the classic way to get a listener that compiles,
            // registers, and never fires.
            let center: *mut AnyObject = msg_send![workspace, notificationCenter];
            if center.is_null() {
                return;
            }

            let name = NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification;
            if name.is_null() {
                return;
            }

            let block = RcBlock::new(move |_note: *mut AnyObject| {
                let _ = handle.emit(CHANGED_EVENT, read_reduce_transparency());
            });

            // queue: nil delivers synchronously on the posting thread, which
            // for this notification is the main thread. `emit` is thread-safe
            // either way.
            let observer: *mut AnyObject = msg_send![
                center,
                addObserverForName: name,
                object: std::ptr::null::<AnyObject>(),
                queue: std::ptr::null::<AnyObject>(),
                usingBlock: &*block,
            ];

            // The center owns the observer token and has copied the block; both
            // are meant to outlive this frame, and we never unsubscribe — the
            // subscription is app-lifetime by design.
            let _ = observer;
            std::mem::forget(block);
        }))
    }));

    if caught.is_err() {
        eprintln!("accessibility: could not observe reduce-transparency changes");
    }
}
