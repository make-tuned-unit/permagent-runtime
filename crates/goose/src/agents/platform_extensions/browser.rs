use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "browser";

#[derive(Debug, Serialize, Deserialize)]
struct PageContent {
    title: String,
    url: String,
    content: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    truncated: bool,
}

pub struct BrowserClient {
    info: InitializeResult,
}

impl BrowserClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("Browser"),
            );
        Ok(Self { info })
    }

    async fn handle_read_browser_content(&self) -> Result<Vec<Content>, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("http://127.0.0.1:3001/api/browser/content/read")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("Failed to request page content: {e}"))?;

        if resp.status() == reqwest::StatusCode::GATEWAY_TIMEOUT {
            return Ok(vec![Content::text(
                "No browser tab is open, or the page content could not be extracted. \
                 Make sure a page is loaded in the Permagent browser.",
            )]);
        }

        if !resp.status().is_success() {
            return Err(format!(
                "Content extraction failed with status {}",
                resp.status()
            ));
        }

        let page: PageContent = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse page content: {e}"))?;

        // Surface distinct failure modes for agent reasoning
        if page.status == "no_tab" {
            return Ok(vec![Content::text(
                "No browser tab is currently open in Permagent. \
                 The user needs to have a page open in the browser for this tool to work.",
            )]);
        }
        if page.status == "error" {
            return Ok(vec![Content::text(format!(
                "Could not read the page content: {}",
                page.content
            ))]);
        }

        let mut text = format!("Page: {}\nURL: {}\n\n{}", page.title, page.url, page.content);
        if page.truncated {
            text.push_str("\n\nNote: This page was long and the content above is truncated.");
        }

        Ok(vec![Content::text(text)])
    }

    fn get_tools() -> Vec<Tool> {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
        .expect("static schema");

        vec![Tool::new(
            "read_browser_content".to_string(),
            "Read the visible text content of the page currently open in the Permagent browser. \
             Returns the page title, URL, and extracted text. Use this when the user asks about \
             what they're looking at, wants you to read a page, or references content in their \
             browser tab."
                .to_string(),
            schema,
        )]
    }
}

#[async_trait]
impl McpClientTrait for BrowserClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        _arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        match name {
            "read_browser_content" => match self.handle_read_browser_content().await {
                Ok(content) => Ok(CallToolResult::success(content)),
                Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {error}"
                ))])),
            },
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool: {name}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
