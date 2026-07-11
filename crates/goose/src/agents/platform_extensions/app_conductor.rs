//! App Conductor — lets the chat agent navigate the user to UI tabs/views.
//!
//! Exposes one tool: `navigate_app`. When called, it validates the tab name
//! against the app catalog, emits an `AppNavigate` event on the global bus,
//! and returns a confirmation to the agent.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::app_catalog::get_global_catalog;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "app_conductor";

// ── Tool parameter schema ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NavigateAppParams {
    /// User-facing tab name from the catalog (e.g. "Brain", "Settings", "Build").
    tab: String,
    /// Optional sub-section within the tab (e.g. "skills" within Automate).
    #[serde(default)]
    section: Option<String>,
    /// Optional opaque JSON state for the receiving view to interpret.
    #[serde(default)]
    state: Option<serde_json::Value>,
    /// Human-readable explanation shown in chat (e.g. "Taking you to the Brain view
    /// so you can see what I remember about X").
    reason: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct AppActionParams {
    /// The surface to act on: "chat" or "build".
    surface: String,
    /// The action to perform within the surface. Valid pairs:
    /// chat → open | close | detach; build → show_browser | hide_browser |
    /// show_terminal | hide_terminal.
    action: String,
    /// Optional opaque JSON params for the action (reserved; unused today).
    #[serde(default)]
    params: Option<serde_json::Value>,
    /// Human-readable explanation shown in chat (e.g. "Hiding the terminal so
    /// the browser fills the Build tab").
    reason: String,
}

/// The catalog of surface → actions the agent may drive. This is the single
/// source of truth the tool validates against (mirrors the tab catalog for
/// `navigate_app`), so an unknown pair is rejected with a helpful list rather
/// than emitting an event the frontend can't handle. The frontend dispatcher
/// mirrors these exact strings.
const ACTION_CATALOG: &[(&str, &[&str])] = &[
    ("chat", &["open", "close", "detach"]),
    (
        "build",
        &[
            "show_browser",
            "hide_browser",
            "show_terminal",
            "hide_terminal",
        ],
    ),
];

fn catalog_lists() -> String {
    ACTION_CATALOG
        .iter()
        .map(|(surface, actions)| format!("{} → {}", surface, actions.join(" | ")))
        .collect::<Vec<_>>()
        .join("; ")
}

fn action_is_valid(surface: &str, action: &str) -> bool {
    ACTION_CATALOG
        .iter()
        .any(|(s, actions)| *s == surface && actions.contains(&action))
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

// ── Client ──────────────────────────────────────────────────────────────────

pub struct AppConductorClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
}

impl AppConductorClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("App Conductor"),
            )
            .with_instructions(
                "Drive the Permagent app for the user. Use navigate_app to take \
                 them to a specific tab or view. Use app_action to operate a \
                 surface once there — open/close/detach the chat dock, or \
                 show/hide the Build tab's browser and terminal panes. Prefer \
                 doing it over describing it: these tools are the only way to \
                 actually change what the user sees.",
            );
        Ok(Self { info, context })
    }

    async fn handle_navigate(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: NavigateAppParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        let catalog =
            get_global_catalog().ok_or_else(|| "App catalog not initialized".to_string())?;

        let entry = catalog.find_by_name(&args.tab).ok_or_else(|| {
            format!(
                "I don't recognize the tab \"{}\". Available tabs are: {}",
                args.tab,
                catalog.tab_names().join(", ")
            )
        })?;

        // Speak-then-act seam: if a transport is intercepting navigations for
        // this session (a voice turn), hand the intent off to it instead of
        // emitting to the global bus — it will sequence the nav after the
        // narration has finished playing. Otherwise (text chat) emit as normal,
        // for instant navigation. `capture` is atomic with this decision.
        let intent = crate::events::nav_intercept::NavIntent {
            tab: entry.name.clone(),
            tool_type: entry.tool_type.clone(),
            panel_type: entry.panel_type.clone(),
            section: args.section.clone(),
            state: args.state.clone(),
            reason: args.reason.clone(),
        };
        if !crate::events::nav_intercept::capture(session_id, intent) {
            crate::events::emit(crate::events::app_navigate(
                &entry.name,
                &entry.tool_type,
                &entry.panel_type,
                args.section.as_deref(),
                args.state.as_ref(),
                &args.reason,
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigating to {}. {}",
            entry.name, args.reason
        ))]))
    }

    async fn handle_action(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: AppActionParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        if !action_is_valid(&args.surface, &args.action) {
            return Err(format!(
                "I can't do \"{}\" on \"{}\". Valid actions are: {}",
                args.action,
                args.surface,
                catalog_lists()
            ));
        }

        // Emit to the global bus; the frontend dispatcher calls the matching
        // store action. Unlike navigate_app there's no voice speak-then-act
        // intercept — an in-surface toggle is instantaneous and doesn't compete
        // with narration.
        crate::events::emit(crate::events::app_action(
            &args.surface,
            &args.action,
            args.params.as_ref(),
            &args.reason,
        ));

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Done: {} on {}. {}",
            args.action, args.surface, args.reason
        ))]))
    }
}

// ── MCP trait implementation ────────────────────────────────────────────────

#[async_trait]
impl McpClientTrait for AppConductorClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
        let tools = vec![
            Tool::new(
                "navigate_app".to_string(),
                "Navigate the user to a specific tab, optionally drilling into a \
                 sub-view. Call this whenever the user expresses intent to view, open, \
                 visit, or be taken to a tab. This is the ONLY way to actually change \
                 what the user sees — describing navigation in text does nothing.\n\n\
                 To open a specific project's detail/kanban view, navigate to the \
                 \"Projects\" tab with state: { \"project_id\": \"<uuid>\" }. Resolve \
                 the project name first with project_resolve to get the ID."
                    .to_string(),
                schema::<NavigateAppParams>(),
            ),
            Tool::new(
                "app_action".to_string(),
                "Act WITHIN a surface — not just navigate to it. Use this to \
                 operate the app on the user's behalf: open, close, or detach the \
                 chat dock, and show/hide the Build tab's browser or terminal \
                 pane. Like navigate_app, describing the action in text does \
                 nothing — you must call this to actually change the UI.\n\n\
                 Valid surface → action pairs: chat → open | close | detach; \
                 build → show_browser | hide_browser | show_terminal | \
                 hide_terminal. (build actions switch to the Build tab first if \
                 needed.)"
                    .to_string(),
                schema::<AppActionParams>(),
            ),
        ];

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "navigate_app" => self.handle_navigate(&ctx.session_id, arguments).await,
            "app_action" => self.handle_action(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
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
    fn valid_pairs_accepted() {
        assert!(action_is_valid("chat", "open"));
        assert!(action_is_valid("chat", "detach"));
        assert!(action_is_valid("build", "hide_browser"));
        assert!(action_is_valid("build", "show_terminal"));
    }

    #[test]
    fn unknown_pairs_rejected() {
        assert!(!action_is_valid("chat", "hide_browser")); // right action, wrong surface
        assert!(!action_is_valid("build", "detach")); // right surface, wrong action
        assert!(!action_is_valid("world", "open")); // unknown surface
        assert!(!action_is_valid("chat", "")); // empty action
    }

    #[test]
    fn catalog_lists_names_every_surface() {
        let listed = catalog_lists();
        assert!(listed.contains("chat →"));
        assert!(listed.contains("build →"));
        assert!(listed.contains("hide_browser"));
    }
}
