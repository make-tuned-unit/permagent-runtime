//! Platform-agnostic accessibility-tree model, filtering, and serialization.
//!
//! The macOS side (`macos.rs`) walks the real AX tree and produces plain
//! [`RawElement`] data — no CoreFoundation handles escape it. Everything
//! agent-facing lives here so it is unit-testable with fixtures on any OS:
//! stable ref assignment (`e0`, `e1`, …), interactive-element filtering,
//! value truncation, secure-field redaction, and the locator index used to
//! re-resolve an element at action time (elements are re-found by child-index
//! path and verified by role/title — we never hold live AX references between
//! tool calls, so a stale snapshot degrades to a clear error, not a wrong
//! click).

use std::collections::HashMap;

/// Max characters of an element's value rendered into a snapshot.
pub const MAX_VALUE_CHARS: usize = 200;
/// Default cap on elements rendered into one snapshot.
pub const DEFAULT_MAX_ELEMENTS: usize = 400;
/// Hard ceiling for the caller-supplied `max_elements`.
pub const MAX_MAX_ELEMENTS: usize = 2000;
/// Default AX-tree walk depth.
pub const DEFAULT_MAX_DEPTH: usize = 12;
/// Hard ceiling for the caller-supplied `max_depth`.
pub const MAX_MAX_DEPTH: usize = 25;
/// Hard ceiling on raw AX nodes visited in one walk (runtime bound).
pub const RAW_NODE_CAP: usize = 5000;

/// The AX role of a shielded password field — its value is never read,
/// rendered, or typed into.
pub const SECURE_TEXT_ROLE: &str = "AXSecureTextField";

/// Roles the interactive-only filter keeps (plus anything pressable/focused).
const INTERACTIVE_ROLES: &[&str] = &[
    "AXButton",
    "AXTextField",
    "AXTextArea",
    "AXSecureTextField",
    "AXSearchField",
    "AXCheckBox",
    "AXRadioButton",
    "AXPopUpButton",
    "AXMenuButton",
    "AXMenuItem",
    "AXMenuBarItem",
    "AXLink",
    "AXComboBox",
    "AXSlider",
    "AXIncrementor",
    "AXDisclosureTriangle",
    "AXSegmentedControl",
    "AXTabGroup",
];

/// Roles `desktop_type` may set the value of.
const TEXT_INPUT_ROLES: &[&str] = &["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"];

/// A running GUI application.
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub pid: i32,
    pub name: String,
    pub bundle_id: Option<String>,
    pub frontmost: bool,
}

/// One element frame in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One AX element as plain data. `children` preserve raw AX child order (a
/// possibly-truncated prefix), so a child's position in the vec IS its raw
/// AX index — the locator path depends on this.
#[derive(Debug, Clone, Default)]
pub struct RawElement {
    pub role: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    /// `Some(false)` renders as `(disabled)`; `None` = attribute absent.
    pub enabled: Option<bool>,
    pub focused: bool,
    pub frame: Option<Frame>,
    pub actions: Vec<String>,
    /// Children were omitted below this node (depth or node budget hit).
    pub truncated: bool,
    pub children: Vec<RawElement>,
}

/// How to find an element again at action time, without holding AX handles.
/// `path[0]` is the window's index in the app's `AXWindows` array; the rest
/// are `AXChildren` indices. Role/title are verified on re-resolution.
#[derive(Debug, Clone)]
pub struct Locator {
    pub path: Vec<usize>,
    pub role: String,
    pub title: Option<String>,
    pub secure: bool,
    pub pressable: bool,
    pub text_input: bool,
}

/// The stored result of one `desktop_tree` call for one session.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub pid: i32,
    pub app_name: String,
    pub locators: HashMap<String, Locator>,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotOptions {
    pub interactive_only: bool,
    pub max_elements: usize,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            interactive_only: true,
            max_elements: DEFAULT_MAX_ELEMENTS,
        }
    }
}

fn is_interactive(el: &RawElement) -> bool {
    INTERACTIVE_ROLES.contains(&el.role.as_str())
        || el.actions.iter().any(|a| a == "AXPress")
        || el.focused
}

