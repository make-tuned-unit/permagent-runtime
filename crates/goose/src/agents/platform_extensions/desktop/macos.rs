//! macOS implementation: the AX (accessibility) C API + NSWorkspace.
//!
//! Follows the house FFI style of `reader/vision_ocr.rs` — raw framework
//! linking with no new crate dependencies, every public entry wrapped in
//! `catch_unwind` + `objc2::exception::catch` so a failure degrades to an
//! error string, never a crash. All CoreFoundation handles live and die
//! inside this module (RAII via [`Owned`]); only plain-data
//! [`tree::RawElement`] values cross the boundary, which keeps the async
//! side `Send`-clean.
//!
//! Elements are re-resolved at action time from a child-index path recorded
//! at snapshot time and verified by role/title — a mismatch means the UI
//! changed and the action is refused as stale rather than mis-aimed.

use super::tree::{AppInfo, Frame, Locator, RawElement, SECURE_TEXT_ROLE};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use std::ffi::c_void;

// NSWorkspace lives in AppKit; the AX C API in ApplicationServices (HIServices).
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFIndex = isize;

#[repr(C)]
struct CFRange {
    location: CFIndex,
    length: CFIndex,
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_DOUBLE_TYPE: CFIndex = 13;
/// AXValueType wrapped-struct tags.
const K_AX_VALUE_CGPOINT: i32 = 1;
const K_AX_VALUE_CGSIZE: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyActionNames(element: CFTypeRef, names: *mut CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFStringRef) -> i32;
    fn AXUIElementSetAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout_seconds: f32) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: CFTypeRef, value_type: i32, out: *mut c_void) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFStringCreateWithBytes(
        alloc: CFTypeRef,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: u32,
        is_external_representation: u8,
    ) -> CFStringRef;
    fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    fn CFStringGetBytes(
        s: CFStringRef,
        range: CFRange,
        encoding: u32,
        loss_byte: u8,
        is_external_representation: u8,
        buffer: *mut u8,
        max_buf_len: CFIndex,
        used_buf_len: *mut CFIndex,
    ) -> CFIndex;
    fn CFArrayGetCount(arr: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(arr: CFTypeRef, idx: CFIndex) -> CFTypeRef;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    fn CFNumberGetValue(number: CFTypeRef, the_type: CFIndex, out: *mut c_void) -> u8;
}

/// Owned (+1 retained) CoreFoundation handle, released on drop.
struct Owned(CFTypeRef);

impl Owned {
    fn new(r: CFTypeRef) -> Option<Owned> {
        if r.is_null() {
            None
        } else {
            Some(Owned(r))
        }
    }

    /// Take shared ownership of a borrowed reference (e.g. a CFArray entry).
    fn retained(r: CFTypeRef) -> Option<Owned> {
        if r.is_null() {
            None
        } else {
            Some(Owned(unsafe { CFRetain(r) }))
        }
    }

    fn get(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cfstr(s: &str) -> Owned {
    let r = unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
            0,
        )
    };
    Owned::new(r).expect("CFStringCreateWithBytes failed")
}

