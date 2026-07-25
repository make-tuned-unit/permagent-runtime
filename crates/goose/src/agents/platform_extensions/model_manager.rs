//! Model Manager — the agent's window onto the local inference models its
//! sub-agents run.
//!
//! P1 (#934): read-only `list_models`, so Henry can *see* which models are
//! installed, their quantization, and their disk footprint. Later phases add
//! `propose_model_upgrade` (→ Decision Inbox → approve → download + select),
//! mirroring the `propose_project_intel` pattern — the agent never swaps a
//! model without the user approving it.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "model_manager";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListModelsParams {}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

pub struct ModelManagerClient {
    info: InitializeResult,
    _context: PlatformExtensionContext,
}

impl ModelManagerClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Model Manager"),
            )
            .with_instructions(
                "Inspect the local inference models your sub-agents run. Use list_models when \
                 the user asks what models are installed, how much disk they use, or whether a \
                 model is available. This is the ONLY mechanism for listing installed models — \
                 do not shell out to `ls`, `du`, or the Ollama CLI. It is read-only; it never \
                 downloads or changes anything.",
            );
        Ok(Self {
            info,
            _context: context,
        })
    }

    async fn handle_list_models(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        use crate::providers::local_inference::local_model_registry::get_registry;

        let (models, total_bytes): (Vec<serde_json::Value>, u64) = {
            let reg = get_registry().lock().unwrap_or_else(|e| e.into_inner());
            let entries = reg.list_models();
            let total = entries.iter().map(|m| m.size_bytes).sum();
            let models = entries
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "repo_id": m.repo_id,
                        "filename": m.filename,
                        "quantization": m.quantization,
                        "size_bytes": m.size_bytes,
                        "source_url": m.source_url,
                        "has_vision": m.mmproj_path.is_some(),
                    })
                })
                .collect();
            (models, total)
        };

        let mut summary = vec![if models.is_empty() {
            "No local inference models are installed yet — the user can add one from \
             Settings → Models."
                .to_string()
        } else {
            format!(
                "{} local inference model(s) installed, {} on disk:",
                models.len(),
                format_bytes(total_bytes)
            )
        }];
        for m in &models {
            summary.push(format!(
                "- {} ({}, {}){}",
                m["id"].as_str().unwrap_or("?"),
                m["quantization"].as_str().unwrap_or("?"),
                format_bytes(m["size_bytes"].as_u64().unwrap_or(0)),
                if m["has_vision"].as_bool().unwrap_or(false) {
                    " — vision"
                } else {
                    ""
                },
            ));
        }
        let json = serde_json::to_string_pretty(&models).unwrap_or_else(|_| "[]".to_string());
        summary.push(format!("\nModels JSON:\n{json}"));

        Ok(CallToolResult::success(vec![Content::text(
            summary.join("\n"),
        )]))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

impl ModelManagerClient {
    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it AND `self_knowledge` includes this extension's inventory.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![Tool::new(
            "list_models".to_string(),
            "Lists the local inference models installed on this machine that your sub-agents \
             can run — id, repo, quantization, size on disk, source, and whether it has a \
             vision encoder. This is the ONLY mechanism for listing installed models; do NOT \
             shell out to `ls`, `du`, or the Ollama CLI. Use it when the user asks what models \
             are installed, how much disk they use, or whether a specific model is available. \
             Read-only — it never downloads or changes anything."
                .to_string(),
            schema::<ListModelsParams>(),
        )]
    }
}

#[async_trait]
impl McpClientTrait for ModelManagerClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, Error> {
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
    ) -> std::result::Result<CallToolResult, Error> {
        let result = match name {
            "list_models" => self.handle_list_models(arguments).await,
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
    fn exposes_list_models_tool() {
        let tools = ModelManagerClient::get_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "list_models");
    }
}