fn has_content(el: &RawElement) -> bool {
    el.title.as_deref().is_some_and(|t| !t.trim().is_empty())
        || el.value.as_deref().is_some_and(|v| !v.trim().is_empty())
        || el
            .description
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty())
}

/// Keep-set decision for one node (ancestors of kept nodes are kept by the
/// recursion in `build_snapshot`, so this only judges the node itself).
fn keep_self(el: &RawElement, interactive_only: bool, is_window: bool) -> bool {
    if is_window {
        return true;
    }
    if interactive_only {
        is_interactive(el)
    } else {
        is_interactive(el) || has_content(el)
    }
}

fn truncate_value(v: &str) -> String {
    if v.chars().count() <= MAX_VALUE_CHARS {
        v.to_string()
    } else {
        let cut: String = v.chars().take(MAX_VALUE_CHARS).collect();
        format!("{cut}… (truncated)")
    }
}

fn render_line(el: &RawElement, id: &str, depth: usize, secure: bool) -> String {
    let mut line = format!("{}{} {}", "  ".repeat(depth), id, el.role);
    if let Some(t) = el.title.as_deref().filter(|t| !t.trim().is_empty()) {
        line.push_str(&format!(" {:?}", t));
    }
    if let Some(d) = el.description.as_deref().filter(|d| !d.trim().is_empty()) {
        if el.description != el.title {
            line.push_str(&format!(" desc={:?}", d));
        }
    }
    if secure {
        line.push_str(" value=(redacted: secure field)");
    } else if let Some(v) = el.value.as_deref().filter(|v| !v.trim().is_empty()) {
        line.push_str(&format!(" value={:?}", truncate_value(v)));
    }
    if let Some(f) = el.frame {
        line.push_str(&format!(" ({:.0},{:.0} {:.0}x{:.0})", f.x, f.y, f.w, f.h));
    }
    if !el.actions.is_empty() {
        line.push_str(&format!(" [{}]", el.actions.join(", ")));
    }
    if el.enabled == Some(false) {
        line.push_str(" (disabled)");
    }
    if el.focused {
        line.push_str(" (focused)");
    }
    if el.truncated {
        line.push_str(" (children truncated)");
    }
    line
}

struct BuildState {
    next_id: usize,
    rendered: usize,
    skipped: usize,
    max_elements: usize,
    interactive_only: bool,
    lines: Vec<String>,
    locators: HashMap<String, Locator>,
}

impl BuildState {
    /// Pre-order walk. Returns true if this subtree rendered anything.
    fn visit(&mut self, el: &RawElement, path: Vec<usize>, depth: usize, is_window: bool) -> bool {
        // Does any descendant need rendering? (Ancestors of kept nodes are
        // kept for structure, so compute descendant demand first.)
        let keep = keep_self(el, self.interactive_only, is_window)
            || subtree_has_keeper(el, self.interactive_only);
        if !keep {
            return false;
        }
        if self.rendered >= self.max_elements {
            self.skipped += 1;
            count_keepers(el, self.interactive_only, &mut self.skipped);
            return false;
        }

        let id = format!("e{}", self.next_id);
        self.next_id += 1;
        self.rendered += 1;
        let secure = el.role == SECURE_TEXT_ROLE;
        self.lines.push(render_line(el, &id, depth, secure));
        self.locators.insert(
            id,
            Locator {
                path: path.clone(),
                role: el.role.clone(),
                title: el.title.clone(),
                secure,
                pressable: el.actions.iter().any(|a| a == "AXPress"),
                text_input: TEXT_INPUT_ROLES.contains(&el.role.as_str()) && !secure,
            },
        );

        for (idx, child) in el.children.iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(idx);
            self.visit(child, child_path, depth + 1, false);
        }
        true
    }
}

fn subtree_has_keeper(el: &RawElement, interactive_only: bool) -> bool {
    el.children
        .iter()
        .any(|c| keep_self(c, interactive_only, false) || subtree_has_keeper(c, interactive_only))
}

fn count_keepers(el: &RawElement, interactive_only: bool, acc: &mut usize) {
    for c in &el.children {
        if keep_self(c, interactive_only, false) {
            *acc += 1;
        }
        count_keepers(c, interactive_only, acc);
    }
}

