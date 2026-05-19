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
                "Navigate the user to specific tabs in the Permagent app. \
                 Use navigate_app when the user asks where something is or you want \
                 to show them a specific view.",
            );
        Ok(Self { info, context })
    }

    async fn handle_navigate(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args: NavigateAppParams = arguments
            .map(|obj| serde_json::from_value(serde_json::Value::Object(obj)))
            .transpose()
            .map_err(|e| format!("Invalid arguments: {}", e))?
            .ok_or_else(|| "Missing arguments".to_string())?;

        let catalog = get_global_catalog()
            .ok_or_else(|| "App catalog not initialized".to_string())?;

        let entry = catalog.find_by_name(&args.tab).ok_or_else(|| {
            format!(
                "I don't recognize the tab \"{}\". Available tabs are: {}",
                args.tab,
                catalog.tab_names().join(", ")
            )
        })?;

        // Emit the navigation event
        crate::events::emit(crate::events::app_navigate(
            &entry.name,
            &entry.tool_type,
            &entry.panel_type,
            args.section.as_deref(),
            args.state.as_ref(),
            &args.reason,
        ));

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigating to {}. {}",
            entry.name, args.reason
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
        let tools = vec![Tool::new(
            "navigate_app".to_string(),
            "Navigate the user to a specific tab. Call this whenever the user \
             expresses intent to view, open, visit, or be taken to a tab. This is \
             the ONLY way to actually change what the user sees — describing \
             navigation in text does nothing."
                .to_string(),
            schema::<NavigateAppParams>(),
        )];

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "navigate_app" => self.handle_navigate(arguments).await,
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
