//! Storage Health — native filesystem scanner exposed as an agent tool.
//!
//! Provides a single tool `scan_storage_health` that runs deterministic
//! filesystem scanning and returns structured findings. The agent narrates
//! results; it does not produce them.

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

pub static EXTENSION_NAME: &str = "storage_health";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ScanStorageParams {
    /// Optional list of categories to scan. If omitted, scans all categories.
    /// Valid values: "dev_cache", "app_cache", "build_artifact", "stale_file", "large_file"
    #[serde(default)]
    categories: Option<Vec<String>>,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

pub struct StorageHealthClient {
    info: InitializeResult,
    _context: PlatformExtensionContext,
}

impl StorageHealthClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Storage Health"),
            )
            .with_instructions(
                "Scan the filesystem for storage cleanup opportunities. \
                 Use scan_storage_health when the user asks about disk space, cleanup, \
                 or storage. This is the ONLY mechanism for producing storage findings.",
            );
        Ok(Self {
            info,
            _context: context,
        })
    }

    async fn handle_scan(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let result = crate::storage_health::run_scan().await;

        // Format the findings as JSON for the agent to narrate
        let findings_json =
            serde_json::to_string_pretty(&result.findings).unwrap_or_else(|_| "[]".to_string());

        // Build a summary for the agent
        let mut summary_lines = vec![format!(
            "Storage scan complete: {} findings totaling {} across {} categories.\n",
            result.findings.len(),
            format_bytes(result.total_bytes),
            result.categories.len(),
        )];

        let mut sorted_categories: Vec<_> = result.categories.iter().collect();
        sorted_categories.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.total_bytes));

        for (cat, stats) in &sorted_categories {
            let label = category_label(cat);
            summary_lines.push(format!(
                "- {}: {} ({} items)",
                label,
                format_bytes(stats.total_bytes),
                stats.count
            ));
        }

        summary_lines.push(format!("\nrun_id: {}", result.run_id));
        summary_lines.push(format!(
            "\nFindings JSON (emit this in <findings>...</findings> tags):\n{}",
            findings_json
        ));

        Ok(CallToolResult::success(vec![Content::text(
            summary_lines.join("\n"),
        )]))
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn category_label(key: &str) -> &str {
    match key {
        "dev_cache" => "Dev Caches",
        "app_cache" => "App Caches",
        "build_artifact" => "Build Artifacts",
        "stale_file" => "Old Downloads",
        "large_file" => "Large User Files",
        _ => key,
    }
}

impl StorageHealthClient {
    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![Tool::new(
            "scan_storage_health".to_string(),
            "Scans the filesystem for storage cleanup opportunities. This is the ONLY \
             mechanism for producing storage findings. You MUST NOT attempt to run find, \
             du, ls, or any other shell command to scan storage — use this tool exclusively. \
             The tool returns deterministic findings the user can audit and act on.\n\n\
             After calling this tool:\n\
             1. Read the returned findings JSON\n\
             2. Write a brief user-facing narrative summary highlighting the top 3 \
                cleanup opportunities\n\
             3. Emit the findings JSON in <findings>{...}</findings> tags for the UI"
                .to_string(),
            schema::<ScanStorageParams>(),
        )]
    }
}

#[async_trait]
impl McpClientTrait for StorageHealthClient {
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
            "scan_storage_health" => self.handle_scan(arguments).await,
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