/// Build the agent-facing snapshot text plus the locator index from raw
/// windows (`windows[i]` must be the element at `AXWindows` index `i`).
pub fn build_snapshot(
    pid: i32,
    app_name: &str,
    windows: &[RawElement],
    opts: SnapshotOptions,
) -> (Snapshot, String) {
    let mut state = BuildState {
        next_id: 0,
        rendered: 0,
        skipped: 0,
        max_elements: opts.max_elements.clamp(1, MAX_MAX_ELEMENTS),
        interactive_only: opts.interactive_only,
        lines: Vec::new(),
        locators: HashMap::new(),
    };
    for (idx, w) in windows.iter().enumerate() {
        state.visit(w, vec![idx], 0, true);
    }

    let mut out = format!(
        "App: {} (pid {}) — {} window{}, {} element{} shown (interactive_only={})\n\
         Refs (e0, e1, …) are valid until the UI changes: act with desktop_click / desktop_type, then take a fresh desktop_tree snapshot.\n\n",
        app_name,
        pid,
        windows.len(),
        if windows.len() == 1 { "" } else { "s" },
        state.rendered,
        if state.rendered == 1 { "" } else { "s" },
        opts.interactive_only,
    );
    out.push_str(&state.lines.join("\n"));
    if state.skipped > 0 {
        out.push_str(&format!(
            "\n(+{} more matching elements truncated — raise max_elements, or narrow the view)",
            state.skipped
        ));
    }
    if windows.is_empty() {
        out.push_str("(no windows — the app is running but has no windows on this desktop)");
    }

    (
        Snapshot {
            pid,
            app_name: app_name.to_string(),
            locators: state.locators,
        },
        out,
    )
}

