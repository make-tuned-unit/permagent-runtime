//! The Librarian — peer agent for writing memory descriptions in Spectral.
//!
//! Uses a local LLM via Ollama to generate who/what/where/when/why prose
//! descriptions for memories stored in the Brain. Exposes two MCP tools:
//! `describe_memory` and `list_undescribed`.
//!
//! STATUS: Scaffolded. Spectral endpoints for get_memory, set_description,
//! and list_undescribed are not yet available. Tool implementations return
//! clear "not yet implemented" errors until the Spectral pin is bumped.

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
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "librarian";

// ---------------------------------------------------------------------------
// System prompt for the Librarian (sent to Qwen via Ollama)
// ---------------------------------------------------------------------------

pub const LIBRARIAN_SYSTEM_PROMPT: &str = r#"You are the Librarian, a memory archivist for an AI agent system.
Your job is to write clear, factual descriptions of memories that have been stored in the system's knowledge graph.

Each memory you describe should answer:
- WHAT happened (the event itself)
- WHO was involved (which agents, tools, or humans)
- WHEN it occurred (timestamp and the broader session context)
- WHERE in the workflow it emerged from (what came before)
- WHY it was stored (what makes this memory worth keeping)
- HOW it was created (was it user input, agent action, tool result, or automated trigger)

Write in past tense, third person, plain prose. No bullet points. No headers. No markdown. 200-400 words. Be informative and neutral — you are writing catalogue entries, not narratives. Future retrieval depends on your descriptions being precise and useful.

Do not editorialize. Do not speculate beyond what the data shows. If a field is unknown, say so plainly rather than inventing it."#;

// ---------------------------------------------------------------------------
// Ollama configuration
// ---------------------------------------------------------------------------

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const PRIMARY_MODEL: &str = "qwen2.5:7b-instruct-q4_K_M";
const FALLBACK_MODEL: &str = "llama3.1:8b-instruct-q4_K_M";
const OLLAMA_TEMPERATURE: f32 = 0.3;
const OLLAMA_TOP_P: f32 = 0.9;
const OLLAMA_MAX_TOKENS: u32 = 600;

// ---------------------------------------------------------------------------
// Tool parameter schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct DescribeMemoryParams {
    /// The Spectral memory ID to describe
    memory_id: String,
    /// If true, regenerate even if a description already exists. Defaults to false.
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ListUndescribedParams {
    /// Maximum number of undescribed memories to return (default 20, max 100)
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

// ---------------------------------------------------------------------------
// Librarian client
// ---------------------------------------------------------------------------

pub struct LibrarianClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
}