fn cfstring_to_string(s: CFTypeRef) -> String {
    unsafe {
        let len = CFStringGetLength(s);
        if len <= 0 {
            return String::new();
        }
        let max = len.checked_mul(4).map(|n| n + 1).unwrap_or(0).max(1);
        let mut buf = vec![0u8; max as usize];
        let mut used: CFIndex = 0;
        let converted = CFStringGetBytes(
            s,
            CFRange {
                location: 0,
                length: len,
            },
            K_CF_STRING_ENCODING_UTF8,
            b'?',
            0,
            buf.as_mut_ptr(),
            max,
            &mut used,
        );
        if converted <= 0 || used <= 0 {
            return String::new();
        }
        buf.truncate(used as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// Copy an AX attribute; `Ok(None)` = attribute absent/no value, `Err` = AX error.
fn copy_attr(element: CFTypeRef, name: &str) -> Result<Option<Owned>, i32> {
    let attr = cfstr(name);
    let mut out: CFTypeRef = std::ptr::null();
    let code = unsafe { AXUIElementCopyAttributeValue(element, attr.get(), &mut out) };
    match code {
        0 => Ok(Owned::new(out)),
        // Absent attributes are normal tree variation, not failures.
        -25205 | -25212 | -25208 => Ok(None),
        other => Err(other),
    }
}

fn attr_string(element: CFTypeRef, name: &str) -> Option<String> {
    let v = copy_attr(element, name).ok().flatten()?;
    unsafe {
        if CFGetTypeID(v.get()) == CFStringGetTypeID() {
            Some(cfstring_to_string(v.get()))
        } else {
            None
        }
    }
}

fn attr_bool(element: CFTypeRef, name: &str) -> Option<bool> {
    let v = copy_attr(element, name).ok().flatten()?;
    unsafe {
        if CFGetTypeID(v.get()) == CFBooleanGetTypeID() {
            Some(CFBooleanGetValue(v.get()) != 0)
        } else {
            None
        }
    }
}

/// Render an element's `AXValue` attribute to text (string, number, or bool).
fn attr_value_string(element: CFTypeRef, name: &str) -> Option<String> {
    let v = copy_attr(element, name).ok().flatten()?;
    unsafe {
        let tid = CFGetTypeID(v.get());
        if tid == CFStringGetTypeID() {
            Some(cfstring_to_string(v.get()))
        } else if tid == CFNumberGetTypeID() {
            let mut d: f64 = 0.0;
            if CFNumberGetValue(
                v.get(),
                K_CF_NUMBER_DOUBLE_TYPE,
                &mut d as *mut f64 as *mut c_void,
            ) != 0
            {
                // Render integers without a trailing .0 for readability.
                if d.fract() == 0.0 && d.abs() < 1e15 {
                    Some(format!("{}", d as i64))
                } else {
                    Some(format!("{d}"))
                }
            } else {
                None
            }
        } else if tid == CFBooleanGetTypeID() {
            Some(if CFBooleanGetValue(v.get()) != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            })
        } else {
            None
        }
    }
}

fn attr_frame(element: CFTypeRef) -> Option<Frame> {
    unsafe {
        let pos = copy_attr(element, "AXPosition").ok().flatten()?;
        let size = copy_attr(element, "AXSize").ok().flatten()?;
        if CFGetTypeID(pos.get()) != AXValueGetTypeID()
            || CFGetTypeID(size.get()) != AXValueGetTypeID()
        {
            return None;
        }
        let mut p = CGPoint::default();
        let mut s = CGSize::default();
        if AXValueGetValue(
            pos.get(),
            K_AX_VALUE_CGPOINT,
            &mut p as *mut CGPoint as *mut c_void,
        ) == 0
            || AXValueGetValue(
                size.get(),
                K_AX_VALUE_CGSIZE,
                &mut s as *mut CGSize as *mut c_void,
            ) == 0
        {
            return None;
        }
        Some(Frame {
            x: p.x,
            y: p.y,
            w: s.width,
            h: s.height,
        })
    }
}

fn action_names(element: CFTypeRef) -> Vec<String> {
    let mut out: CFTypeRef = std::ptr::null();
    let code = unsafe { AXUIElementCopyActionNames(element, &mut out) };
    let Some(arr) = (if code == 0 { Owned::new(out) } else { None }) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    unsafe {
        let n = CFArrayGetCount(arr.get());
        for i in 0..n {
            let item = CFArrayGetValueAtIndex(arr.get(), i);
            if !item.is_null() && CFGetTypeID(item) == CFStringGetTypeID() {
                actions.push(cfstring_to_string(item));
            }
        }
    }
    actions
}

fn ax_err(code: i32) -> String {
    match code {
        -25211 => "accessibility permission not granted (kAXErrorAPIDisabled) — grant \
                   Accessibility to Permagent in System Settings → Privacy & Security → \
                   Accessibility, then retry (desktop_status shows the current state)"
            .to_string(),
        -25204 => "the app did not respond to the accessibility request \
                   (kAXErrorCannotComplete) — it may be busy, hung, or protected"
            .to_string(),
        -25202 => "the UI element no longer exists (kAXErrorInvalidUIElement) — the UI \
                   changed; take a fresh desktop_tree snapshot"
            .to_string(),
        -25205 => {
            "attribute unsupported by this element (kAXErrorAttributeUnsupported)".to_string()
        }
        -25206 => "action unsupported by this element (kAXErrorActionUnsupported)".to_string(),
        -25201 => "illegal argument (kAXErrorIllegalArgument)".to_string(),
        -25212 => "no value (kAXErrorNoValue)".to_string(),
        other => format!("AX error {other}"),
    }
}

const STALE_MSG: &str = "the element at this ref no longer matches the snapshot — the UI \
                         changed; take a fresh desktop_tree snapshot and use a new ref";

/// Belt-and-suspenders guard copied from `vision_ocr.rs`: ObjC exceptions and
/// Rust panics degrade to an error string.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        objc2::exception::catch(std::panic::AssertUnwindSafe(f))
    })) {
        Ok(Ok(r)) => r,
        Ok(Err(exc)) => Err(format!("{what}: ObjC exception: {exc:?}")),
        Err(_) => Err(format!("{what}: panicked")),
    }
}

