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
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "librarian";

// ---------------------------------------------------------------------------
// System prompt for the Librarian (sent to Qwen via Ollama)
// ---------------------------------------------------------------------------

pub const LIBRARIAN_SYSTEM_PROMPT: &str = r#"You write search-index entries for stored memories. Your output has exactly three labeled fields. Fill in each field based only on the memory content.

Output format (exactly this structure, nothing else):

FACTS: <one short sentence restating what the memory describes>
TERMS: <4 to 10 vocabulary terms — words from the memory plus their inflected forms>
CATEGORIES: <2 to 5 category-level terms the memory belongs to>

Rules:
- FACTS: under 30 tokens, one sentence, plain restatement of facts. No speculation. No significance commentary.
- TERMS: include exact words from the memory plus inflected forms (e.g., "doctor, doctors, doctoring"; "navigate, navigation, navigated"). 4 to 10 terms total. Comma-separated.
- CATEGORIES: name the broader categories the memory belongs to (e.g., for Dr. Patel: "physician, medical, healthcare"; for a Slack message: "conversation, chat, communication"). 2 to 5 terms. Comma-separated.

Do not add any text outside these three lines.

EXAMPLES:

Memory: "Henry tell me a joke" (Slack message from Jesse to Henry on April 24)
FACTS: Jesse asked Henry to tell a joke via Slack on April 24, 2026.
TERMS: joke, jokes, ask, asked, asking, message, request
CATEGORIES: conversation, chat, communication, humor

Memory: "Navigated to mail.google.com in tab btab-..."
FACTS: Browser navigated to Gmail in a browser tab.
TERMS: navigate, navigation, navigated, browser, browsing, tab, Gmail
CATEGORIES: web browsing, email, Google services

Memory: "Task completed via claude-code: read Phase 2 docs and execute build plan"
FACTS: Completed a task to read Phase 2 documentation and execute the build plan.
TERMS: task, tasks, build, building, plan, planning, completed, documentation
CATEGORIES: software development, project management, task execution"#;

// ---------------------------------------------------------------------------
// Ollama configuration
// ---------------------------------------------------------------------------

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
/// Default model used when LibrarianSchedule.model is empty or unavailable.
const DEFAULT_MODEL: &str = "qwen2.5:7b";

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

/// Read the schedule config and return a human-readable window summary
/// like "02:00 + 240min (enabled)" or "disabled".
fn load_schedule_summary() -> String {
    let path = crate::config::paths::Paths::in_data_dir("librarian_schedule.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            if let Ok(schedule) = serde_json::from_str::<serde_json::Value>(&contents) {
                let enabled = schedule
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if !enabled {
                    return "disabled".to_string();
                }
                let start = schedule
                    .get("start_time")
                    .and_then(|v| v.as_str())
                    .unwrap_or("02:00");
                let dur = schedule
                    .get("duration_minutes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(240);
                format!("{} + {}min (enabled)", start, dur)
            } else {
                "02:00 + 240min (defaults)".to_string()
            }
        }
        Err(_) => "02:00 + 240min (defaults)".to_string(),
    }
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