impl LibrarianClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(
            Implementation::new(EXTENSION_NAME, "0.1.0")
                .with_title("Librarian"),
        )
        .with_instructions(
            "The Librarian generates prose descriptions for memories stored in the Brain. \
             Use describe_memory to create a who/what/where/when/why description for a \
             specific memory. Use list_undescribed to find memories that need descriptions.",
        );

        // TODO: On startup, verify Ollama is reachable and the primary model is pulled.
        // If not pulled, attempt to pull (can take minutes on first run).
        // If both primary and fallback fail, start in degraded mode.
        tracing::info!(
            "Librarian extension loaded. Ollama at {}, primary model: {}",
            OLLAMA_BASE_URL,
            PRIMARY_MODEL
        );

        Ok(Self { info, context })
    }

    async fn handle_describe_memory(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: DescribeMemoryParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid parameters: {}", e))?;

        // TODO: Once Spectral exposes get_memory(id) and set_description(id, text):
        //
        // 1. let brain = self.context.brain.as_ref()
        //        .ok_or("Brain not available")?;
        //
        // 2. let memory = tokio::task::spawn_blocking({
        //        let brain = brain.clone();
        //        let id = params.memory_id.clone();
        //        move || brain.get_memory(&id)
        //    }).await.map_err(|e| e.to_string())?
        //      .map_err(|e| e.to_string())?;
        //
        // 3. If memory has description && !params.force, return early with cached=true
        //
        // 4. Build prompt from memory fields (event type, actors, timestamp, context)
        //
        // 5. Call Ollama:
        //    POST http://localhost:11434/api/generate
        //    { model, prompt, system: LIBRARIAN_SYSTEM_PROMPT,
        //      options: { temperature: 0.3, top_p: 0.9, num_predict: 600 } }
        //
        // 6. Validate response is coherent prose (not empty, not refusal)
        //
        // 7. Write back: brain.set_description(&params.memory_id, &description)
        //
        // 8. Return description + metadata (model, latency_ms, cached: false)

        Err(format!(
            "describe_memory is scaffolded but not yet functional. \
             Waiting for Spectral endpoints: get_memory('{}'), set_description(). \
             See crates/goose/src/agents/platform_extensions/librarian.rs for the \
             implementation plan.",
            params.memory_id
        ))
    }

    async fn handle_list_undescribed(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let limit = if let Some(args) = arguments {
            let params: ListUndescribedParams =
                serde_json::from_value(serde_json::Value::Object(args))
                    .map_err(|e| format!("Invalid parameters: {}", e))?;
            params.limit.min(100)
        } else {
            default_limit()
        };

        // TODO: Once Spectral exposes list_undescribed(limit):
        //
        // 1. let brain = self.context.brain.as_ref()
        //        .ok_or("Brain not available")?;
        //
        // 2. let memories = tokio::task::spawn_blocking({
        //        let brain = brain.clone();
        //        move || brain.list_undescribed(limit)
        //    }).await.map_err(|e| e.to_string())?
        //      .map_err(|e| e.to_string())?;
        //
        // 3. Return { memories: [...], total_undescribed: N }

        let _ = limit; // suppress unused warning
        Err(
            "list_undescribed is scaffolded but not yet functional. \
             Waiting for Spectral endpoint: list_undescribed(). \
             See crates/goose/src/agents/platform_extensions/librarian.rs for the \
             implementation plan."
                .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// MCP trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl McpClientTrait for LibrarianClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        let tools = vec![
            Tool::new(
                "describe_memory".to_string(),
                "Generate a prose description for a Spectral memory. Fetches the memory by ID, \
                 sends its structured data to a local LLM (Ollama), and writes the resulting \
                 who/what/where/when/why description back to Spectral. Returns cached description \
                 if one already exists (unless force=true)."
                    .to_string(),
                schema::<DescribeMemoryParams>(),
            ),
            Tool::new(
                "list_undescribed".to_string(),
                "List memories in the Brain that don't yet have a prose description, ordered by \
                 recency (newest first). Use this to find memories that need the Librarian's \
                 attention."
                    .to_string(),
                schema::<ListUndescribedParams>(),
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
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "describe_memory" => self.handle_describe_memory(arguments).await,
            "list_undescribed" => self.handle_list_undescribed(arguments).await,
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn schema<T: JsonSchema>() -> JsonObject {
    let mut obj = serde_json::to_value(schema_for!(T))
        .map(|v| v.as_object().unwrap().clone())
        .expect("valid schema");
    obj.entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    obj
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionManager;

    fn test_context() -> PlatformExtensionContext {
        PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(SessionManager::new(std::env::temp_dir())),
            session: None,
        }
    }

    #[tokio::test]
    async fn test_librarian_creates_successfully() {
        let client = LibrarianClient::new(test_context());
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_list_tools_returns_two_tools() {
        let client = LibrarianClient::new(test_context()).unwrap();
        let result = client
            .list_tools("test", None, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.tools.len(), 2);
        let names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"describe_memory".to_string()));
        assert!(names.contains(&"list_undescribed".to_string()));
    }

    #[tokio::test]
    async fn test_describe_memory_returns_not_implemented() {
        let client = LibrarianClient::new(test_context()).unwrap();
        let args: JsonObject =
            serde_json::from_str(r#"{"memory_id": "test-123", "force": false}"#).unwrap();
        let result = client
            .call_tool(
                &ToolCallContext {
                    session_id: "test".to_string(),
                    working_dir: None,
                tool_call_request_id: None,
                },
                "describe_memory",
                Some(args),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        // Should return an error result (scaffolded, not functional)
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_list_undescribed_returns_not_implemented() {
        let client = LibrarianClient::new(test_context()).unwrap();
        let result = client
            .call_tool(
                &ToolCallContext {
                    session_id: "test".to_string(),
                    working_dir: None,
                tool_call_request_id: None,
                },
                "list_undescribed",
                None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[test]
    fn test_describe_memory_schema_is_valid() {
        let s = schema::<DescribeMemoryParams>();
        assert!(s.contains_key("properties"));
    }

    #[test]
    fn test_list_undescribed_schema_is_valid() {
        let s = schema::<ListUndescribedParams>();
        assert!(s.contains_key("properties"));
    }
}
