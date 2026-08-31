//! Public data sources the user enabled under Settings → Data sources.
//!
//! Tools are live as soon as a source is toggled on. The Orchestrator may
//! call every enabled source; suggested specialist agents are named on each
//! catalog row so they pick the source up without a restart.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use crate::public_apis;
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "public_apis";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListParams {
    /// Optional category, e.g. `Finance`. Empty lists enabled sources.
    #[serde(default)]
    category: Option<String>,
    /// When true, list the full catalog for that category (still off until enabled).
    #[serde(default)]
    catalog: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CallParams {
    /// Catalog slug, e.g. `alpha-vantage`.
    slug: String,
    /// Optional HTTPS URL on the same host as the catalog listing.
    #[serde(default)]
    url: Option<String>,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

pub struct PublicApisClient {
    info: InitializeResult,
}

impl PublicApisClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Data sources"),
            )
            .with_instructions(
                "Public data sources the user enabled under Settings → Data sources.\n\n\
                 public_api_list shows enabled sources (and, with catalog=true, a category \
                 to browse). public_api_call GETs an enabled source. Enabling a source \
                 makes it callable on the next turn — the live enabled list is injected \
                 into this agent's prompt, not snapshotted at startup. The Orchestrator \
                 may call every enabled source. Suggested specialist agents receive the \
                 sources in their category automatically."
                    .to_string(),
            );
        Ok(Self { info })
    }

    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "public_api_list".to_string(),
                "List public data sources. Default: the sources the user has enabled \
                 under Settings → Data sources, with suggested agents. Pass a category \
                 (Finance, Weather, …) and catalog=true to browse that category a few \
                 at a time. A source the user just turned on is in this list immediately. \
                 The Orchestrator can call any enabled source via public_api_call."
                    .to_string(),
                schema::<ListParams>(),
            ),
            Tool::new(
                "public_api_call".to_string(),
                "GET an enabled public data source by slug. Optional url must be https \
                 on the same host as the catalog listing. Only sources the user turned \
                 on under Settings → Data sources work. The Orchestrator may call any \
                 enabled source; suggested specialist agents receive the ones in their \
                 category as soon as they are toggled on. OAuth sources refuse. A missing \
                 apiKey is an answer, not a crash."
                    .to_string(),
                schema::<CallParams>(),
            ),
        ]
    }
}

#[async_trait]
impl McpClientTrait for PublicApisClient {
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
            "public_api_list" => list_sources(arguments),
            "public_api_call" => call_source(arguments).await,
            other => Err(format!("Unknown tool: {other}")),
        };
        match result {
            Ok(result) => Ok(result),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

/// Keep `public_apis` on a worker that should receive enabled sources, as long
/// as the parent session already had it. Restoring from the parent is not a
/// grant-widen: the run already had the extension, and a Settings toggle mid-
/// session must still flow to the suggested agent.
pub fn ensure_extension_for_agent(
    parent: &[crate::agents::ExtensionConfig],
    mut narrowed: Vec<crate::agents::ExtensionConfig>,
    worker_key: &str,
) -> Vec<crate::agents::ExtensionConfig> {
    if narrowed.iter().any(|c| c.key() == EXTENSION_NAME) {
        return narrowed;
    }
    if !public_apis::agent_is_data_source_consumer(worker_key) {
        return narrowed;
    }
    if let Some(ext) = parent.iter().find(|c| c.key() == EXTENSION_NAME) {
        narrowed.push(ext.clone());
    }
    narrowed
}

fn list_sources(arguments: Option<JsonObject>) -> std::result::Result<CallToolResult, String> {
    let p: ListParams = serde_json::from_value(serde_json::Value::Object(
        arguments.unwrap_or_default(),
    ))
    .unwrap_or(ListParams {
        category: None,
        catalog: None,
    });
    let catalog_mode = p.catalog.unwrap_or(false);
    let category = p
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if catalog_mode {
        if let Some(cat) = category {
            let rows: Vec<_> = public_apis::catalog()
                .iter()
                .filter(|e| e.category.eq_ignore_ascii_case(cat))
                .cloned()
                .collect();
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&serde_json::json!({
                    "category": cat,
                    "suggestedAgents": public_apis::suggested_agents_for(cat),
                    "sources": rows,
                }))
                .unwrap_or_else(|_| "[]".into()),
            )]));
        }
        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&public_apis::categories())
                .unwrap_or_else(|_| "[]".into()),
        )]));
    }
    let mut enabled = public_apis::enabled_entries();
    if let Some(cat) = category {
        enabled.retain(|e| e.category.eq_ignore_ascii_case(cat));
    }
    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&serde_json::json!({
            "enabled": enabled,
            "note": public_apis::instructions_for_enabled(),
        }))
        .unwrap_or_else(|_| "{}".into()),
    )]))
}

async fn call_source(arguments: Option<JsonObject>) -> std::result::Result<CallToolResult, String> {
    let p: CallParams =
        serde_json::from_value(serde_json::Value::Object(arguments.unwrap_or_default()))
            .map_err(|e| e.to_string())?;
    match public_apis::call(&p.slug, p.url.as_deref()).await {
        Ok(body) => Ok(CallToolResult::success(vec![Content::text(body)])),
        Err(e) => Ok(CallToolResult::success(vec![Content::text(e)])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_named() {
        let names: Vec<_> = PublicApisClient::get_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(names.contains(&"public_api_list".into()));
        assert!(names.contains(&"public_api_call".into()));
        for tool in PublicApisClient::get_tools() {
            assert!(tool.description.as_ref().is_some_and(|d| d.len() > 40));
        }
    }

    fn platform(name: &str) -> crate::agents::ExtensionConfig {
        crate::agents::ExtensionConfig::Platform {
            name: name.into(),
            description: String::new(),
            display_name: None,
            bundled: Some(true),
            available_tools: vec![],
        }
    }

    #[test]
    fn ensure_extension_restores_public_apis_for_suggested_agents() {
        let parent = vec![platform("developer"), platform("public_apis")];
        let narrowed = vec![platform("developer")];
        let restored = ensure_extension_for_agent(&parent, narrowed.clone(), "financier");
        assert!(restored.iter().any(|c| c.key() == EXTENSION_NAME));
        let left_alone = ensure_extension_for_agent(&parent, narrowed, "cursor");
        assert!(!left_alone.iter().any(|c| c.key() == EXTENSION_NAME));
    }

    #[test]
    fn ensure_extension_does_not_invent_public_apis() {
        let parent = vec![platform("developer")];
        let narrowed = vec![platform("developer")];
        let got = ensure_extension_for_agent(&parent, narrowed, "financier");
        assert!(!got.iter().any(|c| c.key() == EXTENSION_NAME));
    }
}