pub(super) fn is_supported() -> bool {
    true
}

pub(super) fn ax_trusted() -> Option<bool> {
    Some(unsafe { AXIsProcessTrusted() } != 0)
}

/// Running GUI apps via NSWorkspace. `include_background` adds accessory
/// (menu-bar) and background apps; the default is regular windowed apps.
pub(super) fn list_apps(include_background: bool) -> Result<Vec<AppInfo>, String> {
    guarded("list_apps", || {
        objc2::rc::autoreleasepool(|_pool| unsafe {
            let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            if ws.is_null() {
                return Err("NSWorkspace unavailable".to_string());
            }
            let apps: *mut AnyObject = msg_send![ws, runningApplications];
            if apps.is_null() {
                return Err("runningApplications unavailable".to_string());
            }
            let count: usize = msg_send![apps, count];
            let mut out = Vec::new();
            for i in 0..count {
                let app: *mut AnyObject = msg_send![apps, objectAtIndex: i];
                if app.is_null() {
                    continue;
                }
                let policy: isize = msg_send![app, activationPolicy];
                // 0 = regular (Dock, windows), 1 = accessory, 2 = prohibited.
                if policy != 0 && !include_background {
                    continue;
                }
                let name_ns: *mut NSString = msg_send![app, localizedName];
                let name = if name_ns.is_null() {
                    String::new()
                } else {
                    (*name_ns).to_string()
                };
                if name.is_empty() {
                    continue;
                }
                let bid_ns: *mut NSString = msg_send![app, bundleIdentifier];
                let bundle_id = if bid_ns.is_null() {
                    None
                } else {
                    Some((*bid_ns).to_string())
                };
                let pid: i32 = msg_send![app, processIdentifier];
                let frontmost: bool = msg_send![app, isActive];
                out.push(AppInfo {
                    pid,
                    name,
                    bundle_id,
                    frontmost,
                });
            }
            out.sort_by(|a, b| {
                b.frontmost
                    .cmp(&a.frontmost)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            Ok(out)
        })
    })
}

fn require_trusted() -> Result<(), String> {
    if ax_trusted() == Some(true) {
        Ok(())
    } else {
        Err(ax_err(-25211))
    }
}

fn app_element(pid: i32) -> Result<Owned, String> {
    let app = Owned::new(unsafe { AXUIElementCreateApplication(pid) })
        .ok_or_else(|| format!("could not create AX element for pid {pid}"))?;
    // Bound every AX message so a hung target app cannot wedge the tool call.
    unsafe { AXUIElementSetMessagingTimeout(app.get(), 1.5) };
    Ok(app)
}

fn build_element(
    element: CFTypeRef,
    depth: usize,
    max_depth: usize,
    budget: &mut usize,
) -> RawElement {
    let role = attr_string(element, "AXRole").unwrap_or_else(|| "AXUnknown".to_string());
    let secure = role == SECURE_TEXT_ROLE;
    let mut el = RawElement {
        title: attr_string(element, "AXTitle"),
        description: attr_string(element, "AXDescription"),
        // A secure field's value is never read — redacted at the source.
        value: if secure {
            None
        } else {
            attr_value_string(element, "AXValue")
        },
        enabled: attr_bool(element, "AXEnabled"),
        focused: attr_bool(element, "AXFocused").unwrap_or(false),
        frame: attr_frame(element),
        actions: action_names(element),
        truncated: false,
        children: Vec::new(),
        role,
    };

    let children = copy_attr(element, "AXChildren").ok().flatten();
    if let Some(children) = children {
        let n = unsafe { CFArrayGetCount(children.get()) };
        if n > 0 && depth >= max_depth {
            el.truncated = true;
            return el;
        }
        for i in 0..n {
            if *budget == 0 {
                el.truncated = true;
                break;
            }
            let child = unsafe { CFArrayGetValueAtIndex(children.get(), i) };
            if child.is_null() {
                // Keep positional indices honest: paths index into AXChildren.
                el.children.push(RawElement {
                    role: "AXUnknown".to_string(),
                    ..Default::default()
                });
                continue;
            }
            *budget -= 1;
            el.children
                .push(build_element(child, depth + 1, max_depth, budget));
        }
    }
    el
}

/// Snapshot the app's windows into plain data. `windows[i]` corresponds to
/// `AXWindows` index `i` — the locator-path contract with `tree.rs`.
pub(super) fn snapshot_app(
    pid: i32,
    max_depth: usize,
    node_cap: usize,
) -> Result<Vec<RawElement>, String> {
    guarded("desktop_tree", || {
        require_trusted()?;
        let app = app_element(pid)?;
        let windows = copy_attr(app.get(), "AXWindows").map_err(ax_err)?;
        let Some(windows) = windows else {
            return Ok(Vec::new());
        };
        let n = unsafe { CFArrayGetCount(windows.get()) };
        let mut budget = node_cap;
        let mut out = Vec::new();
        for i in 0..n {
            let w = unsafe { CFArrayGetValueAtIndex(windows.get(), i) };
            if w.is_null() {
                out.push(RawElement {
                    role: "AXUnknown".to_string(),
                    ..Default::default()
                });
                continue;
            }
            out.push(build_element(w, 0, max_depth, &mut budget));
        }
        Ok(out)
    })
}

/// Re-resolve a locator path to a live element and verify identity.
fn resolve_element(pid: i32, locator: &Locator) -> Result<Owned, String> {
    if locator.path.is_empty() {
        return Err("invalid element locator (empty path)".to_string());
    }
    let app = app_element(pid)?;
    let windows = copy_attr(app.get(), "AXWindows")
        .map_err(ax_err)?
        .ok_or_else(|| STALE_MSG.to_string())?;
    let n = unsafe { CFArrayGetCount(windows.get()) } as usize;
    let widx = locator.path[0];
    if widx >= n {
        return Err(STALE_MSG.to_string());
    }
    let mut current =
        Owned::retained(unsafe { CFArrayGetValueAtIndex(windows.get(), widx as CFIndex) })
            .ok_or_else(|| STALE_MSG.to_string())?;

    for &idx in &locator.path[1..] {
        let children = copy_attr(current.get(), "AXChildren")
            .map_err(ax_err)?
            .ok_or_else(|| STALE_MSG.to_string())?;
        let n = unsafe { CFArrayGetCount(children.get()) } as usize;
        if idx >= n {
            return Err(STALE_MSG.to_string());
        }
        current =
            Owned::retained(unsafe { CFArrayGetValueAtIndex(children.get(), idx as CFIndex) })
                .ok_or_else(|| STALE_MSG.to_string())?;
    }

    // Identity check: the element at this path must still be what the
    // snapshot said it was, or the action is refused as stale.
    let role_now = attr_string(current.get(), "AXRole");
    if role_now.as_deref() != Some(locator.role.as_str()) {
        return Err(STALE_MSG.to_string());
    }
    if locator.title.is_some() {
        let title_now = attr_string(current.get(), "AXTitle");
        if title_now != locator.title {
            return Err(STALE_MSG.to_string());
        }
    }
    Ok(current)
}

/// Perform `AXPress` on the element a locator points at.
pub(super) fn press_element(pid: i32, locator: &Locator) -> Result<(), String> {
    guarded("desktop_click", || {
        require_trusted()?;
        let element = resolve_element(pid, locator)?;
        let action = cfstr("AXPress");
        let code = unsafe { AXUIElementPerformAction(element.get(), action.get()) };
        if code == 0 {
            Ok(())
        } else {
            Err(ax_err(code))
        }
    })
}

/// Replace the value of a text-input element with `text`.
pub(super) fn set_element_value(pid: i32, locator: &Locator, text: &str) -> Result<(), String> {
    guarded("desktop_type", || {
        require_trusted()?;
        let element = resolve_element(pid, locator)?;
        // Re-verify against the LIVE role — never type into a secure field,
        // even if the snapshot raced a field switching to secure entry.
        let role_now = attr_string(element.get(), "AXRole").unwrap_or_default();
        if role_now == SECURE_TEXT_ROLE {
            return Err("refusing to type into a secure (password) field".to_string());
        }
        let value = cfstr(text);
        let attr = cfstr("AXValue");
        let code = unsafe { AXUIElementSetAttributeValue(element.get(), attr.get(), value.get()) };
        if code == 0 {
            Ok(())
        } else {
            Err(ax_err(code))
        }
    })
}
