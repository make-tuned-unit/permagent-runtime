//! Desktop Control — local-first native-app grounding and action via the
//! macOS accessibility tree (slice 1 of issue #649).
//!
//! Perception and action are grounded in REAL UI elements (`AXUIElement`),
//! never screenshots or pixel coordinates: nothing is captured off-screen and
//! nothing leaves the machine to see the UI. Slice 1 ships reads
//! (`desktop_status`, `desktop_apps`, `desktop_tree`) plus the smallest safe
//! action set — press an element (`desktop_click`) and replace a text field's
//! value (`desktop_type`) by element ref.
//!
//! Safety posture (defense in depth):
//! - **Runtime flag, default OFF**: every tool except `desktop_status` is
//!   gated behind `DESKTOP_CONTROL_ENABLED` (config or env), re-checked on
//!   every call — the `LIBRARIAN_ATOMS_ENABLED` pattern. The extension is
//!   also `default_enabled: false` in the registry.
//! - **OS permission**: macOS Accessibility (TCC) must be granted; the AX
//!   API fails closed without it and `desktop_status` explains how to grant.
//! - **Approval seam**: the action tools are annotated non-read-only /
//!   destructive / open-world exactly like `shell`, so the existing
//!   permission inspector routes them to confirmation (Decision Inbox) under
//!   Approve/SmartApprove modes.
//! - **Element identity**: actions re-resolve the element by recorded path
//!   and verify role/title — a changed UI refuses as stale instead of
//!   mis-clicking. Secure (password) fields are redacted on read and refused
//!   on type.

pub mod tree;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

/// Non-macOS stub: same surface, honest errors. Keeps Linux CI green.
#[cfg(not(target_os = "macos"))]
mod platform {
    use super::tree::{AppInfo, Locator, RawElement};

    pub(super) const UNSUPPORTED: &str = "Desktop Control is only available on macOS \
        (accessibility-tree grounding uses the macOS AX API); this platform is unsupported.";

    pub(super) fn is_supported() -> bool {
        false
    }

    pub(super) fn ax_trusted() -> Option<bool> {
        None
    }

    pub(super) fn list_apps(_include_background: bool) -> Result<Vec<AppInfo>, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(super) fn snapshot_app(
        _pid: i32,
        _max_depth: usize,
        _node_cap: usize,
    ) -> Result<Vec<RawElement>, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(super) fn press_element(_pid: i32, _locator: &Locator) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    pub(super) fn set_element_value(
        _pid: i32,
        _locator: &Locator,
        _text: &str,
    ) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
}

use std::collections::HashMap;
use std::sync::Mutex;

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "desktop";

/// Runtime kill-switch for the whole capability. Default OFF.
pub const FLAG_KEY: &str = "DESKTOP_CONTROL_ENABLED";

fn flag_enabled() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(FLAG_KEY)
        .unwrap_or(false)
}

fn disabled_message() -> String {
    format!(
        "Desktop Control is disabled. Ask the user to set {FLAG_KEY}=true (config or \
         environment) to enable it, and to grant macOS Accessibility permission \
         (desktop_status shows the current state). It ships off by default because it \
         can act on the user's other applications."
    )
}