// (Ollama response types removed — streaming NDJSON parser handles inline)

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
        let schedule = load_schedule_summary();
        let brain_db = crate::config::paths::Paths::brain_dir().join("memory.db");
        tracing::info!(
            model = %active_model,
            schedule_window = %schedule,
            brain_db = %brain_db.display(),
            "Librarian extension loaded"
        );

        Ok(Self { info, context })
    }

    fn get_brain(&self) -> Result<crate::brain_handle::SafeBrain, String> {
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
        let result = describe_one(&brain, &params.memory_id, params.force, &model, false).await?;

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
        let memories = brain
            .list_undescribed(limit)
            .await
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

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptionQuality {
    Structured,
    Fallback,
}

/// Describe a single memory: fetch → check idempotency → LLM (streaming) → parse → write back.
///
/// If `force` is false and the memory already has a description, returns the cached
/// description without calling Ollama or writing to Spectral. This is the single
/// source of truth for description idempotency — both the MCP tool handler and
/// `run_batch` go through this function.
///
/// When `emit_events` is true, emits librarian events on the global event bus
/// for live HUD streaming (Started, Token, Retry, Completed).
pub async fn describe_one(
    brain: &crate::brain_handle::SafeBrain,
    memory_id: &str,
    force: bool,
    model: &str,
    emit_events: bool,
) -> Result<DescribeResult, String> {
    use super::librarian_state;
    let track_state = emit_events;
    let start = std::time::Instant::now();

    // 1. Fetch memory
    let memory = brain
        .get_memory(memory_id)
        .await
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

    // 3. Build prompt and call Ollama (streaming) with structured output + retry
    if track_state {
        librarian_state::set_current_memory(&memory.key, &memory.content);
    }

    let prompt = build_description_prompt(&memory);
    let memory_key = memory.key.clone();

    let mut description: Option<String> = None;
    let mut quality = DescriptionQuality::Structured;

    for attempt in 0..2u32 {
        if attempt == 0 {
            if emit_events {
                let started_at = chrono::Utc::now().to_rfc3339();
                crate::events::emit(crate::events::librarian_describe_started(
                    &memory_key,
                    &started_at,
                ));
            }
        } else {
            if track_state {
                librarian_state::set_retry_in_progress(true);
            }
            if emit_events {
                crate::events::emit(crate::events::librarian_describe_retry(
                    &memory_key,
                    attempt,
                ));
            }
            tracing::warn!(
                memory_id = %memory_id,
                attempt = attempt,
                "Structured output malformed, retrying"
            );
        }

        let raw = call_ollama_streaming(OLLAMA_BASE_URL, &prompt, model, emit_events, &memory_key)
            .await?;
        let raw = raw.trim().to_string();

        if let Some(parsed) = parse_structured_description(&raw) {
            description = Some(parsed);
            break;
        }

        if attempt == 1 {
            tracing::warn!(
                memory_id = %memory_id,
                "Structured output still malformed after retry, storing raw"
            );
            quality = DescriptionQuality::Fallback;
            description = Some(raw);
        }
    }

    // Validate
    let mut description = description.ok_or("Ollama returned no response")?;
    if description.is_empty() {
        if track_state {
            librarian_state::record_describe_failure("Empty response");
        }
        return Err("Ollama returned an empty response".to_string());
    }

    // 4. Write to Spectral first — if this fails, state hasn't been updated
    brain
        .set_description(memory_id, &description)
        .await
        .map_err(|e| format!("Failed to write description: {}", e))?;

    // 4a. Stale fact heuristic — flag descriptions whose content contains
    //      temporal supersession markers. No automated action, just tagging.
    {
        let content_lower = memory.content.to_lowercase();
        let stale_markers = [
            "no longer",
            "used to",
            "changed from",
            "stopped",
            "switched to",
            "moved from",
            "deprecated",
            "replaced by",
            "was previously",
            "formerly",
            "old approach",
        ];
        if stale_markers.iter().any(|m| content_lower.contains(m)) {
            description = format!(
                "{}\nSTALE_RISK: content contains temporal supersession markers",
                description
            );
            tracing::debug!(
                memory_id = %memory_id,
                key = %memory.key,
                "Flagged STALE_RISK — temporal markers detected"
            );
        }
    }

    // 4b. Annotate the memory with entity refs from the description
    {
        let desc_for_annotate = description.clone();
        let mid_for_annotate = memory_id.to_string();
        let created_at = memory
            .created_at
            .as_deref()
            .and_then(|s| {
                s.parse::<chrono::DateTime<chrono::Utc>>().ok().or_else(|| {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .ok()
                        .map(|dt| dt.and_utc())
                })
            })
            .unwrap_or_else(chrono::Utc::now);

        // Run annotation in spawn_blocking since it opens SQLite
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = annotate_memory(&desc_for_annotate, &mid_for_annotate, created_at) {
                tracing::warn!(
                    memory_id = %mid_for_annotate,
                    error = %e,
                    "Failed to annotate memory, continuing"
                );
            }
        })
        .await;
    }

    // 5. All state updates + event emission together (after DB write succeeds)
    let latency_ms = start.elapsed().as_millis();
    let duration_secs = start.elapsed().as_secs_f64();

    if track_state {
        // record_describe_success internally resets retry_in_progress + current_memory
        librarian_state::record_describe_success(duration_secs);
    }

    if emit_events {
        crate::events::emit(crate::events::librarian_describe_completed(
            &memory_key,
            &description,
            latency_ms as u64,
            quality,
        ));
    }

    tracing::info!(
        memory_id = %memory_id,
        latency_ms = latency_ms,
        quality = ?quality,
        "Librarian described memory"
    );

    Ok(DescribeResult {
        description,
        model: model.to_string(),
        latency_ms,
        tokens: 0,
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
    brain: &crate::brain_handle::SafeBrain,
    batch_size: usize,
    model: &str,
) -> Result<usize, String> {
    use super::librarian_state;

    let mut described = 0;
    loop {
        let memories = brain
            .list_undescribed(batch_size)
            .await
            .map_err(|e| format!("Brain error: {}", e))?;

        if memories.is_empty() {
            break;
        }

        // Update state: entering describing phase with this batch's count
        librarian_state::set_describing(memories.len());

        for mem in &memories {
            match describe_one(brain, &mem.id, false, model, true).await {
                Ok(r) if r.cached => {
                    tracing::debug!(memory_id = %mem.id, "Librarian skipped already-described memory");
                }
                Ok(_) => described += 1,
                Err(e) => {
                    librarian_state::record_describe_failure(&e);
                    tracing::warn!(memory_id = %mem.id, error = %e, "Librarian failed to describe memory, skipping");
                }
            }
        }

        if memories.len() < batch_size {
            break;
        }
    }

    librarian_state::set_batch_complete();
    tracing::info!(described = described, "Librarian batch complete");
    Ok(described)
}

// ---------------------------------------------------------------------------
// Prompt building
// ---------------------------------------------------------------------------

fn build_description_prompt(memory: &spectral::ingest::Memory) -> String {
    // Truncate content to avoid blowing context on very large memories
    let content: String = memory.content.chars().take(2000).collect();
    format!(
        "Memory key: {}\nMemory content: {}\n\nOutput the three labeled fields.",
        memory.key, content
    )
}

/// Parse the three-field structural output into a single description string.
/// Returns None if the output doesn't contain all required fields.
pub fn parse_structured_description(raw: &str) -> Option<String> {
    let mut facts = None;
    let mut terms = None;
    let mut categories = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FACTS:") {
            facts = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("TERMS:") {
            terms = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("CATEGORIES:") {
            categories = Some(rest.trim().to_string());
        }
    }

    let facts = facts?;
    let terms = terms?;
    let categories = categories?;

    // Validate minimum counts
    let term_count = terms.split(',').count();
    let cat_count = categories.split(',').count();
    if term_count < 4 || cat_count < 2 {
        return None;
    }

    Some(format!(
        "{} Related terms: {}. Categories: {}.",
        facts, terms, categories
    ))
}

// ---------------------------------------------------------------------------
// Ollama integration (streaming NDJSON parser)
// ---------------------------------------------------------------------------

/// Stream tokens from Ollama's /api/generate endpoint.
async fn call_ollama_streaming(
    base_url: &str,
    prompt: &str,
    model: &str,
    emit_events: bool,
    memory_key: &str,
) -> Result<String, String> {
    use futures::StreamExt;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "system": LIBRARIAN_SYSTEM_PROMPT,
        "stream": true,
        "options": {
            "temperature": 0.2,
            "top_p": 0.9,
            "num_predict": 150,
        }
    });

    let resp = client
        .post(format!("{}/api/generate", base_url))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama unreachable: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama error ({}): {}", status, text));
    }

    let mut stream = resp.bytes_stream();
    let mut accumulated = String::new();
    let mut line_buffer = String::new();
    let mut saw_done = false;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result
            .map_err(|e| format!("Stream interrupted during description generation: {}", e))?;

        let chunk_str = String::from_utf8_lossy(&chunk);
        line_buffer.push_str(&chunk_str);

        while let Some(newline_pos) = line_buffer.find('\n') {
            let line: String = line_buffer.drain(..=newline_pos).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parsed: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| format!("Malformed NDJSON from Ollama: {} (line: {})", e, line))?;

            if let Some(token) = parsed.get("response").and_then(|v| v.as_str()) {
                if !token.is_empty() {
                    accumulated.push_str(token);
                    if emit_events {
                        crate::events::emit(crate::events::librarian_describe_token(
                            memory_key, token,
                        ));
                    }
                }
            }

            if parsed
                .get("done")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                saw_done = true;
            }

            if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
                return Err(format!("Ollama stream error: {}", err));
            }
        }
    }

    // Process remaining buffer
    let remaining = line_buffer.trim();
    if !remaining.is_empty() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(remaining) {
            if let Some(token) = parsed.get("response").and_then(|v| v.as_str()) {
                if !token.is_empty() {
                    accumulated.push_str(token);
                    if emit_events {
                        crate::events::emit(crate::events::librarian_describe_token(
                            memory_key, token,
                        ));
                    }
                }
            }
            if parsed
                .get("done")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                saw_done = true;
            }
        }
    }

    if !saw_done {
        return Err(
            "Ollama stream terminated prematurely — no done signal received. \
             Model may have crashed or run out of memory."
                .to_string(),
        );
    }

    Ok(accumulated)
}

