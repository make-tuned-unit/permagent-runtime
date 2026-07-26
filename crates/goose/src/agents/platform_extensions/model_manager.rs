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
use sqlx::{Pool, Sqlite};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "model_manager";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListModelsParams {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposeModelUpgradeParams {
    /// The model id to switch the active inference model to. It must already be
    /// installed (see list_models) — download stays in Settings → Models.
    model_id: String,
    /// Why the target is better than the current model (more compact, faster,
    /// more accurate…). Shown on the Decision Inbox card.
    why_better: String,
}

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
    context: PlatformExtensionContext,
}

impl ModelManagerClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Model Manager"),
            )
            .with_instructions(
                "Inspect and steward the local inference models your sub-agents run. Use \
                 list_models to see what is installed (the ONLY mechanism — do not shell out to \
                 `ls`, `du`, or the Ollama CLI). When you find a better installed model, use \
                 propose_model_upgrade to propose switching the active model — it is review-gated \
                 through the Decision Inbox and NOTHING changes until the user approves.",
            );
        Ok(Self { info, context })
    }

    async fn pool(&self) -> std::result::Result<Pool<Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())
    }

    async fn handle_propose_model_upgrade(
        &self,
        arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let model_id = args
            .get("model_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: model_id")?
            .trim()
            .to_string();
        let why_better = args
            .get("why_better")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: why_better")?
            .trim()
            .to_string();
        if model_id.is_empty() {
            return Err("model_id must not be empty".to_string());
        }
        if why_better.is_empty() {
            return Err("why_better must not be empty".to_string());
        }

        let current_model = crate::config::Config::global()
            .get_param::<String>("GOOSE_MODEL")
            .ok();
        let detail = match &current_model {
            Some(c) => format!(
                "Proposed switching the active inference model from \"{c}\" to \"{model_id}\".\n\nWhy: {why_better}"
            ),
            None => format!(
                "Proposed switching the active inference model to \"{model_id}\".\n\nWhy: {why_better}"
            ),
        };
        let payload = crate::decisions::ModelUpgradePayload {
            model_id: model_id.clone(),
            why_better,
            current_model,
        };

        let pool = self.pool().await?;
        let decision = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "model_upgrade".to_string(),
                headline: Some("Switch active inference model".to_string()),
                detail: Some(detail),
                payload: serde_json::to_value(&payload).map_err(|e| e.to_string())?,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;

        if decision.kind == "malformed" {
            return Err(format!(
                "The proposal was rejected as malformed: {}",
                decision.detail
            ));
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Proposed switching the active model to \"{model_id}\" — decision {} is waiting in the \
             Decision Inbox. NOTHING changes until the user approves it there; on approval the \
             active model switches (the model must already be installed).",
            decision.id
        ))]))
    }

    #[cfg(feature = "local-inference")]
    async fn handle_list_models(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        use crate::providers::local_inference::local_model_registry::get_registry;

        let (models, total_bytes): (Vec<serde_json::Value>, u64) = {
            let reg = get_registry().lock().unwrap_or_else(|e| e.into_inner());
            let entries = reg.list_models();
            let total: u64 = entries.iter().map(|m| m.size_bytes).sum();
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

    /// When the `local-inference` feature is not compiled in, there is no local
    /// model registry to read — report that plainly rather than failing.
    #[cfg(not(feature = "local-inference"))]
    async fn handle_list_models(
        &self,
        _arguments: Option<JsonObject>,
    ) -> std::result::Result<CallToolResult, String> {
        Ok(CallToolResult::success(vec![Content::text(
            "Local inference is not compiled into this build, so there are no locally-run \
             inference models to list. Cloud provider models are configured in Settings."
                .to_string(),
        )]))
    }
}

#[cfg(feature = "local-inference")]
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
        vec![
            Tool::new(
                "list_models".to_string(),
                "Lists the local inference models installed on this machine that your sub-agents \
             can run — id, repo, quantization, size on disk, source, and whether it has a \
             vision encoder. This is the ONLY mechanism for listing installed models; do NOT \
             shell out to `ls`, `du`, or the Ollama CLI. Use it when the user asks what models \
             are installed, how much disk they use, or whether a specific model is available. \
             Read-only — it never downloads or changes anything."
                    .to_string(),
                schema::<ListModelsParams>(),
            ),
            Tool::new(
                "propose_model_upgrade".to_string(),
                "Propose switching the active inference model to a different, already-installed \
             model (see list_models). The proposal lands in the Decision Inbox for the user to \
             approve — NOTHING changes until they approve; you never switch models yourself. Use \
             it when you identify a better (more compact, faster, more accurate) installed model. \
             Provide model_id (the target, must already be installed) and why_better (the \
             rationale shown on the card). Download of new models stays in Settings → Models."
                    .to_string(),
                schema::<ProposeModelUpgradeParams>(),
            ),
        ]
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
            "propose_model_upgrade" => self.handle_propose_model_upgrade(arguments).await,
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
    fn exposes_expected_tools() {
        let names: Vec<_> = ModelManagerClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, vec!["list_models", "propose_model_upgrade"]);
    }
}
