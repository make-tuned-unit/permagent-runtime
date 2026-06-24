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

        let mut text = format!(
            "Page: {}\nURL: {}\n\n{}",
            page.title, page.url, page.content
        );
        if page.truncated {
            text.push_str("\n\nNote: This page was long and the content above is truncated.");
        }

        Ok(vec![Content::text(text)])
    }

    /// Open a new tab in the Permagent browser pointed at `url`. Emits a
    /// `BrowserNavigate` event on the global bus; the command-center catches it
    /// (`useAppNavigate`) and calls the existing `openInBrowser` path, which
    /// focuses the Build workspace and opens a fresh tab at the URL. Works
    /// identically whether the request arrived by text or by voice — both route
    /// through this tool call.
    async fn handle_open_browser_tab(
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let url = arguments
            .as_ref()
            .and_then(|a| a.get("url"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("Missing required parameter: url")?;

        crate::events::emit(crate::events::browser_navigate(
            url,
            &format!("Opening {url} in the browser"),
        ));

        Ok(vec![Content::text(format!(
            "Opened a new browser tab and navigated to {url}."
        ))])
    }

    fn get_tools() -> Vec<Tool> {
        let empty_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }))
        .expect("static schema");

        let open_tab_schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to open (e.g. \"reckonize.org\" or \"https://example.com\"). A bare domain is fine — it is normalized to https:// by the browser."
                }
            },
            "required": ["url"]
        }))
        .expect("static schema");

        vec![
            Tool::new(
                "read_browser_content".to_string(),
                "Read the visible text content of the page currently open in the Permagent browser. \
                 Returns the page title, URL, and extracted text. Use this when the user asks about \
                 what they're looking at, wants you to read a page, or references content in their \
                 browser tab."
                    .to_string(),
                empty_schema,
            ),
            Tool::new(
                "open_browser_tab".to_string(),
                "Open a new tab in the Permagent browser and navigate it to a URL. Use this whenever \
                 the user asks you to open a website, go to a URL, pull up a page, or \"open a tab and \
                 go to X\" — from typed or spoken instructions alike. Pass the URL in `url` (a bare \
                 domain like \"reckonize.org\" is fine)."
                    .to_string(),
                open_tab_schema,
            ),
        ]
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
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "read_browser_content" => self.handle_read_browser_content().await,
            "open_browser_tab" => Self::handle_open_browser_tab(arguments).await,
            _ => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Unknown tool: {name}"
                ))]))
            }
        };
        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
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
    use crate::events::PermagentEventType;

    #[test]
    fn exposes_both_tools_with_open_tab_requiring_url() {
        let tools = BrowserClient::get_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"read_browser_content"));
        assert!(names.contains(&"open_browser_tab"));

        let open_tab = tools
            .into_iter()
            .find(|t| t.name == "open_browser_tab")
            .expect("open_browser_tab tool present");
        let required = open_tab
            .input_schema
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        assert!(required.iter().any(|v| v == "url"));
    }

    #[tokio::test]
    async fn open_browser_tab_emits_navigate_event() {
        let mut rx = crate::events::subscribe();
        let args: JsonObject =
            serde_json::from_value(serde_json::json!({ "url": "reckonize.org" })).unwrap();

        let result = BrowserClient::handle_open_browser_tab(Some(args)).await;
        assert!(result.is_ok(), "handler should succeed: {result:?}");

        // The bus is global and shared across parallel tests; drain until ours.
        let mut navigate = None;
        while let Ok(event) = rx.try_recv() {
            if event.event_type == PermagentEventType::BrowserNavigate {
                navigate = Some(event);
                break;
            }
        }
        let event = navigate.expect("a BrowserNavigate event was emitted");
        assert_eq!(
            event.payload.get("url").and_then(|v| v.as_str()),
            Some("reckonize.org")
        );
    }

    #[tokio::test]
    async fn open_browser_tab_rejects_missing_url() {
        let err = BrowserClient::handle_open_browser_tab(None)
            .await
            .unwrap_err();
        assert!(err.contains("url"), "error should mention url: {err}");
    }
}