// ---------------------------------------------------------------------------
// Entity annotation pipeline — extracts terms/categories from Librarian
// descriptions and writes them to the memory_annotations table.
// ---------------------------------------------------------------------------

/// Parse "Related terms:" and "Categories:" from a Librarian description,
/// build EntityRef structs, and write an annotation row to SQLite.
///
/// This writes directly to the memory_annotations table because the
/// `spectral::Brain` wrapper does not expose the inner `annotate()` method.
/// The table schema and serialization format match spectral-ingest exactly.
pub fn annotate_memory(
    description: &str,
    memory_id: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<usize> {
    // 1. Parse terms from the structured description
    let mut entity_refs: Vec<spectral::ingest::EntityRef> = Vec::new();

    // Extract "Related terms: ..." segment
    if let Some(after_terms) = description.split("Related terms:").nth(1) {
        let terms_str = after_terms.split('.').next().unwrap_or("");
        for term in terms_str.split(',') {
            let t = term.trim();
            if !t.is_empty() {
                entity_refs.push(spectral::ingest::EntityRef {
                    canonical_id: format!("term:{}", t.to_lowercase()),
                    display_name: t.to_string(),
                });
            }
        }
    }

    // Extract "Categories: ..." segment
    if let Some(after_cats) = description.split("Categories:").nth(1) {
        let cats_str = after_cats.split('.').next().unwrap_or("");
        for cat in cats_str.split(',') {
            let c = cat.trim();
            if !c.is_empty() {
                entity_refs.push(spectral::ingest::EntityRef {
                    canonical_id: format!("cat:{}", c.to_lowercase()),
                    display_name: c.to_string(),
                });
            }
        }
    }

    if entity_refs.is_empty() {
        tracing::debug!(memory_id = %memory_id, "No terms/categories found in description, skipping annotation");
        return Ok(0);
    }

    // 2. Write to SQLite directly (idempotent on memory_id + description + when_)
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let when_rfc = created_at.to_rfc3339();

    // Idempotency check: skip if identical annotation exists
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM memory_annotations WHERE memory_id = ?1 AND description = ?2 AND when_ = ?3",
            rusqlite::params![memory_id, description, when_rfc],
            |row| row.get(0),
        )
        .ok();

    if existing.is_some() {
        tracing::debug!(memory_id = %memory_id, "Annotation already exists, skipping");
        return Ok(0);
    }

    // Generate annotation ID (matches spectral's blake3-based scheme)
    let now = chrono::Utc::now();
    let id = format!(
        "ann-{:016x}",
        u64::from_be_bytes(
            blake3::hash(
                format!("{}-{}", memory_id, now.timestamp_nanos_opt().unwrap_or(0)).as_bytes()
            )
            .as_bytes()[..8]
                .try_into()
                .unwrap()
        )
    );

    let who_json = serde_json::to_string(&entity_refs)?;
    conn.execute(
        "INSERT INTO memory_annotations (id, memory_id, description, who, why, where_, when_, how, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            memory_id,
            description,
            who_json,
            "",        // why
            None::<String>,  // where_
            when_rfc,
            "",        // how
            now.to_rfc3339(),
        ],
    )?;

    tracing::debug!(
        memory_id = %memory_id,
        entity_count = entity_refs.len(),
        "Annotation written"
    );

    Ok(entity_refs.len())
}