fn render_status(supported: bool, flag_on: bool, trusted: Option<bool>) -> String {
    let mut out = String::from("Desktop Control status:\n");
    out.push_str(if supported {
        "- Platform: macOS (supported)\n"
    } else {
        "- Platform: unsupported (Desktop Control is macOS-only; grounding uses the AX accessibility API)\n"
    });
    if flag_on {
        out.push_str(&format!("- Flag {FLAG_KEY}: on\n"));
    } else {
        out.push_str(&format!(
            "- Flag {FLAG_KEY}: off (default) — set {FLAG_KEY}=true in config or environment to enable\n"
        ));
    }
    match trusted {
        Some(true) => out.push_str("- macOS Accessibility permission: granted\n"),
        Some(false) => out.push_str(
            "- macOS Accessibility permission: NOT granted — grant it to Permagent in \
             System Settings → Privacy & Security → Accessibility (the permission \
             attaches to the app bundle, covering the bundled daemon), then retry\n",
        ),
        None => out.push_str("- macOS Accessibility permission: n/a (unsupported platform)\n"),
    }
    if supported && flag_on && trusted == Some(true) {
        out.push_str(
            "\nReady: desktop_apps lists running apps, desktop_tree snapshots one, and \
             desktop_click / desktop_type act on its elements by ref.",
        );
    }
    out
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct StatusParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct AppsParams {
    /// Also include menu-bar (accessory) and background apps. Default false:
    /// regular windowed apps only.
    #[serde(default)]
    include_background: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TreeParams {
    /// Which app to snapshot: an app name (e.g. "TextEdit") or a pid from
    /// desktop_apps.
    app: String,
    /// Only interactive elements (buttons, fields, menus, links…) plus their
    /// containers. Default true. Pass false to also read static text.
    #[serde(default)]
    interactive_only: Option<bool>,
    /// Cap on elements returned (default 400, max 2000).
    #[serde(default)]
    max_elements: Option<u32>,
    /// Cap on tree depth walked (default 12, max 25).
    #[serde(default)]
    max_depth: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ClickParams {
    /// An element ref (e.g. "e12") from the latest desktop_tree snapshot.
    element_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TypeParams {
    /// An element ref (e.g. "e12") from the latest desktop_tree snapshot.
    element_id: String,
    /// The new value. REPLACES the field's entire current value.
    text: String,
}

pub struct DesktopClient {
    info: InitializeResult,
    _context: PlatformExtensionContext,
    /// Latest snapshot per session — actions only accept refs minted by the
    /// most recent desktop_tree call for that session.
    snapshots: Mutex<HashMap<String, tree::Snapshot>>,
}

impl DesktopClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Desktop Control"),
            )
            .with_instructions(
                indoc! {r#"
                Desktop Control (accessibility-tree grounding)

                Ground every action in real UI elements, never pixels:
                1. desktop_status — platform support, the DESKTOP_CONTROL_ENABLED flag, and macOS Accessibility permission. Check it first when anything fails.
                2. desktop_apps — list running apps (name, pid, frontmost).
                3. desktop_tree — snapshot an app's windows and elements as stable refs (e0, e1, ...). Interactive elements only by default; pass interactive_only=false to also read static text.
                4. desktop_click / desktop_type — act on an element by its ref from the LATEST snapshot.

                Rules:
                - Refs go stale: after any action, or if the app's UI changed, take a fresh desktop_tree snapshot before acting again.
                - desktop_type REPLACES the whole field value.
                - Secure (password) fields are redacted and refused — never work around that.
                - Acting on the user's desktop is high-consequence: prefer small, reversible steps and say what you are about to do before you do it.
            "#}
                .to_string(),
            );
        Ok(Self {
            info,
            _context: context,
            snapshots: Mutex::new(HashMap::new()),
        })
    }

    /// The full, static tool inventory (the superset `list_tools` selects
    /// from — same contract as Extension Manager's `all_possible_tools`).
    /// The self-knowledge completeness guard derives its inventory from this,
    /// so every tool here must be named in the registry `description`.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "desktop_status".to_string(),
                "Report Desktop Control's state: platform support, the \
                 DESKTOP_CONTROL_ENABLED flag, and whether macOS Accessibility \
                 permission is granted — with how to fix whatever is missing. \
                 Always available; run it first when any desktop tool fails."
                    .to_string(),
                schema::<StatusParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Desktop status".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "desktop_apps".to_string(),
                "List running desktop applications (name, pid, bundle id, frontmost). \
                 Use the name or pid with desktop_tree to snapshot an app's UI."
                    .to_string(),
                schema::<AppsParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("List desktop apps".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "desktop_tree".to_string(),
                "Snapshot a running app's real UI elements from the macOS accessibility \
                 tree — windows, roles, titles, values, frames, and available actions — \
                 each with a stable ref (e0, e1, ...). Local-first: nothing is \
                 screenshotted and nothing leaves the machine. Refs are only valid until \
                 the UI changes; re-snapshot after every action. Secure (password) \
                 fields are always redacted."
                    .to_string(),
                schema::<TreeParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Read app UI tree".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "desktop_click".to_string(),
                "Press a UI element (button, menu item, link, checkbox...) in another \
                 app, by its ref from the LATEST desktop_tree snapshot. Element-grounded \
                 (AXPress on the real control), never a pixel click. Fails safe if the \
                 UI changed since the snapshot. This acts on the user's desktop — be \
                 deliberate, and re-snapshot after each press."
                    .to_string(),
                schema::<ClickParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Click desktop element".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
            )),
            Tool::new(
                "desktop_type".to_string(),
                "Replace the text value of an editable field in another app, by its ref \
                 from the LATEST desktop_tree snapshot. Sets the accessibility value of \
                 the real element (no keystroke injection), REPLACING its entire current \
                 value. Refuses secure (password) fields. Fails safe if the UI changed \
                 since the snapshot."
                    .to_string(),
                schema::<TypeParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Type into desktop element".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
            )),
        ]
    }

    /// The tools actually exposed for a given flag state: everything when the
    /// capability is enabled, only `desktop_status` when disabled (so the
    /// agent can still explain how to turn it on). Pure for testability;
    /// `call_tool` re-checks the flag so this is presentational, not the gate.
    fn tools_for(flag_on: bool) -> Vec<Tool> {
        let mut tools = Self::get_tools();
        if !flag_on {
            tools.retain(|t| t.name == "desktop_status");
        }
        tools
    }

    fn parse<T: for<'de> Deserialize<'de>>(
        arguments: Option<JsonObject>,
    ) -> std::result::Result<T, String> {
        serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
            .map_err(|e| format!("Invalid arguments: {e}"))
    }

    async fn handle_status(&self) -> std::result::Result<String, String> {
        let supported = platform::is_supported();
        let flag_on = flag_enabled();
        let trusted = tokio::task::spawn_blocking(platform::ax_trusted)
            .await
            .map_err(|e| e.to_string())?;
        Ok(render_status(supported, flag_on, trusted))
    }

    async fn handle_apps(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<String, String> {
        let params: AppsParams = Self::parse(arguments)?;
        let include_background = params.include_background.unwrap_or(false);
        let apps = tokio::task::spawn_blocking(move || platform::list_apps(include_background))
            .await
            .map_err(|e| e.to_string())??;
        if apps.is_empty() {
            return Ok("No running apps found.".to_string());
        }
        let mut out = format!("{} running app(s):\n", apps.len());
        for a in &apps {
            out.push_str(&format!(
                "- {} (pid {}){}{}\n",
                a.name,
                a.pid,
                if a.frontmost { " [frontmost]" } else { "" },
                a.bundle_id
                    .as_deref()
                    .map(|b| format!(" — {b}"))
                    .unwrap_or_default(),
            ));
        }
        out.push_str("\nUse desktop_tree with a name or pid to snapshot an app's UI.");
        Ok(out)
    }

    async fn handle_tree(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<String, String> {
        let params: TreeParams = Self::parse(arguments)?;
        let opts = tree::SnapshotOptions {
            interactive_only: params.interactive_only.unwrap_or(true),
            max_elements: (params
                .max_elements
                .unwrap_or(tree::DEFAULT_MAX_ELEMENTS as u32) as usize)
                .clamp(1, tree::MAX_MAX_ELEMENTS),
        };
        let max_depth = (params.max_depth.unwrap_or(tree::DEFAULT_MAX_DEPTH as u32) as usize)
            .clamp(1, tree::MAX_MAX_DEPTH);

        let selector = params.app.clone();
        let (pid, app_name) = tokio::task::spawn_blocking(move || {
            let apps = platform::list_apps(true)?;
            let app = tree::resolve_app_selector(&selector, &apps)?;
            Ok::<_, String>((app.pid, app.name.clone()))
        })
        .await
        .map_err(|e| e.to_string())??;

        let windows = tokio::task::spawn_blocking(move || {
            platform::snapshot_app(pid, max_depth, tree::RAW_NODE_CAP)
        })
        .await
        .map_err(|e| e.to_string())??;

        let (snapshot, rendered) = tree::build_snapshot(pid, &app_name, &windows, opts);
        self.snapshots
            .lock()
            .expect("desktop snapshot lock poisoned")
            .insert(session_id.to_string(), snapshot);
        Ok(rendered)
    }

    fn locator_for(
        &self,
        session_id: &str,
        element_id: &str,
    ) -> std::result::Result<(i32, String, tree::Locator), String> {
        let snapshots = self
            .snapshots
            .lock()
            .expect("desktop snapshot lock poisoned");
        let snapshot = snapshots.get(session_id).ok_or_else(|| {
            "No desktop_tree snapshot in this session yet — call desktop_tree first, then \
             act on its refs."
                .to_string()
        })?;
        let locator = snapshot.locators.get(element_id).ok_or_else(|| {
            format!(
                "Unknown element ref {element_id:?} — it is not in the latest desktop_tree \
                 snapshot ({} of {}). Take a fresh snapshot and use a current ref.",
                snapshot.app_name, snapshot.pid
            )
        })?;
        Ok((snapshot.pid, snapshot.app_name.clone(), locator.clone()))
    }

    async fn handle_click(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<String, String> {
        let params: ClickParams = Self::parse(arguments)?;
        let (pid, app_name, locator) = self.locator_for(session_id, &params.element_id)?;
        if !locator.pressable {
            return Err(format!(
                "Element {} ({} {:?}) does not advertise a press action — pick an element \
                 shown with [AXPress] in the snapshot.",
                params.element_id,
                locator.role,
                locator.title.as_deref().unwrap_or("")
            ));
        }
        tokio::task::spawn_blocking(move || platform::press_element(pid, &locator))
            .await
            .map_err(|e| e.to_string())??;
        Ok(format!(
            "Pressed {} in {app_name}. The UI may have changed — take a fresh desktop_tree \
             snapshot before the next action.",
            params.element_id
        ))
    }

    async fn handle_type(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<String, String> {
        let params: TypeParams = Self::parse(arguments)?;
        let (pid, app_name, locator) = self.locator_for(session_id, &params.element_id)?;
        if locator.secure {
            return Err(
                "Refusing to type into a secure (password) field — the user must enter \
                 secrets themselves."
                    .to_string(),
            );
        }
        if !locator.text_input {
            return Err(format!(
                "Element {} ({} {:?}) is not an editable text field.",
                params.element_id,
                locator.role,
                locator.title.as_deref().unwrap_or("")
            ));
        }
        let text = params.text.clone();
        tokio::task::spawn_blocking(move || platform::set_element_value(pid, &locator, &text))
            .await
            .map_err(|e| e.to_string())??;
        Ok(format!(
            "Set the value of {} in {app_name} ({} chars). Take a fresh desktop_tree \
             snapshot to verify the result.",
            params.element_id,
            params.text.chars().count()
        ))
    }
}

#[async_trait]
impl McpClientTrait for DesktopClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::tools_for(flag_enabled()),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let session_id = &ctx.session_id;
        // The flag is THE gate and is re-checked on every call — a stale tool
        // list can never act through a disabled capability.
        let result = match name {
            "desktop_status" => self.handle_status().await,
            "desktop_apps" | "desktop_tree" | "desktop_click" | "desktop_type"
                if !flag_enabled() =>
            {
                Err(disabled_message())
            }
            "desktop_apps" => self.handle_apps(arguments).await,
            "desktop_tree" => self.handle_tree(session_id, arguments).await,
            "desktop_click" => self.handle_click(session_id, arguments).await,
            "desktop_type" => self.handle_type(session_id, arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_inventory_has_the_five_tools() {
        let names: Vec<_> = DesktopClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "desktop_status",
                "desktop_apps",
                "desktop_tree",
                "desktop_click",
                "desktop_type"
            ]
        );
    }

    #[test]
    fn flag_off_exposes_only_status() {
        let names: Vec<_> = DesktopClient::tools_for(false)
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, vec!["desktop_status"]);
        assert_eq!(DesktopClient::tools_for(true).len(), 5);
    }

    #[test]
    fn actions_are_annotated_dangerous_reads_are_read_only() {
        for tool in DesktopClient::get_tools() {
            let ann = tool.annotations.as_ref().expect("every tool is annotated");
            match tool.name.as_ref() {
                "desktop_click" | "desktop_type" => {
                    // Same shape as developer shell: not read-only, destructive,
                    // open-world — the permission inspector must gate these.
                    assert_eq!(ann.read_only_hint, Some(false), "{}", tool.name);
                    assert_eq!(ann.destructive_hint, Some(true), "{}", tool.name);
                    assert_eq!(ann.open_world_hint, Some(true), "{}", tool.name);
                }
                _ => {
                    assert_eq!(ann.read_only_hint, Some(true), "{}", tool.name);
                    assert_eq!(ann.destructive_hint, Some(false), "{}", tool.name);
                }
            }
        }
    }

    #[test]
    fn status_rendering_covers_the_gating_matrix() {
        let ready = render_status(true, true, Some(true));
        assert!(ready.contains("macOS (supported)"));
        assert!(ready.contains("on\n"));
        assert!(ready.contains("granted"));
        assert!(ready.contains("Ready:"));

        let no_perm = render_status(true, true, Some(false));
        assert!(no_perm.contains("NOT granted"));
        assert!(no_perm.contains("System Settings"));
        assert!(!no_perm.contains("Ready:"));

        let flag_off = render_status(true, false, Some(true));
        assert!(flag_off.contains("off (default)"));
        assert!(flag_off.contains(FLAG_KEY));
        assert!(!flag_off.contains("Ready:"));

        let unsupported = render_status(false, false, None);
        assert!(unsupported.contains("unsupported"));
        assert!(unsupported.contains("n/a"));
    }

    #[test]
    fn disabled_message_names_the_flag() {
        assert!(disabled_message().contains(FLAG_KEY));
        assert!(disabled_message().contains("desktop_status"));
    }
}