/// Resolve a user-facing app selector ("TextEdit" or "812") against the
/// running-app list. Pure so it is testable without a platform.
pub fn resolve_app_selector<'a>(
    selector: &str,
    apps: &'a [AppInfo],
) -> Result<&'a AppInfo, String> {
    let sel = selector.trim();
    if let Ok(pid) = sel.parse::<i32>() {
        return apps.iter().find(|a| a.pid == pid).ok_or_else(|| {
            format!("No running app with pid {pid}. Call desktop_apps to list running apps.")
        });
    }
    let lower = sel.to_lowercase();
    let exact: Vec<&AppInfo> = apps
        .iter()
        .filter(|a| a.name.to_lowercase() == lower)
        .collect();
    let matches: Vec<&AppInfo> = if exact.is_empty() {
        apps.iter()
            .filter(|a| a.name.to_lowercase().contains(&lower))
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [] => Err(format!(
            "No running app matches {sel:?}. Call desktop_apps to list running apps."
        )),
        [one] => Ok(one),
        many => Err(format!(
            "App selector {sel:?} is ambiguous: {}. Use the pid instead.",
            many.iter()
                .map(|a| format!("{} (pid {})", a.name, a.pid))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(role: &str, title: Option<&str>) -> RawElement {
        RawElement {
            role: role.to_string(),
            title: title.map(str::to_string),
            ..Default::default()
        }
    }

    fn window(title: &str, children: Vec<RawElement>) -> RawElement {
        RawElement {
            role: "AXWindow".to_string(),
            title: Some(title.to_string()),
            children,
            ..Default::default()
        }
    }

    fn fixture_windows() -> Vec<RawElement> {
        // Window
        // ├── AXGroup (boring)
        // │   ├── AXButton "Save" [AXPress]
        // │   └── AXStaticText "Ready"       (content, not interactive)
        // ├── AXGroup (boring, empty)        (dropped entirely)
        // └── AXTextField "Name"
        let mut save = el("AXButton", Some("Save"));
        save.actions = vec!["AXPress".to_string()];
        let mut status = el("AXStaticText", None);
        status.value = Some("Ready".to_string());
        let mut group = el("AXGroup", None);
        group.children = vec![save, status];
        let empty_group = el("AXGroup", None);
        let field = el("AXTextField", Some("Name"));
        vec![window("Doc", vec![group, empty_group, field])]
    }

    #[test]
    fn interactive_filter_keeps_ancestors_and_assigns_stable_ids() {
        let (snap, text) = build_snapshot(1, "App", &fixture_windows(), SnapshotOptions::default());
        // Kept: window e0, group e1 (ancestor), Save e2, field e3.
        // Dropped: static text (interactive_only), empty group.
        assert_eq!(snap.locators.len(), 4);
        assert!(text.contains("e0 AXWindow \"Doc\""));
        assert!(text.contains("e2 AXButton \"Save\""));
        assert!(text.contains("e3 AXTextField \"Name\""));
        assert!(!text.contains("Ready"));

        let save = &snap.locators["e2"];
        assert_eq!(save.path, vec![0, 0, 0]);
        assert!(save.pressable);
        assert!(!save.text_input);
        let field = &snap.locators["e3"];
        // Raw child index 2 — the dropped empty group must not shift paths.
        assert_eq!(field.path, vec![0, 2]);
        assert!(field.text_input);
    }

    #[test]
    fn full_mode_includes_content_elements() {
        let opts = SnapshotOptions {
            interactive_only: false,
            ..Default::default()
        };
        let (snap, text) = build_snapshot(1, "App", &fixture_windows(), opts);
        assert!(text.contains("value=\"Ready\""));
        assert_eq!(snap.locators.len(), 5);
    }

    #[test]
    fn max_elements_truncates_with_note() {
        let opts = SnapshotOptions {
            interactive_only: true,
            max_elements: 2,
        };
        let (snap, text) = build_snapshot(1, "App", &fixture_windows(), opts);
        assert_eq!(snap.locators.len(), 2);
        assert!(text.contains("more matching elements truncated"));
    }

    #[test]
    fn secure_fields_are_redacted_and_flagged() {
        let mut pw = el(SECURE_TEXT_ROLE, Some("Password"));
        pw.value = Some("hunter2".to_string());
        let windows = vec![window("Login", vec![pw])];
        let (snap, text) = build_snapshot(1, "App", &windows, SnapshotOptions::default());
        assert!(text.contains("value=(redacted: secure field)"));
        assert!(!text.contains("hunter2"));
        let loc = &snap.locators["e1"];
        assert!(loc.secure);
        assert!(!loc.text_input, "secure fields must not be typeable");
    }

    #[test]
    fn long_values_truncate() {
        let long = "x".repeat(MAX_VALUE_CHARS + 50);
        let mut txt = el("AXTextField", Some("Body"));
        txt.value = Some(long);
        let windows = vec![window("W", vec![txt])];
        let (_, text) = build_snapshot(1, "App", &windows, SnapshotOptions::default());
        assert!(text.contains("… (truncated)"));
    }

    #[test]
    fn state_markers_render() {
        let mut b = el("AXButton", Some("Go"));
        b.enabled = Some(false);
        b.focused = true;
        b.frame = Some(Frame {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
        });
        b.actions = vec!["AXPress".to_string()];
        let windows = vec![window("W", vec![b])];
        let (_, text) = build_snapshot(1, "App", &windows, SnapshotOptions::default());
        assert!(text.contains("(disabled)"));
        assert!(text.contains("(focused)"));
        assert!(text.contains("(10,20 30x40)"));
        assert!(text.contains("[AXPress]"));
    }

    #[test]
    fn app_selector_resolution() {
        let apps = vec![
            AppInfo {
                pid: 100,
                name: "TextEdit".into(),
                bundle_id: None,
                frontmost: false,
            },
            AppInfo {
                pid: 200,
                name: "Texture".into(),
                bundle_id: None,
                frontmost: true,
            },
        ];
        assert_eq!(resolve_app_selector("100", &apps).unwrap().pid, 100);
        assert_eq!(resolve_app_selector("textedit", &apps).unwrap().pid, 100);
        // Substring "text" matches both → ambiguous.
        assert!(resolve_app_selector("text", &apps)
            .unwrap_err()
            .contains("ambiguous"));
        assert!(resolve_app_selector("999", &apps).is_err());
        assert!(resolve_app_selector("Safari", &apps).is_err());
    }
}
