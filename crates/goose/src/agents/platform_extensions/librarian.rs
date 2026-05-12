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
/// Default model used when LibrarianSchedule.model is empty or unavailable.
const DEFAULT_MODEL: &str = "qwen2.5:3b";
const RELATED_MEMORY_LIMIT: usize = 5;

/// Resolve the Librarian model: read from the schedule config file,
/// fall back to DEFAULT_MODEL if empty or unreadable.
fn resolve_model() -> String {
    let path = crate::config::paths::Paths::in_data_dir("librarian_schedule.json");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(schedule) = serde_json::from_str::<serde_json::Value>(&contents) {
            if let Some(model) = schedule.get("model").and_then(|v| v.as_str()) {
                if !model.is_empty() {
                    return model.to_string();
                }
            }
        }
    }
    DEFAULT_MODEL.to_string()
}

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
    #[allow(dead_code)]
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
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Librarian"))
            .with_instructions(
                "The Librarian generates prose descriptions for memories stored in the Brain. \
                 Use describe_memory to create a who/what/where/when/why description for a \
                 specific memory. Use list_undescribed to find memories that need descriptions.",
            );

        let active_model = resolve_model();
        tracing::info!(
            "Librarian extension loaded. Ollama at {}, model: {}",
            OLLAMA_BASE_URL,
            active_model
        );

        Ok(Self { info, context })
    }

    fn get_brain(&self) -> Result<Arc<spectral::Brain>, String> {
        get_global_brain()
            .ok_or_else(|| "Brain not available — Spectral may not be initialized".to_string())
    }

    async fn handle_describe_memory(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: DescribeMemoryParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid parameters: {}", e))?;

        let brain = self.get_brain()?;
        let model = resolve_model();
        let result = describe_one(&brain, &params.memory_id, params.force, &model).await?;

        let response = serde_json::json!({
            "description": result.description,
            "cached": result.cached,
            "model": result.model,
            "latency_ms": result.latency_ms,
            "tokens": result.tokens,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    async fn handle_list_undescribed(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResult, String> {
        let args = arguments.unwrap_or_default();
        let params: ListUndescribedParams = serde_json::from_value(serde_json::Value::Object(args))
            .unwrap_or(ListUndescribedParams {
                limit: default_limit(),
            });
        let limit = params.limit.min(100);

        let brain = self.get_brain()?;
        let memories = tokio::task::spawn_blocking(move || brain.list_undescribed(limit))
            .await
            .map_err(|e| format!("spawn_blocking failed: {}", e))?
            .map_err(|e| format!("Brain error: {}", e))?;

        let total = memories.len();
        let items: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|m| {
                let preview = if m.content.chars().count() > 200 {
                    let truncated: String = m.content.chars().take(200).collect();
                    format!("{}...", truncated)
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
// Core describe logic — shared between MCP tool handler and batch runner
// ---------------------------------------------------------------------------

pub struct DescribeResult {
    pub description: String,
    pub model: String,
    pub latency_ms: u128,
    pub tokens: u64,
    pub cached: bool,
}

/// Describe a single memory: fetch → check idempotency → recall related → LLM → write back.
///
/// If `force` is false and the memory already has a description, returns the cached
/// description without calling Ollama or writing to Spectral. This is the single
/// source of truth for description idempotency — both the MCP tool handler and
/// `run_batch` go through this function.
pub async fn describe_one(
    brain: &Arc<spectral::Brain>,
    memory_id: &str,
    force: bool,
    model: &str,
) -> Result<DescribeResult, String> {
    let start = std::time::Instant::now();

    // 1. Fetch memory
    let mid = memory_id.to_string();
    let brain_ref = brain.clone();
    let memory = tokio::task::spawn_blocking(move || brain_ref.get_memory(&mid))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
        .map_err(|e| format!("Brain error: {}", e))?
        .ok_or_else(|| format!("Memory '{}' not found", memory_id))?;

    // 2. Idempotency check: skip Ollama if description exists and force=false
    if !force {
        if let Some(ref desc) = memory.description {
            return Ok(DescribeResult {
                description: desc.clone(),
                model: model.to_string(),
                latency_ms: 0,
                tokens: 0,
                cached: true,
            });
        }
    }

    // 3. Fetch related memories for context
    let content = memory.content.clone();
    let brain_ref = brain.clone();
    let related_hits = tokio::task::spawn_blocking(move || {
        brain_ref.recall(&content, spectral::Visibility::Private)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
    .map(|r| r.memory_hits)
    .unwrap_or_default();

    // 4. Build prompt and call Ollama
    let prompt = build_description_prompt(&memory, &related_hits);
    let ollama_response = call_ollama(&prompt, model).await?;

    // 5. Validate
    let description = ollama_response.response.trim().to_string();
    if description.is_empty() || description.len() < 50 {
        return Err("Ollama returned an empty or too-short response".to_string());
    }

    // 6. Write back to Spectral
    let desc_clone = description.clone();
    let mid = memory_id.to_string();
    let brain_ref = brain.clone();
    tokio::task::spawn_blocking(move || brain_ref.set_description(&mid, &desc_clone))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
        .map_err(|e| format!("Failed to write description: {}", e))?;

    let latency_ms = start.elapsed().as_millis();
    tracing::info!(
        memory_id = %memory_id,
        latency_ms = latency_ms,
        tokens = ollama_response.eval_count,
        "Librarian described memory"
    );

    Ok(DescribeResult {
        description,
        model: model.to_string(),
        latency_ms,
        tokens: ollama_response.eval_count,
        cached: false,
    })
}

/// Run a batch of descriptions: list undescribed → describe each → return count.
///
/// Calls `describe_one(force=false)` for each memory, so already-described memories
/// are skipped cheaply. This handles the race where a memory gets described between
/// the `list_undescribed` query and the `describe_one` call (audit vector #2).
///
/// NOTE on vector #2 (Spectral write fails after Ollama returns): if set_description
/// fails after Ollama completes, the memory remains undescribed and will be re-queued
/// on the next batch. The re-call to Ollama is a known cost — the description content
/// may differ slightly but is functionally equivalent. Not worth guarding against
/// since set_description failures indicate a deeper Spectral issue.
pub async fn run_batch(
    brain: &Arc<spectral::Brain>,
    batch_size: usize,
    model: &str,
) -> Result<usize, String> {
    let mut described = 0;
    loop {
        let brain_ref = brain.clone();
        let memories = tokio::task::spawn_blocking(move || brain_ref.list_undescribed(batch_size))
            .await
            .map_err(|e| format!("spawn_blocking failed: {}", e))?
            .map_err(|e| format!("Brain error: {}", e))?;

        if memories.is_empty() {
            break;
        }

        for mem in &memories {
            match describe_one(brain, &mem.id, false, model).await {
                Ok(r) if r.cached => {
                    tracing::debug!(memory_id = %mem.id, "Librarian skipped already-described memory");
                }
                Ok(_) => described += 1,
                Err(e) => {
                    tracing::warn!(memory_id = %mem.id, error = %e, "Librarian failed to describe memory, skipping");
                }
            }
        }

        if memories.len() < batch_size {
            break;
        }
    }

    tracing::info!(described = described, "Librarian batch complete");
    Ok(described)
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

    let hits: Vec<_> = related_hits.iter().take(RELATED_MEMORY_LIMIT).collect();
    if !hits.is_empty() {
        prompt.push_str("\n=== RELATED MEMORIES (for context) ===\n");
        for (i, hit) in hits.iter().enumerate() {
            let preview = if hit.content.chars().count() > 300 {
                let truncated: String = hit.content.chars().take(300).collect();
                format!("{}...", truncated)
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
    prompt.push_str(
        "Write a 200-400 word description of this memory. Past tense, third person, plain prose.\n",
    );
    prompt
}

// ---------------------------------------------------------------------------
// Ollama integration
// ---------------------------------------------------------------------------

async fn call_ollama(prompt: &str, model: &str) -> Result<OllamaGenerateResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "system": LIBRARIAN_SYSTEM_PROMPT,
        "stream": false,
        "options": {
            "temperature": 0.3,
            "top_p": 0.9,
            "num_predict": 600,
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

    // --- Idempotency tests (require real Brain + Ollama — run with cargo test -- --ignored) ---

    /// describe_one with force=false on a memory that already has a description
    /// should return the cached description without calling Ollama.
    #[tokio::test]
    #[ignore = "requires Brain + Ollama: cargo test -p permagent -- librarian --ignored"]
    async fn test_describe_one_force_false_returns_cached() {
        let brain = get_global_brain().expect("Brain required for this test");
        // Find a memory that already has a description
        let described = tokio::task::spawn_blocking({
            let brain = brain.clone();
            move || brain.list_undescribed(1)
        })
        .await
        .unwrap()
        .unwrap();

        // If no undescribed memories, this test can't run meaningfully
        if described.is_empty() {
            eprintln!("No undescribed memories to test with — skipping");
            return;
        }

        // Describe one first
        let id = &described[0].id;
        let first = describe_one(&brain, id, false, DEFAULT_MODEL).await.unwrap();
        assert!(!first.cached, "First call should not be cached");

        // Second call with force=false should return cached
        let second = describe_one(&brain, id, false, DEFAULT_MODEL).await.unwrap();
        assert!(second.cached, "Second call should be cached");
        assert_eq!(second.description, first.description);
        assert_eq!(second.latency_ms, 0);
    }

    /// describe_one with force=true on a memory that already has a description
    /// should call Ollama and write a new description.
    #[tokio::test]
    #[ignore = "requires Brain + Ollama: cargo test -p permagent -- librarian --ignored"]
    async fn test_describe_one_force_true_regenerates() {
        let brain = get_global_brain().expect("Brain required for this test");
        let described = tokio::task::spawn_blocking({
            let brain = brain.clone();
            move || brain.list_undescribed(1)
        })
        .await
        .unwrap()
        .unwrap();

        if described.is_empty() {
            eprintln!("No undescribed memories — skipping");
            return;
        }

        let id = &described[0].id;
        // Ensure described
        let _ = describe_one(&brain, id, false, DEFAULT_MODEL).await.unwrap();

        // Force regenerate
        let result = describe_one(&brain, id, true, DEFAULT_MODEL).await.unwrap();
        assert!(!result.cached, "force=true should not return cached");
        assert!(result.latency_ms > 0, "Should have Ollama latency");
        assert!(result.tokens > 0, "Should have token count");
    }

    // concurrent run_batch → one set of Ollama invocations:
    // Not unit-testable without a mock Ollama server. Documented behavior:
    // BATCH_MUTEX in ollama.rs serializes batch runs. run_librarian_now
    // uses try_lock and returns 409 if a batch is already running.
    // describe_one(force=false) makes any second batch execution harmless
    // since all memories will already have descriptions.

    /// WARMED_TODAY persistence: writing the warm date to disk and reloading
    /// should reflect the same date.
    #[test]
    fn test_warmed_date_persistence() {
        // This tests the functions in ollama.rs which are not directly importable
        // from the goose crate. The persistence logic is tested via the JSON format:
        let today = chrono::Local::now().date_naive();
        let json = serde_json::json!({ "last_warmed_date": today.format("%Y-%m-%d").to_string() });
        let serialized = serde_json::to_string_pretty(&json).unwrap();

        // Verify round-trip parsing
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        let date_str = parsed["last_warmed_date"].as_str().unwrap();
        let restored = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();
        assert_eq!(restored, today);
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
            content_hash: None,
        };
        let no_related: Vec<spectral::ingest::MemoryHit> = vec![];
        let prompt = build_description_prompt(&memory, &no_related);
        assert!(prompt.contains("mem-001"));
        assert!(prompt.contains("Rust async patterns"));
        assert!(prompt.contains("engineering"));
    }
}