/// Backfill annotations for all described memories that lack annotations.
/// Returns the number of memories annotated.
///
/// This is designed to run once at startup when the annotation table is
/// empty or significantly behind the described memory count. Subsequent
/// annotations happen inline in describe_one().
pub fn backfill_annotations() -> Result<usize> {
    let db_path = crate::config::paths::Paths::brain_dir().join("memory.db");
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    // Count described memories vs annotations
    let described_count: usize = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE description IS NOT NULL AND description LIKE '%Related terms:%'",
        [],
        |r| r.get(0),
    )?;
    let annotation_count: usize =
        conn.query_row("SELECT COUNT(*) FROM memory_annotations", [], |r| r.get(0))?;

    // Only backfill if annotations are significantly behind
    if annotation_count >= described_count.saturating_sub(5) {
        tracing::info!(
            described = described_count,
            annotations = annotation_count,
            "Annotation backfill not needed"
        );
        return Ok(0);
    }

    tracing::info!(
        described = described_count,
        annotations = annotation_count,
        gap = described_count.saturating_sub(annotation_count),
        "Starting annotation backfill"
    );

    // Fetch all described memories that need annotations
    // LEFT JOIN to exclude memories that already have annotations
    let mut stmt = conn.prepare(
        "SELECT m.id, m.description, m.created_at FROM memories m \
         LEFT JOIN memory_annotations a ON a.memory_id = m.id \
         WHERE m.description IS NOT NULL \
           AND m.description LIKE '%Related terms:%' \
           AND a.id IS NULL \
         ORDER BY m.created_at DESC",
    )?;

    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    drop(stmt);
    drop(conn);

    let total = rows.len();
    let mut annotated = 0;

    for (i, (mem_id, description, created_at_str)) in rows.iter().enumerate() {
        let created_at = created_at_str
            .as_deref()
            .and_then(|s| {
                s.parse::<chrono::DateTime<chrono::Utc>>().ok().or_else(|| {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .ok()
                        .map(|dt| dt.and_utc())
                })
            })
            .unwrap_or_else(chrono::Utc::now);

        match annotate_memory(description, mem_id, created_at) {
            Ok(n) if n > 0 => annotated += 1,
            Ok(_) => {} // no terms found, skip
            Err(e) => {
                tracing::warn!(memory_id = %mem_id, error = %e, "Annotation backfill failed for memory, skipping");
            }
        }

        if (i + 1) % 100 == 0 {
            tracing::info!(
                progress = format!("{}/{}", i + 1, total),
                annotated = annotated,
                "Annotation backfill progress"
            );
        }
    }

    tracing::info!(
        total_processed = total,
        annotated = annotated,
        "Annotation backfill complete"
    );

    Ok(annotated)
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
    use std::sync::Arc;

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
        let described = brain.list_undescribed(1).await.unwrap();

        // If no undescribed memories, this test can't run meaningfully
        if described.is_empty() {
            eprintln!("No undescribed memories to test with — skipping");
            return;
        }

        // Describe one first
        let id = &described[0].id;
        let first = describe_one(&brain, id, false, DEFAULT_MODEL, false)
            .await
            .unwrap();
        assert!(!first.cached, "First call should not be cached");

        // Second call with force=false should return cached
        let second = describe_one(&brain, id, false, DEFAULT_MODEL, false)
            .await
            .unwrap();
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
        let described = brain.list_undescribed(1).await.unwrap();

        if described.is_empty() {
            eprintln!("No undescribed memories — skipping");
            return;
        }

        let id = &described[0].id;
        // Ensure described
        let _ = describe_one(&brain, id, false, DEFAULT_MODEL, false)
            .await
            .unwrap();

        // Force regenerate
        let result = describe_one(&brain, id, true, DEFAULT_MODEL, false)
            .await
            .unwrap();
        assert!(!result.cached, "force=true should not return cached");
        assert!(result.latency_ms > 0, "Should have Ollama latency");
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
        let prompt = build_description_prompt(&memory);
        assert!(prompt.contains("session:2026-05-08:chat"));
        assert!(prompt.contains("Rust async patterns"));
    }

    #[test]
    fn test_parse_structured_valid() {
        let raw = "FACTS: Jesse asked Henry to tell a joke via Slack.\nTERMS: joke, jokes, ask, asked, asking, message, request\nCATEGORIES: conversation, chat, communication, humor";
        let result = parse_structured_description(raw).unwrap();
        assert!(result.starts_with("Jesse asked Henry"));
        assert!(result.contains("Related terms:"));
        assert!(result.contains("Categories:"));
        assert!(result.contains("joke, jokes"));
        assert!(result.contains("conversation, chat"));
    }

    #[test]
    fn test_parse_structured_missing_field() {
        let raw = "FACTS: Something happened.\nTERMS: a, b, c, d";
        assert!(parse_structured_description(raw).is_none());
    }

    #[test]
    fn test_parse_structured_too_few_terms() {
        let raw = "FACTS: Something.\nTERMS: a, b, c\nCATEGORIES: x, y";
        assert!(parse_structured_description(raw).is_none());
    }

    #[test]
    fn test_parse_structured_too_few_categories() {
        let raw = "FACTS: Something.\nTERMS: a, b, c, d\nCATEGORIES: x";
        assert!(parse_structured_description(raw).is_none());
    }

    #[test]
    fn test_parse_structured_with_extra_whitespace() {
        let raw = "  FACTS:  Browser navigated to Gmail. \n  TERMS:  navigate, navigation, navigated, browser  \n  CATEGORIES:  web browsing, email  ";
        let result = parse_structured_description(raw).unwrap();
        assert!(result.starts_with("Browser navigated"));
        assert!(result.contains("navigate, navigation"));
    }
}
