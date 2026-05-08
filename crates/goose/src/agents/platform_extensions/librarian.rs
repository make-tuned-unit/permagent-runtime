//! The Librarian — peer agent for writing memory descriptions in Spectral.
//!
//! Uses a local LLM via Ollama to generate who/what/where/when/why prose
//! descriptions for memories stored in the Brain. Exposes two MCP tools:
//! `describe_memory` and `list_undescribed`.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::platform_extensions::get_global_brain;
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
const PRIMARY_MODEL: &str = "qwen2.5:3b";
const OLLAMA_TEMPERATURE: f32 = 0.3;
const OLLAMA_TOP_P: f32 = 0.9;
const OLLAMA_MAX_TOKENS: u32 = 600;
const RELATED_MEMORY_LIMIT: usize = 5;

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
// Ollama response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    total_duration: u64,
    #[serde(default)]
    eval_count: u64,
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
            Implementation::new(EXTENSION_NAME, "1.0.0")
                .with_title("Librarian"),
        )
        .with_instructions(
            "The Librarian generates prose descriptions for memories stored in the Brain. \
             Use describe_memory to create a who/what/where/when/why description for a \
             specific memory. Use list_undescribed to find memories that need descriptions.",
        );

        tracing::info!(
            "Librarian extension loaded. Ollama at {}, model: {}",
            OLLAMA_BASE_URL,
            PRIMARY_MODEL
        );

        Ok(Self { info, context })
    }

    fn get_brain(&self) -> Result<Arc<spectral::Brain>, String> {
        get_global_brain().ok_or_else(|| "Brain not available — Spectral may not be initialized".to_string())
    }

    async fn handle_describe_memory(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: DescribeMemoryParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid parameters: {}", e))?;

        let brain = self.get_brain()?;
        let start = std::time::Instant::now();

        // 1. Fetch memory by ID
        let memory_id = params.memory_id.clone();
        let brain_ref = brain.clone();
        let memory = tokio::task::spawn_blocking(move || brain_ref.get_memory(&memory_id))
            .await
            .map_err(|e| format!("spawn_blocking failed: {}", e))?
            .map_err(|e| format!("Brain error: {}", e))?
            .ok_or_else(|| format!("Memory '{}' not found", params.memory_id))?;

        // 2. Check for existing description (idempotency)
        if !params.force {
            if let Some(ref desc) = memory.description {
                let result = serde_json::json!({
                    "description": desc,
                    "cached": true,
                    "model": PRIMARY_MODEL,
                    "latency_ms": 0,
                });
                return Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap(),
                )]));
            }
        }

        // 3. Fetch related memories for context via recall
        let content_for_recall = memory.content.clone();
        let brain_ref = brain.clone();
        let related_hits = tokio::task::spawn_blocking(move || {
            brain_ref.recall(&content_for_recall, spectral::Visibility::Private)
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
        .map(|r| r.memory_hits)
        .unwrap_or_default();

        // 4. Build the prompt
        let prompt = build_description_prompt(&memory, &related_hits);

        // 5. Call Ollama
        let ollama_response = call_ollama(&prompt).await?;

        // 6. Validate the response
        let description = ollama_response.response.trim().to_string();
        if description.is_empty() || description.len() < 50 {
            return Err("Ollama returned an empty or too-short response".to_string());
        }

        // 7. Write description back to Spectral
        let desc_clone = description.clone();
        let mid = params.memory_id.clone();
        let brain_ref = brain.clone();
        tokio::task::spawn_blocking(move || brain_ref.set_description(&mid, &desc_clone))
            .await
            .map_err(|e| format!("spawn_blocking failed: {}", e))?
            .map_err(|e| format!("Failed to write description: {}", e))?;

        let latency_ms = start.elapsed().as_millis();
        tracing::info!(
            memory_id = %params.memory_id,
            latency_ms = latency_ms,
            tokens = ollama_response.eval_count,
            "Librarian described memory"
        );

        // 8. Return result
        let result = serde_json::json!({
            "description": description,
            "cached": false,
            "model": PRIMARY_MODEL,
            "latency_ms": latency_ms,
            "tokens": ollama_response.eval_count,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
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

        let brain = self.get_brain()?;
        let memories = tokio::task::spawn_blocking(move || brain.list_undescribed(limit))
            .await
            .map_err(|e| format!("spawn_blocking failed: {}", e))?
            .map_err(|e| format!("Brain error: {}", e))?;

        let total = memories.len();
        let items: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|m| {
                let preview = if m.content.len() > 200 {
                    format!("{}...", &m.content[..200])
                } else {
                    m.content.clone()
                };
                serde_json::json!({
                    "id": m.id,
                    "created_at": m.created_at,
                    "wing": m.wing,
                    "source": m.source,
                    "preview": preview,
                })
            })
            .collect();

        let result = serde_json::json!({
            "memories": items,
            "total_undescribed": total,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap(),
        )]))
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
// Prompt building
// ---------------------------------------------------------------------------

fn build_description_prompt(
    memory: &spectral::ingest::Memory,
    related_hits: &[spectral::ingest::MemoryHit],
) -> String {
    let mut prompt = String::with_capacity(2000);
    prompt.push_str("Describe the following memory from the knowledge graph.\n\n");

    prompt.push_str("=== MEMORY ===\n");
    prompt.push_str(&format!("ID: {}\n", memory.id));
    prompt.push_str(&format!("Key: {}\n", memory.key));
    if let Some(ref wing) = memory.wing {
        prompt.push_str(&format!("Wing: {}\n", wing));
    }
    if let Some(ref source) = memory.source {
        prompt.push_str(&format!("Source: {}\n", source));
    }
    if let Some(ref created) = memory.created_at {
        prompt.push_str(&format!("Created: {}\n", created));
    }
    if let Some(ref tier) = memory.compaction_tier {
        prompt.push_str(&format!("Compaction tier: {:?}\n", tier));
    }
    prompt.push_str(&format!("\nContent:\n{}\n", memory.content));

    // Add related memories for context (up to RELATED_MEMORY_LIMIT)
    let hits: Vec<_> = related_hits.iter().take(RELATED_MEMORY_LIMIT).collect();
    if !hits.is_empty() {
        prompt.push_str("\n=== RELATED MEMORIES (for context) ===\n");
        for (i, hit) in hits.iter().enumerate() {
            let preview = if hit.content.len() > 300 {
                format!("{}...", &hit.content[..300])
            } else {
                hit.content.clone()
            };
            prompt.push_str(&format!(
                "\n--- Related {} (score: {:.2}) ---\nKey: {}\n{}\n",
                i + 1,
                hit.signal_score,
                hit.key,
                preview,
            ));
        }
    }

    prompt.push_str("\n=== INSTRUCTIONS ===\n");
    prompt.push_str("Write a 200-400 word description of this memory. Past tense, third person, plain prose.\n");
    prompt
}

// ---------------------------------------------------------------------------
// Ollama integration
// ---------------------------------------------------------------------------

async fn call_ollama(prompt: &str) -> Result<OllamaGenerateResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let body = serde_json::json!({
        "model": PRIMARY_MODEL,
        "prompt": prompt,
        "system": LIBRARIAN_SYSTEM_PROMPT,
        "stream": false,
        "options": {
            "temperature": OLLAMA_TEMPERATURE,
            "top_p": OLLAMA_TOP_P,
            "num_predict": OLLAMA_MAX_TOKENS,
        }
    });

    let resp = client
        .post(format!("{}/api/generate", OLLAMA_BASE_URL))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama unreachable: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama error ({}): {}", status, text));
    }

    resp.json::<OllamaGenerateResponse>()
        .await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))
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
    async fn test_describe_memory_without_brain_returns_error() {
        // No global brain set → should return a clear error
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
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_list_undescribed_without_brain_returns_error() {
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

    #[test]
    fn test_prompt_building() {
        let memory = spectral::ingest::Memory {
            id: "mem-001".to_string(),
            key: "session:2026-05-08:chat".to_string(),
            content: "User asked about Rust async patterns.".to_string(),
            wing: Some("engineering".to_string()),
            hall: None,
            signal_score: 0.0,
            visibility: "private".to_string(),
            source: Some("permagent.chat".to_string()),
            device_id: None,
            confidence: 1.0,
            created_at: Some("2026-05-08T10:00:00Z".to_string()),
            last_reinforced_at: None,
            episode_id: None,
            compaction_tier: None,
            declarative_density: None,
            description: None,
            description_generated_at: None,
        };
        let no_related: Vec<spectral::ingest::MemoryHit> = vec![];
        let prompt = build_description_prompt(&memory, &no_related);
        assert!(prompt.contains("mem-001"));
        assert!(prompt.contains("Rust async patterns"));
        assert!(prompt.contains("engineering"));
    }
}
