use crate::conversation::message::{Message, MessageContent};
use crate::cost_router::cache::{
    is_turn_context_text, wrap_turn_context, CacheTtl, SystemPromptParts,
};
use crate::mcp_utils::extract_text_from_resource;
use crate::model::ModelConfig;
use crate::providers::base::Usage;
use crate::providers::errors::ProviderError;
use crate::providers::utils::{convert_image, sanitize_tool_use_id, ImageFormat};
use anyhow::{anyhow, Result};
use rmcp::model::{object, CallToolRequestParams, ErrorCode, ErrorData, JsonObject, Role, Tool};
use rmcp::object as json_object;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $str:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s.to_lowercase().as_str() {
                    $($str => Ok(Self::$variant),)+
                    other => Err(format!("unknown {}: '{other}'", stringify!($name))),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$variant => write!(f, $str),)+ }
            }
        }
    }
}

string_enum!(ThinkingType { Adaptive => "adaptive", Enabled => "enabled", Disabled => "disabled" });
string_enum!(ThinkingEffort { Low => "low", Medium => "medium", High => "high", Max => "max" });

pub fn supports_adaptive_thinking(model_name: &str) -> bool {
    let lower = model_name.to_lowercase();
    lower.contains("claude-opus-4-6") || lower.contains("claude-sonnet-4-6")
}

pub fn thinking_type(model_config: &ModelConfig) -> ThinkingType {
    let model_lower = model_config.model_name.to_lowercase();
    if !model_lower.contains("claude") {
        return ThinkingType::Disabled;
    }

    let is_adaptive_model = supports_adaptive_thinking(&model_config.model_name);

    if let Some(s) =
        model_config.get_config_param::<String>("thinking_type", "CLAUDE_THINKING_TYPE")
    {
        let tt = s.parse::<ThinkingType>().unwrap_or_else(|e| {
            tracing::warn!("{e}");
            ThinkingType::Disabled
        });
        if tt == ThinkingType::Adaptive && !is_adaptive_model {
            tracing::warn!(
                "Adaptive thinking not supported for {}, disabling thinking",
                model_config.model_name
            );
            return ThinkingType::Disabled;
        }
        return tt;
    }

    if is_adaptive_model {
        ThinkingType::Adaptive
    } else if std::env::var("CLAUDE_THINKING_ENABLED").is_ok() {
        tracing::warn!(
            "CLAUDE_THINKING_ENABLED is deprecated, use CLAUDE_THINKING_TYPE=enabled instead"
        );
        ThinkingType::Enabled
    } else {
        ThinkingType::Disabled
    }
}

// Constants for frequently used strings in Anthropic API format
const TYPE_FIELD: &str = "type";
const CONTENT_FIELD: &str = "content";
const TEXT_TYPE: &str = "text";
const ROLE_FIELD: &str = "role";
const USER_ROLE: &str = "user";
const ASSISTANT_ROLE: &str = "assistant";
const TOOL_USE_TYPE: &str = "tool_use";
const TOOL_RESULT_TYPE: &str = "tool_result";
const THINKING_TYPE: &str = "thinking";
const REDACTED_THINKING_TYPE: &str = "redacted_thinking";
const CACHE_CONTROL_FIELD: &str = "cache_control";
const ID_FIELD: &str = "id";
const NAME_FIELD: &str = "name";
const INPUT_FIELD: &str = "input";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";
const IS_ERROR_FIELD: &str = "is_error";
const SIGNATURE_FIELD: &str = "signature";
const DATA_FIELD: &str = "data";
const EVENT_MESSAGE_START: &str = "message_start";
const EVENT_MESSAGE_DELTA: &str = "message_delta";
const EVENT_MESSAGE_STOP: &str = "message_stop";
const EVENT_CONTENT_BLOCK_START: &str = "content_block_start";
const EVENT_CONTENT_BLOCK_DELTA: &str = "content_block_delta";
const EVENT_CONTENT_BLOCK_STOP: &str = "content_block_stop";

/// Convert internal Message format to Anthropic's API message specification
pub fn format_messages(messages: &[Message]) -> Vec<Value> {
    let mut anthropic_messages = Vec::new();

    // Diagnostic: count image blocks across all messages
    let total_image_blocks: usize = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| matches!(c, MessageContent::Image(_)))
        .count();
    if total_image_blocks > 0 {
        tracing::info!(
            image_blocks = total_image_blocks,
            total_messages = messages.len(),
            "[anthropic::format_messages] image content blocks present in payload"
        );
    }

    for message in messages {
        let role = match message.role {
            Role::User => USER_ROLE,
            Role::Assistant => ASSISTANT_ROLE,
        };

        let mut content = Vec::new();
        for msg_content in &message.content {
            match msg_content {
                MessageContent::Text(text) => {
                    if !text.text.trim().is_empty() {
                        content.push(json!({
                            TYPE_FIELD: TEXT_TYPE,
                            TEXT_TYPE: text.text
                        }));
                    }
                }
                MessageContent::ToolRequest(tool_request) => {
                    match &tool_request.tool_call {
                        Ok(tool_call) => {
                            content.push(json!({
                                TYPE_FIELD: TOOL_USE_TYPE,
                                ID_FIELD: sanitize_tool_use_id(&tool_request.id),
                                NAME_FIELD: tool_call.name,
                                INPUT_FIELD: tool_call.arguments
                            }));
                        }
                        Err(_tool_error) => {
                            // Skip malformed tool requests - they shouldn't be sent to Anthropic
                            // This maintains the existing behavior for ToolRequest errors
                        }
                    }
                }
                MessageContent::ToolResponse(tool_response) => match &tool_response.tool_result {
                    Ok(result) => {
                        let text = result
                            .content
                            .iter()
                            .filter_map(|c| {
                                if let Some(t) = c.as_text() {
                                    return Some(t.text.clone());
                                }
                                if let Some(r) = c.as_resource() {
                                    let text = extract_text_from_resource(&r.resource);
                                    if !text.is_empty() {
                                        return Some(text);
                                    }
                                }
                                None
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        // A tool that FAILED reports it via CallToolResult::error
                        // (is_error = true) and still arrives here as Ok — the
                        // transport succeeded, the tool did not. Dropping the flag
                        // made every ordinary tool failure look like a SUCCESS whose
                        // text happens to start with "Error:", so the model had only
                        // prose to steer on. Weak models steer on the structural
                        // flag; this is what marks a result as something to retry
                        // differently rather than build on.
                        let mut result_block = json!({
                            TYPE_FIELD: TOOL_RESULT_TYPE,
                            TOOL_USE_ID_FIELD: sanitize_tool_use_id(&tool_response.id),
                            CONTENT_FIELD: text
                        });
                        if result.is_error.unwrap_or(false) {
                            result_block[IS_ERROR_FIELD] = json!(true);
                        }
                        content.push(result_block);
                    }
                    Err(tool_error) => {
                        content.push(json!({
                            TYPE_FIELD: TOOL_RESULT_TYPE,
                            TOOL_USE_ID_FIELD: sanitize_tool_use_id(&tool_response.id),
                            CONTENT_FIELD: format!("Error: {}", tool_error),
                            IS_ERROR_FIELD: true
                        }));
                    }
                },
                MessageContent::ToolConfirmationRequest(_tool_confirmation_request) => {
                    // Skip tool confirmation requests
                }
                MessageContent::ActionRequired(_action_required) => {
                    // Skip action required messages - they're for UI only
                }
                MessageContent::SystemNotification(_) => {
                    // Skip
                }
                MessageContent::Thinking(thinking) => {
                    if !thinking.signature.is_empty() {
                        content.push(json!({
                            TYPE_FIELD: THINKING_TYPE,
                            THINKING_TYPE: thinking.thinking,
                            SIGNATURE_FIELD: thinking.signature
                        }));
                    }
                }
                MessageContent::RedactedThinking(redacted) => {
                    content.push(json!({
                        TYPE_FIELD: REDACTED_THINKING_TYPE,
                        DATA_FIELD: redacted.data
                    }));
                }
                MessageContent::Image(image) => {
                    content.push(convert_image(image, &ImageFormat::Anthropic));
                }
                MessageContent::FrontendToolRequest(tool_request) => {
                    if let Ok(tool_call) = &tool_request.tool_call {
                        content.push(json!({
                            TYPE_FIELD: TOOL_USE_TYPE,
                            ID_FIELD: sanitize_tool_use_id(&tool_request.id),
                            NAME_FIELD: tool_call.name,
                            INPUT_FIELD: tool_call.arguments
                        }));
                    }
                }
            }
        }

        // Skip messages with empty content
        if !content.is_empty() {
            anthropic_messages.push(json!({
                ROLE_FIELD: role,
                CONTENT_FIELD: content
            }));
        }
    }

    // If no messages, add a default one
    if anthropic_messages.is_empty() {
        anthropic_messages.push(json!({
            ROLE_FIELD: USER_ROLE,
            CONTENT_FIELD: [{
                TYPE_FIELD: TEXT_TYPE,
                TEXT_TYPE: "Ignore"
            }]
        }));
    }

    // Add "cache_control" to the last and second-to-last "user" messages.
    // During each turn, we mark the final message with cache_control so the conversation can be
    // incrementally cached. The second-to-last user message is also marked for caching with the
    // cache_control parameter, so that this checkpoint can read from the previous cache.
    //
    // The marker goes on the last block that is NOT per-turn host context (MOIM,
    // a relocated volatile suffix). Those blocks ride only on the newest user
    // message and are gone when the next turn replays it as history, so a
    // breakpoint on one would cache a prefix no later request can ever repeat.
    //
    // A user message that is ONLY such context (MOIM riding as its own message
    // after the tool results) has no anchor and does not use up a breakpoint,
    // so both still land on the last two user messages that the next turn
    // replays.
    let mut user_count = 0;
    for message in anthropic_messages.iter_mut().rev() {
        if message.get(ROLE_FIELD) == Some(&json!(USER_ROLE)) {
            let mut anchored = false;
            if let Some(content) = message.get_mut(CONTENT_FIELD) {
                if let Some(content_array) = content.as_array_mut() {
                    if let Some(anchor) = content_array
                        .iter_mut()
                        .rev()
                        .find(|block| !is_turn_context_block(block))
                    {
                        if let Some(obj) = anchor.as_object_mut() {
                            obj.insert(
                                CACHE_CONTROL_FIELD.to_string(),
                                json!({ TYPE_FIELD: "ephemeral" }),
                            );
                            anchored = true;
                        }
                    }
                }
            }
            if !anchored {
                continue;
            }
            user_count += 1;
            if user_count >= 2 {
                break;
            }
        }
    }

    anthropic_messages
}

/// A formatted text block whose text is per-turn host context (see
/// [`is_turn_context_text`]).
fn is_turn_context_block(block: &Value) -> bool {
    block.get(TYPE_FIELD).and_then(Value::as_str) == Some(TEXT_TYPE)
        && block
            .get(TEXT_TYPE)
            .and_then(Value::as_str)
            .is_some_and(is_turn_context_text)
}

/// Append the system prompt's volatile suffix as a trailing text block of the
/// LAST message, when that message is a user message. Returns whether it did.
///
/// Anthropic orders a request tools → system → messages, so a volatile block in
/// `system` sits upstream of the entire history: any change to it (a worker
/// flipping state, a briefing read) misses every message-level breakpoint and
/// re-writes the whole history at the 1.25x write rate. At the tail it sits
/// after both rolling breakpoints and costs only its own tokens. Must run AFTER
/// [`format_messages`] placed the breakpoints (it also skips turn-context
/// blocks, so the order is belt and braces).
fn append_turn_context_to_last_user(messages: &mut [Value], volatile_suffix: &str) -> bool {
    let Some(last) = messages.last_mut() else {
        return false;
    };
    if last.get(ROLE_FIELD).and_then(Value::as_str) != Some(USER_ROLE) {
        return false;
    }
    let Some(content) = last.get_mut(CONTENT_FIELD).and_then(Value::as_array_mut) else {
        return false;
    };
    content.push(json!({
        TYPE_FIELD: TEXT_TYPE,
        TEXT_TYPE: wrap_turn_context(volatile_suffix),
    }));
    true
}

fn anthropic_flavored_input_schema(input_schema: Arc<JsonObject>) -> Arc<JsonObject> {
    if input_schema.is_empty() {
        return Arc::new(json_object!({
            "type": "object",
        }));
    }
    input_schema
}

/// Convert internal Tool format to Anthropic's API tool specification
pub fn format_tools(tools: &[Tool]) -> Vec<Value> {
    let mut unique_tools = HashSet::new();
    let mut tool_specs = Vec::new();

    for tool in tools {
        if unique_tools.insert(tool.name.clone()) {
            tool_specs.push(json!({
                NAME_FIELD: tool.name,
                "description": tool.description,
                "input_schema": anthropic_flavored_input_schema(tool.input_schema.clone())
            }));
        }
    }

    // Add "cache_control" to the last tool spec, if any. This means that all tool definitions,
    // will be cached as a single prefix.
    if let Some(last_tool) = tool_specs.last_mut() {
        last_tool.as_object_mut().unwrap().insert(
            CACHE_CONTROL_FIELD.to_string(),
            json!({ TYPE_FIELD: "ephemeral" }),
        );
    }

    tool_specs
}

const EPHEMERAL: &str = "ephemeral";
const TTL_FIELD: &str = "ttl";

/// The XML-ish wrapper that delimits pinned read-only context inside the cached
/// system block — the analogue of `<repo_map>` for the [`PrefixSegment::ReadOnlyFiles`]
/// slot. The cache guards key on this marker to prove the segment lands inside
/// the cached prefix (after the repo-map) and never leaks into the volatile tail.
const READ_ONLY_FILES_OPEN: &str = "<read_only_files>";
const READ_ONLY_FILES_CLOSE: &str = "</read_only_files>";

/// A read-only context file pinned into the cached system prefix for the session.
///
/// Explicitly-referenced read-only context — pinned files, CLAUDE.md-type project
/// context — is stable across turns, so it belongs *inside* the cached system
/// block (billed once at the cache write, then ~0.1× on every read) rather than
/// re-sent in the volatile `messages` tail every turn. Its content must never be
/// reordered or mutated turn-to-turn, or the byte-exact cached prefix breaks and
/// every turn pays a fresh write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyFile {
    pub path: String,
    pub content: String,
}

impl ReadOnlyFile {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        ReadOnlyFile {
            path: path.into(),
            content: content.into(),
        }
    }
}

/// The `cache_control` value for a breakpoint at the given TTL. 5-minute is the
/// bare `ephemeral` marker; 1-hour adds the extended `"ttl": "1h"`. TTL policy —
/// including the honest 1-hour break-even — lives in [`CacheTtl`].
fn cache_control(ttl: CacheTtl) -> Value {
    match ttl.extended_ttl_str() {
        Some(ttl_str) => json!({ TYPE_FIELD: EPHEMERAL, TTL_FIELD: ttl_str }),
        None => json!({ TYPE_FIELD: EPHEMERAL }),
    }
}

/// Render pinned read-only files into a single canonical, byte-stable block for
/// the cached system prefix, or `None` when there is nothing to pin (so the
/// system block is byte-identical to the no-read-only case). Files are emitted in
/// the order supplied — the caller owns a stable order so the cached prefix hits.
fn format_read_only_files(read_only_files: &[ReadOnlyFile]) -> Option<String> {
    if read_only_files.is_empty() {
        return None;
    }
    let mut out = String::from(READ_ONLY_FILES_OPEN);
    for file in read_only_files {
        out.push_str("\n<file path=\"");
        out.push_str(&file.path);
        out.push_str("\">\n");
        out.push_str(&file.content);
        out.push_str("\n</file>");
    }
    out.push('\n');
    out.push_str(READ_ONLY_FILES_CLOSE);
    Some(out)
}

/// Convert system message to Anthropic's API system specification (no pinned
/// read-only context, default 5-minute cache TTL).
pub fn format_system(system: &str) -> Value {
    format_system_with_read_only(system, &[])
}

/// Convert the system message to Anthropic's system spec with pinned read-only
/// context folded into the SAME cached block, after the base system (which
/// already carries the repo-map). This fills the reserved
/// [`PrefixSegment::ReadOnlyFiles`] slot without adding a fifth breakpoint: the
/// single system text block still carries exactly one `cache_control` marker. The
/// breakpoint always lands on the last block, so `format_tools` → `format_system`
/// stays canonical order. TTL is applied to every breakpoint later, in
/// [`apply_cache_ttl`].
pub fn format_system_with_read_only(system: &str, read_only_files: &[ReadOnlyFile]) -> Value {
    format_system_parts(
        &SystemPromptParts::all_stable(system.to_string()),
        read_only_files,
    )
}

/// Convert a stable/volatile-split system prompt into Anthropic's system spec.
///
/// Emits the stable prefix (plus any pinned read-only context) as the FIRST text
/// block, carrying the sole `cache_control` marker, and the volatile suffix as a
/// SECOND, unmarked block. Anthropic caches everything up to and including a
/// marked block, so the turn-specific tail can be rewritten every turn without
/// touching the cached prefix — which is the whole point. A second block adds no
/// breakpoint, so the 4-breakpoint cap is untouched.
///
/// With an empty volatile suffix this produces exactly one block and is
/// byte-identical to the pre-split payload.
pub fn format_system_parts(system: &SystemPromptParts, read_only_files: &[ReadOnlyFile]) -> Value {
    let stable = system.stable_prefix();
    let text = match format_read_only_files(read_only_files) {
        Some(block) if stable.is_empty() => block,
        Some(block) => format!("{stable}\n\n{block}"),
        None => stable.to_string(),
    };
    let mut blocks = vec![json!({
        TYPE_FIELD: TEXT_TYPE,
        TEXT_TYPE: text,
        CACHE_CONTROL_FIELD: cache_control(CacheTtl::FiveMinute)
    })];
    if !system.volatile_suffix().is_empty() {
        // Deliberately NO cache_control: a breakpoint here would cache a block
        // that changes every turn, paying the 1.25× write premium each time for
        // a read that can never hit.
        blocks.push(json!({
            TYPE_FIELD: TEXT_TYPE,
            TEXT_TYPE: system.volatile_suffix(),
        }));
    }
    Value::Array(blocks)
}

/// Upgrade every `cache_control` breakpoint already present in `payload` to the
/// requested TTL. This NEVER adds or removes a breakpoint — it only re-annotates
/// the `ephemeral` markers the formatters emitted — so the Anthropic 4-breakpoint
/// cap is preserved regardless of TTL. The 5-minute TTL is exactly the marker the
/// formatters already produce, so this is a no-op for it; the 1-hour TTL adds
/// `"ttl": "1h"` to each existing marker (tools, system, and the rolling
/// user-message reads alike, so the whole warm prefix shares one lifetime).
fn apply_cache_ttl(payload: &mut Value, ttl: CacheTtl) {
    if !ttl.is_extended() {
        return; // formatters already emit the 5-minute `ephemeral` marker
    }
    fn walk(value: &mut Value, replacement: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(cc) = map.get_mut(CACHE_CONTROL_FIELD) {
                    // Only upgrade the bare 5-minute marker the formatters emit;
                    // leave anything else untouched.
                    if cc.get(TYPE_FIELD).and_then(|t| t.as_str()) == Some(EPHEMERAL)
                        && cc.get(TTL_FIELD).is_none()
                    {
                        *cc = replacement.clone();
                    }
                }
                for v in map.values_mut() {
                    walk(v, replacement);
                }
            }
            Value::Array(arr) => {
                for v in arr.iter_mut() {
                    walk(v, replacement);
                }
            }
            _ => {}
        }
    }
    let replacement = cache_control(ttl);
    walk(payload, &replacement);
}

/// Convert Anthropic's API response to internal Message format
pub fn response_to_message(response: &Value) -> Result<Message> {
    let content_blocks = response
        .get(CONTENT_FIELD)
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("Invalid response format: missing content array"))?;

    let mut message = Message::assistant();

    for block in content_blocks {
        match block.get(TYPE_FIELD).and_then(|t| t.as_str()) {
            Some(TEXT_TYPE) => {
                if let Some(text) = block.get(TEXT_TYPE).and_then(|t| t.as_str()) {
                    message = message.with_text(text.to_string());
                }
            }
            Some(TOOL_USE_TYPE) => {
                let id = block
                    .get(ID_FIELD)
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| anyhow!("Missing tool_use id"))?;
                let name = block
                    .get(NAME_FIELD)
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| anyhow!("Missing tool_use name"))?
                    .to_string();
                let input = block
                    .get(INPUT_FIELD)
                    .ok_or_else(|| anyhow!("Missing tool_use input"))?;

                let tool_call =
                    CallToolRequestParams::new(name).with_arguments(object(input.clone()));
                message = message.with_tool_request(id, Ok(tool_call));
            }
            Some(THINKING_TYPE) => {
                let thinking = block
                    .get(THINKING_TYPE)
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| anyhow!("Missing thinking content"))?
                    .to_string();
                let signature = block
                    .get(SIGNATURE_FIELD)
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow!("Missing thinking signature"))?;
                message = message.with_thinking(thinking, signature);
            }
            Some(REDACTED_THINKING_TYPE) => {
                let data = block
                    .get(DATA_FIELD)
                    .and_then(|d| d.as_str())
                    .ok_or_else(|| anyhow!("Missing redacted_thinking data"))?;
                message = message.with_redacted_thinking(data);
            }
            _ => continue,
        }
    }

    Ok(message)
}

/// Extract usage information from Anthropic's API response
pub fn get_usage(data: &Value) -> Result<Usage> {
    // Extract usage data if available
    if let Some(usage) = data.get("usage") {
        // Get all token fields for analysis
        let input_tokens = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let cache_creation_tokens = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let cache_read_tokens = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let output_tokens = usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // IMPORTANT: For display purposes, we want to show the ACTUAL total tokens consumed
        // The cache pricing should only affect cost calculation, not token count display
        let total_input_tokens = input_tokens + cache_creation_tokens + cache_read_tokens;

        // Convert to i32 with bounds checking
        let total_input_i32 = total_input_tokens.min(i32::MAX as u64) as i32;
        let output_tokens_i32 = output_tokens.min(i32::MAX as u64) as i32;
        let total_tokens_i32 =
            (total_input_i32 as i64 + output_tokens_i32 as i64).min(i32::MAX as i64) as i32;

        // Carry the cache read/creation split onto the ledger. `input_tokens` above
        // is the cache-INCLUSIVE surface (fresh + creation + read); the ledger
        // (`canonical::cost`) carves these two categories back out for correct
        // pricing, and it is what makes cache hit-rate measurable day one.
        Ok(Usage::new(
            Some(total_input_i32),
            Some(output_tokens_i32),
            Some(total_tokens_i32),
        )
        .with_cache_tokens(
            Some(cache_read_tokens.min(i32::MAX as u64) as i32),
            Some(cache_creation_tokens.min(i32::MAX as u64) as i32),
        ))
    } else if data.as_object().is_some() {
        // Check if the data itself is the usage object (for message_delta events that might have usage at top level)
        let input_tokens = data
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let cache_creation_tokens = data
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let cache_read_tokens = data
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let output_tokens = data
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // If we found any token data, process it
        if input_tokens > 0
            || cache_creation_tokens > 0
            || cache_read_tokens > 0
            || output_tokens > 0
        {
            let total_input_tokens = input_tokens + cache_creation_tokens + cache_read_tokens;

            let total_input_i32 = total_input_tokens.min(i32::MAX as u64) as i32;
            let output_tokens_i32 = output_tokens.min(i32::MAX as u64) as i32;
            let total_tokens_i32 =
                (total_input_i32 as i64 + output_tokens_i32 as i64).min(i32::MAX as i64) as i32;

            tracing::debug!("🔍 Anthropic ACTUAL token counts from direct object: input={}, output={}, total={}",
                    total_input_i32, output_tokens_i32, total_tokens_i32);

            // Same cache read/creation split as the nested-usage branch above, so
            // streamed `message_delta` usage also feeds the ledger's hit-rate.
            Ok(Usage::new(
                Some(total_input_i32),
                Some(output_tokens_i32),
                Some(total_tokens_i32),
            )
            .with_cache_tokens(
                Some(cache_read_tokens.min(i32::MAX as u64) as i32),
                Some(cache_creation_tokens.min(i32::MAX as u64) as i32),
            ))
        } else {
            tracing::debug!("🔍 Anthropic no token data found in object");
            Ok(Usage::new(None, None, None))
        }
    } else {
        tracing::debug!(
            "Failed to get usage data: {}",
            ProviderError::UsageError("No usage data found in response".to_string())
        );
        // If no usage data, return None for all values
        Ok(Usage::new(None, None, None))
    }
}

pub fn thinking_effort(model_config: &ModelConfig) -> ThinkingEffort {
    match model_config.get_config_param::<String>("effort", "CLAUDE_THINKING_EFFORT") {
        Some(s) => s.parse().unwrap_or_else(|e| {
            tracing::warn!("{e}, defaulting to 'high'");
            ThinkingEffort::High
        }),
        None => ThinkingEffort::High,
    }
}

/// Select the prompt-cache TTL for this request. Defaults to the cheap 5-minute
/// `ephemeral` TTL everywhere; opt into the 1-hour extended TTL via the
/// `cache_ttl` config key or the `CLAUDE_CACHE_TTL` env var
/// (`"1h"` / `"hour"` / `"extended"`), for long interactive / recipe-driven
/// coding sessions whose think-gaps routinely exceed 5 minutes and let the
/// 5-minute cache go cold. The 1-hour write costs 2× vs the 5-minute 1.25×, so it
/// only pays off above ~1.6× prefix reuse within the hour — see [`CacheTtl`] for
/// the honest break-even. Anything unrecognized keeps the 5-minute default.
pub fn cache_ttl(model_config: &ModelConfig) -> CacheTtl {
    match model_config
        .get_config_param::<String>("cache_ttl", "CLAUDE_CACHE_TTL")
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        Some("1h" | "1hr" | "1hour" | "hour" | "extended" | "long") => CacheTtl::OneHour,
        _ => CacheTtl::FiveMinute,
    }
}

/// Anthropic's floor for an extended-thinking budget; a smaller one is rejected.
const MIN_THINKING_BUDGET_TOKENS: i32 = 1_024;
/// Output kept for the visible answer when a thinking budget is fitted inside
/// the authorised ceiling. Without it a tight ceiling would buy thinking and
/// leave no room to say anything.
const MIN_VISIBLE_OUTPUT_TOKENS: i32 = 512;

/// Fit an extended-thinking budget INSIDE an authorised output ceiling.
///
/// Anthropic counts thinking tokens against `max_tokens`, so both this format
/// and the Databricks one used to buy room by sending `max_tokens + budget` —
/// which walks past the ceiling the cost router priced. `max_tokens` is
/// `model_config.max_output_tokens()`, and on a reserved call that IS the
/// reservation's cap, so a call authorised for 4,096 output tokens asked the
/// API for 20,096 at the default budget: the reservation under-priced it 5x and
/// the task/session ceilings could be passed without tripping.
///
/// `None` means the ceiling cannot hold the minimum budget plus a visible
/// answer, and the caller must send NO thinking block rather than raise the
/// ceiling — a turn that thinks less is recoverable, one that silently
/// overspends is not. Shared so the two Claude-speaking formats cannot drift.
pub(crate) fn fit_thinking_budget_within_ceiling(
    max_tokens: i32,
    requested_budget: i32,
) -> Option<i32> {
    let requested = requested_budget.max(MIN_THINKING_BUDGET_TOKENS);
    let affordable = max_tokens.saturating_sub(MIN_VISIBLE_OUTPUT_TOKENS);
    if affordable < MIN_THINKING_BUDGET_TOKENS {
        return None;
    }
    Some(requested.min(affordable))
}

fn apply_thinking_config(payload: &mut Value, model_config: &ModelConfig, max_tokens: i32) {
    let obj = payload.as_object_mut().unwrap();
    match thinking_type(model_config) {
        ThinkingType::Adaptive => {
            obj.insert("thinking".to_string(), json!({"type": "adaptive"}));
            let effort = thinking_effort(model_config).to_string();
            obj.insert("output_config".to_string(), json!({"effort": effort}));
        }
        ThinkingType::Enabled => {
            // Anthropic counts thinking tokens against `max_tokens`, so extended
            // thinking needs room for the budget PLUS the visible answer. This
            // used to buy that room by sending `max_tokens + budget_tokens` —
            // which walks straight past the ceiling the cost router priced.
            // `max_tokens` here is `model_config.max_output_tokens()`, and on a
            // reserved call that IS the reservation's cap (see
            // `cost_router::reservation` and `apply_reserved_output_cap`): a call
            // authorised for 4,096 output tokens asked the API for 20,096 at the
            // default budget, so the reservation under-priced it by 5x and the
            // task/session ceilings could be passed without ever tripping.
            //
            // The priced ceiling wins. The budget is fitted INSIDE it, leaving
            // `MIN_VISIBLE_OUTPUT_TOKENS` for the answer itself; if the ceiling
            // cannot hold Anthropic's 1,024-token minimum budget plus that floor,
            // thinking is dropped rather than the ceiling broken — a turn that
            // thinks less is recoverable, a turn that silently overspends is not.
            let requested = model_config
                .get_config_param::<i32>("budget_tokens", "CLAUDE_THINKING_BUDGET")
                .unwrap_or(16000);
            let Some(budget_tokens) = fit_thinking_budget_within_ceiling(max_tokens, requested)
            else {
                tracing::warn!(
                    target: "permagent::cost_router",
                    max_tokens,
                    requested_budget = requested,
                    "extended thinking skipped: the authorised output ceiling cannot hold \
                     the minimum thinking budget plus a visible answer"
                );
                return;
            };
            // `max_tokens` is deliberately NOT rewritten: it already carries the
            // priced ceiling, and the budget now lives inside it.
            obj.insert(
                "thinking".to_string(),
                json!({
                    "type": "enabled",
                    "budget_tokens": budget_tokens
                }),
            );
        }
        ThinkingType::Disabled => {}
    }
}

/// Build an Anthropic `/v1/messages` payload with the default cache discipline:
/// no pinned read-only context and the 5-minute cache TTL. The cache-tuned
/// entrypoint — used to pin read-only context or select the 1-hour TTL — is
/// [`create_request_cached`]; this delegates to it and is byte-identical to the
/// pre-cache-last-mile builder.
pub fn create_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<Value> {
    create_request_cached(
        model_config,
        system,
        messages,
        tools,
        &[],
        CacheTtl::FiveMinute,
    )
}

/// Build an Anthropic `/v1/messages` payload, pinning `read_only_files` into the
/// cached system block (after the repo-map, inside the existing system
/// breakpoint) and writing every cache breakpoint at `ttl`. Passing an empty
/// `read_only_files` with [`CacheTtl::FiveMinute`] reproduces [`create_request`]
/// byte-for-byte.
pub fn create_request_cached(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
    read_only_files: &[ReadOnlyFile],
    ttl: CacheTtl,
) -> Result<Value> {
    create_request_split(
        model_config,
        &SystemPromptParts::all_stable(system.to_string()),
        messages,
        tools,
        read_only_files,
        ttl,
    )
}

/// [`create_request_cached`] with the system prompt split into its byte-stable
/// prefix and its turn-volatile suffix, so only the prefix carries the
/// `cache_control` breakpoint. Passing an all-stable prompt reproduces
/// [`create_request_cached`] byte-for-byte.
pub fn create_request_split(
    model_config: &ModelConfig,
    system: &SystemPromptParts,
    messages: &[Message],
    tools: &[Tool],
    read_only_files: &[ReadOnlyFile],
    ttl: CacheTtl,
) -> Result<Value> {
    let mut anthropic_messages = format_messages(messages);
    let tool_specs = format_tools(tools);

    if anthropic_messages.is_empty() {
        return Err(anyhow!("No valid messages to send to Anthropic API"));
    }

    // The volatile suffix rides at the END of the request whenever the request
    // ends on a user message (every agent-loop turn does); only a request that
    // ends on something else keeps it as the unmarked second system block.
    let relocated = !system.volatile_suffix().is_empty()
        && append_turn_context_to_last_user(&mut anthropic_messages, system.volatile_suffix());
    let stable_only;
    let system_for_spec = if relocated {
        stable_only = SystemPromptParts::all_stable(system.stable_prefix().to_string());
        &stable_only
    } else {
        system
    };
    let system_spec = format_system_parts(system_for_spec, read_only_files);

    let max_tokens = model_config.max_output_tokens();
    let mut payload = json!({
        "model": model_config.model_name,
        "messages": anthropic_messages,
        "max_tokens": max_tokens,
    });

    // Emit the system block whenever there is either a base prompt or pinned
    // read-only context — read-only context alone must still be sent (and cached).
    if !system_for_spec.is_empty() || !read_only_files.is_empty() {
        // (`is_empty` covers both halves — a prompt that is only a volatile
        // suffix still has to be sent.)
        payload
            .as_object_mut()
            .unwrap()
            .insert("system".to_string(), json!(system_spec));
    }

    if !tool_specs.is_empty() {
        payload
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(tool_specs));
    }

    if let Some(temp) = model_config.temperature {
        payload
            .as_object_mut()
            .unwrap()
            .insert("temperature".to_string(), json!(temp));
    }

    apply_thinking_config(&mut payload, model_config, max_tokens);
    // Stamp the selected TTL onto every breakpoint the formatters emitted. This
    // only re-annotates existing `ephemeral` markers — it never adds one — so the
    // ≤4-breakpoint cap holds for both TTLs.
    apply_cache_ttl(&mut payload, ttl);

    Ok(payload)
}

/// Process streaming response from Anthropic's API
pub fn response_to_streaming_message<S>(
    mut stream: S,
) -> impl futures::Stream<
    Item = anyhow::Result<(
        Option<Message>,
        Option<crate::providers::base::ProviderUsage>,
    )>,
> + 'static
where
    S: futures::Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    use async_stream::try_stream;
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug)]
    struct StreamingEvent {
        #[serde(rename = "type")]
        event_type: String,
        #[serde(flatten)]
        data: Value,
    }

    #[derive(Deserialize, Debug)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(clippy::enum_variant_names)]
    enum ContentDelta {
        TextDelta { text: String },
        InputJsonDelta { partial_json: String },
        ThinkingDelta { thinking: String },
        SignatureDelta { signature: String },
    }

    struct ThinkingState {
        text: String,
        signature: String,
    }

    try_stream! {
        let mut accumulated_tool_calls: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
        let mut current_tool_id: Option<String> = None;
        let mut final_usage: Option<crate::providers::base::ProviderUsage> = None;
        let mut message_id: Option<String> = None;
        let mut thinking: Option<ThinkingState> = None;

        while let Some(line_result) = stream.next().await {
            let line = line_result?;

            // Skip empty lines and non-data lines
            // Note: SSE spec allows both "data: value" and "data:value" (space is optional)
            if line.trim().is_empty() || !line.starts_with("data:") {
                continue;
            }

            let data_part = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")).unwrap_or(&line);

            // Handle end of stream
            if data_part.trim() == "[DONE]" {
                break;
            }

            // Parse the JSON event
            let event: StreamingEvent = match serde_json::from_str(data_part) {
                Ok(event) => event,
                Err(e) => {
                    tracing::debug!("Failed to parse streaming event: {} - Line: {}", e, data_part);
                    continue;
                }
            };

            match event.event_type.as_str() {
                EVENT_MESSAGE_START => {
                    if let Some(message_data) = event.data.get("message") {
                        if let Some(id) = message_data.get("id").and_then(|v| v.as_str()) {
                            message_id = Some(id.to_string());
                        }

                        if let Some(usage_data) = message_data.get("usage") {
                            let usage = get_usage(usage_data).unwrap_or_default();
                            let model = message_data.get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            final_usage = Some(crate::providers::base::ProviderUsage::new(model, usage));
                        }
                    }
                    continue;
                }
                EVENT_CONTENT_BLOCK_START => {
                    if let Some(content_block) = event.data.get("content_block") {
                        match content_block.get(TYPE_FIELD).and_then(|v| v.as_str()) {
                            Some(TOOL_USE_TYPE) => {
                                if let Some(id) = content_block.get("id").and_then(|v| v.as_str()) {
                                    current_tool_id = Some(id.to_string());
                                    if let Some(name) = content_block.get("name").and_then(|v| v.as_str()) {
                                        accumulated_tool_calls.insert(id.to_string(), (name.to_string(), String::new()));
                                    }
                                }
                            }
                            Some(THINKING_TYPE) => {
                                thinking = Some(ThinkingState {
                                    text: String::new(),
                                    signature: String::new(),
                                });
                            }
                            Some(REDACTED_THINKING_TYPE) => {
                                if let Some(data) = content_block.get(DATA_FIELD).and_then(|d| d.as_str()) {
                                    let mut message = Message::assistant()
                                        .with_redacted_thinking(data);
                                    message.id = message_id.clone();
                                    yield (Some(message), None);
                                } else {
                                    tracing::warn!("redacted_thinking block missing '{}' field", DATA_FIELD);
                                }
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                EVENT_CONTENT_BLOCK_DELTA => {
                    if let Some(delta) = event.data.get("delta") {
                        match serde_json::from_value::<ContentDelta>(delta.clone()) {
                            Ok(ContentDelta::TextDelta { text }) => {
                                let mut message = Message::assistant().with_text(&text);
                                message.id = message_id.clone();
                                yield (Some(message), None);
                            }
                            Ok(ContentDelta::InputJsonDelta { partial_json }) => {
                                if let Some(tool_id) = &current_tool_id {
                                    if let Some((_name, args)) = accumulated_tool_calls.get_mut(tool_id) {
                                        args.push_str(&partial_json);
                                    }
                                }
                            }
                            Ok(ContentDelta::ThinkingDelta { thinking: t }) => {
                                if let Some(ref mut state) = thinking {
                                    state.text.push_str(&t);
                                }
                            }
                            Ok(ContentDelta::SignatureDelta { signature: s }) => {
                                if let Some(ref mut state) = thinking {
                                    state.signature.push_str(&s);
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Unknown content_block_delta type: {}", e);
                            }
                        }
                    }
                    continue;
                }
                EVENT_CONTENT_BLOCK_STOP => {
                    if let Some(state) = thinking.take() {
                        if !state.text.is_empty() {
                            let mut message = Message::assistant()
                                .with_thinking(state.text, state.signature);
                            message.id = message_id.clone();
                            yield (Some(message), None);
                        }
                    }
                    if let Some(tool_id) = current_tool_id.take() {
                        // Tool call finished, yield complete tool call
                        if let Some((name, args)) = accumulated_tool_calls.remove(&tool_id) {
                            let parsed_args = if args.is_empty() {
                                json!({})
                            } else {
                                match serde_json::from_str::<Value>(&args) {
                                    Ok(parsed) => parsed,
                                    Err(_) => {
                                        // If parsing fails, create an error tool request
                                        let error = ErrorData::new(
                                            ErrorCode::INVALID_PARAMS,
                                            format!("Could not parse tool arguments: {}", args),
                                            None,
                                        );
                                        let mut message = Message::new(
                                            Role::Assistant,
                                            chrono::Utc::now().timestamp(),
                                            vec![MessageContent::tool_request(tool_id, Err(error))],
                                        );
                                        message.id = message_id.clone();
                                        yield (Some(message), None);
                                        continue;
                                    }
                                }
                            };

                            let tool_call = CallToolRequestParams::new(name).with_arguments(object(parsed_args));

                            let mut message = Message::new(
                                rmcp::model::Role::Assistant,
                                chrono::Utc::now().timestamp(),
                                vec![MessageContent::tool_request(tool_id, Ok(tool_call))],
                            );
                            message.id = message_id.clone();
                            yield (Some(message), None);
                        }
                    }
                    continue;
                }
                EVENT_MESSAGE_DELTA => {
                    if let Some(usage_data) = event.data.get("usage") {
                        let delta_usage = get_usage(usage_data).unwrap_or_default();

                        if let Some(existing_usage) = &final_usage {
                            let merged_input = existing_usage.usage.input_tokens.or(delta_usage.input_tokens);
                            let merged_output = delta_usage.output_tokens.or(existing_usage.usage.output_tokens);
                            let merged_total = match (merged_input, merged_output) {
                                (Some(input), Some(output)) => Some(input + output),
                                (Some(input), None) => Some(input),
                                (None, Some(output)) => Some(output),
                                (None, None) => None,
                            };

                            // `message_start` carries Anthropic's input/cache
                            // breakdown; the trailing `message_delta` normally
                            // carries only the final output count. Preserve the
                            // cache split when rebuilding Usage here or the
                            // ledger prices every cached token as fresh input.
                            let merged_cache_read = existing_usage
                                .usage
                                .cache_read_input_tokens
                                .or(delta_usage.cache_read_input_tokens);
                            let merged_cache_write = existing_usage
                                .usage
                                .cache_write_input_tokens
                                .or(delta_usage.cache_write_input_tokens);
                            let merged_usage = crate::providers::base::Usage::new(
                                merged_input,
                                merged_output,
                                merged_total,
                            )
                            .with_cache_tokens(merged_cache_read, merged_cache_write);
                            final_usage = Some(crate::providers::base::ProviderUsage::new(existing_usage.model.clone(), merged_usage));
                        } else {
                            let model = event.data.get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            final_usage = Some(crate::providers::base::ProviderUsage::new(model, delta_usage));
                        }
                    }
                    continue;
                }
                EVENT_MESSAGE_STOP => {
                    if let Some(usage_data) = event.data.get("usage") {
                        let usage = get_usage(usage_data).unwrap_or_default();
                        let model = event.data.get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        // `message_stop` is sparse on Anthropic: it commonly
                        // carries output only, while message_start carried the
                        // input/cache breakdown. Preserve every known field
                        // rather than replacing that authoritative snapshot.
                        final_usage = Some(match final_usage.take() {
                            Some(existing) => {
                                let input = existing.usage.input_tokens.or(usage.input_tokens);
                                let output = usage.output_tokens.or(existing.usage.output_tokens);
                                let total = match (input, output) {
                                    (Some(input), Some(output)) => {
                                        Some(input.saturating_add(output))
                                    }
                                    _ => usage.total_tokens.or(existing.usage.total_tokens),
                                };
                                let merged = crate::providers::base::Usage::new(
                                    input,
                                    output,
                                    total,
                                )
                                .with_cache_tokens(
                                    existing.usage.cache_read_input_tokens
                                        .or(usage.cache_read_input_tokens),
                                    existing.usage.cache_write_input_tokens
                                        .or(usage.cache_write_input_tokens),
                                );
                                let model = if model == "unknown" {
                                    existing.model
                                } else {
                                    model
                                };
                                crate::providers::base::ProviderUsage::new(model, merged)
                            }
                            None => crate::providers::base::ProviderUsage::new(model, usage),
                        });
                    }
                    break;
                }
                _ => {
                    // Unknown event type, log and continue
                    tracing::debug!("Unknown streaming event type: {}", event.event_type);
                    continue;
                }
            }
        }

        if let Some(usage) = final_usage {
            yield (None, Some(usage));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use rmcp::object;
    use serde_json::json;

    #[test]
    fn test_parse_text_response() -> Result<()> {
        let response = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Hello! How can I assist you today?"
            }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 12,
                "output_tokens": 15,
                "cache_creation_input_tokens": 12,
                "cache_read_input_tokens": 0
            }
        });

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;

        if let MessageContent::Text(text) = &message.content[0] {
            assert_eq!(text.text, "Hello! How can I assist you today?");
        } else {
            panic!("Expected Text content");
        }

        assert_eq!(usage.input_tokens, Some(24)); // 12 + 12 = 24 actual tokens
        assert_eq!(usage.output_tokens, Some(15));
        assert_eq!(usage.total_tokens, Some(39)); // 24 + 15

        Ok(())
    }

    #[test]
    fn test_parse_tool_response() -> Result<()> {
        let response = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tool_1",
                "name": "calculator",
                "input": {
                    "expression": "2 + 2"
                }
            }],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 15,
                "output_tokens": 20,
                "cache_creation_input_tokens": 15,
                "cache_read_input_tokens": 0,
            }
        });

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;

        if let MessageContent::ToolRequest(tool_request) = &message.content[0] {
            let tool_call = tool_request.tool_call.as_ref().unwrap();
            assert_eq!(tool_call.name, "calculator");
            assert_eq!(tool_call.arguments, Some(object!({"expression": "2 + 2"})));
        } else {
            panic!("Expected ToolRequest content");
        }

        assert_eq!(usage.input_tokens, Some(30)); // 15 + 15 = 30 actual tokens
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(50)); // 30 + 20

        Ok(())
    }

    #[test]
    fn test_message_to_anthropic_spec() {
        let messages = vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi there"),
            Message::user().with_text("How are you?"),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 3);
        assert_eq!(spec[0]["role"], "user");
        assert_eq!(spec[0]["content"][0]["type"], "text");
        assert_eq!(spec[0]["content"][0]["text"], "Hello");
        assert_eq!(spec[1]["role"], "assistant");
        assert_eq!(spec[1]["content"][0]["text"], "Hi there");
        assert_eq!(spec[2]["role"], "user");
        assert_eq!(spec[2]["content"][0]["text"], "How are you?");
    }

    #[test]
    fn test_message_to_anthropic_spec_skips_unsigned_thinking() {
        let messages = vec![
            Message::assistant().with_content(MessageContent::thinking("internal", "")),
            Message::assistant().with_text("Hi there"),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "assistant");
        assert_eq!(spec[0]["content"][0]["type"], "text");
        assert_eq!(spec[0]["content"][0]["text"], "Hi there");
    }

    #[test]
    fn test_tools_to_anthropic_spec() {
        let tools = vec![
            Tool::new(
                "calculator",
                "Calculate mathematical expressions",
                object!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "The mathematical expression to evaluate"
                        }
                    }
                }),
            ),
            Tool::new(
                "weather",
                "Get weather information",
                object!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The location to get weather for"
                        }
                    }
                }),
            ),
        ];

        let spec = format_tools(&tools);

        assert_eq!(spec.len(), 2);
        assert_eq!(spec[0]["name"], "calculator");
        assert_eq!(spec[0]["description"], "Calculate mathematical expressions");
        assert_eq!(spec[1]["name"], "weather");
        assert_eq!(spec[1]["description"], "Get weather information");

        // Verify cache control is added to last tool
        assert!(spec[1].get("cache_control").is_some());
    }

    #[test]
    fn test_system_to_anthropic_spec() {
        let system = "You are a helpful assistant.";
        let spec = format_system(system);

        assert!(spec.is_array());
        let spec_array = spec.as_array().unwrap();
        assert_eq!(spec_array.len(), 1);
        assert_eq!(spec_array[0]["type"], "text");
        assert_eq!(spec_array[0]["text"], system);
        assert!(spec_array[0].get("cache_control").is_some());
    }

    #[test]
    fn test_cache_pricing_calculation() -> Result<()> {
        // Test realistic cache scenario: small fresh input, large cached content
        let response = json!({
            "id": "msg_cache_test",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Based on the cached context, here's my response."
            }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 7,        // Small fresh input
                "output_tokens": 50,      // Output tokens
                "cache_creation_input_tokens": 10000, // Large cache creation
                "cache_read_input_tokens": 5000       // Large cache read
            }
        });

        let usage = get_usage(&response)?;

        // ACTUAL input tokens should be:
        // 7 + 10000 + 5000 = 15007 total actual tokens
        assert_eq!(usage.input_tokens, Some(15007));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(15057)); // 15007 + 50

        // The cache read/creation split is now carried onto the ledger (it used
        // to be folded away), so hit-rate and cache savings are measurable. read
        // = cache_read_input_tokens, write = cache_creation_input_tokens.
        assert_eq!(usage.cache_read_input_tokens, Some(5000));
        assert_eq!(usage.cache_write_input_tokens, Some(10000));

        Ok(())
    }

    #[test]
    fn test_create_request_adaptive_thinking_for_46_models() -> Result<()> {
        let _guard = env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", Some("adaptive")),
            ("CLAUDE_THINKING_EFFORT", Some("high")),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
        ]);

        let mut config = cfg("claude-opus-4-6");
        config.max_tokens = Some(4096);
        let messages = vec![Message::user().with_text("Hello")];
        let payload = create_request(&config, "system", &messages, &[])?;

        assert_eq!(payload["thinking"]["type"], "adaptive");
        assert_eq!(payload["output_config"]["effort"], "high");
        assert!(payload.get("budget_tokens").is_none());

        Ok(())
    }

    #[test]
    fn test_create_request_enabled_thinking_with_budget() -> Result<()> {
        let _guard = env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", None::<&str>),
            ("CLAUDE_THINKING_EFFORT", None::<&str>),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
            ("CLAUDE_THINKING_BUDGET", None::<&str>),
        ]);

        let mut params = std::collections::HashMap::new();
        params.insert("thinking_type".to_string(), json!("enabled"));
        params.insert("budget_tokens".to_string(), json!(10000));

        let mut config = cfg("claude-3-7-sonnet-20250219");
        config.max_tokens = Some(4096);
        config.request_params = Some(params);

        let messages = vec![Message::user().with_text("Hello")];
        let payload = create_request(&config, "system", &messages, &[])?;

        assert_eq!(payload["thinking"]["type"], "enabled");
        // The 10,000-token request is fitted INSIDE the 4,096 ceiling rather
        // than added to it: 4096 - 512 = 3584. Before this fix the payload
        // asked for 14,096 output tokens on a call priced for 4,096.
        assert_eq!(payload["thinking"]["budget_tokens"], 3584);
        assert_eq!(
            payload["max_tokens"], 4096,
            "max_tokens must stay at the authorised ceiling"
        );

        Ok(())
    }

    /// The reservation cap is the whole point: whatever thinking asks for, the
    /// payload may never request more output than the call was priced for.
    #[test]
    fn thinking_never_raises_max_tokens_above_the_authorised_ceiling() -> Result<()> {
        let _guard = env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", None::<&str>),
            ("CLAUDE_THINKING_EFFORT", None::<&str>),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
            ("CLAUDE_THINKING_BUDGET", None::<&str>),
        ]);

        for (ceiling, want_thinking, want_budget) in [
            // Roomy ceiling: the full request fits untouched.
            (32_000, true, Some(10_000)),
            // Tight ceiling: the budget shrinks to fit, answer room preserved.
            (4_096, true, Some(3_584)),
            // Too tight for the 1,024 minimum plus 512 of answer: no thinking.
            (1_400, false, None),
        ] {
            let mut params = std::collections::HashMap::new();
            params.insert("thinking_type".to_string(), json!("enabled"));
            params.insert("budget_tokens".to_string(), json!(10000));

            let mut config = cfg("claude-3-7-sonnet-20250219");
            config.max_tokens = Some(ceiling);
            config.request_params = Some(params);

            let messages = vec![Message::user().with_text("Hello")];
            let payload = create_request(&config, "system", &messages, &[])?;

            assert_eq!(
                payload["max_tokens"], ceiling,
                "ceiling {ceiling} must be preserved exactly"
            );
            if want_thinking {
                assert_eq!(payload["thinking"]["type"], "enabled", "ceiling {ceiling}");
                assert_eq!(
                    payload["thinking"]["budget_tokens"],
                    want_budget.unwrap(),
                    "ceiling {ceiling}"
                );
            } else {
                assert!(
                    payload.get("thinking").is_none(),
                    "ceiling {ceiling} cannot afford thinking, so none may be requested"
                );
            }
        }

        Ok(())
    }

    #[test]
    fn test_create_request_disabled_thinking_no_thinking_field() -> Result<()> {
        let _guard = env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", None::<&str>),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
        ]);

        let config = cfg("claude-sonnet-4-20250514");
        let messages = vec![Message::user().with_text("Hello")];
        let payload = create_request(&config, "system", &messages, &[])?;

        assert!(payload.get("thinking").is_none());
        assert!(payload.get("output_config").is_none());

        Ok(())
    }

    #[test]
    fn test_tool_error_handling_maintains_pairing() {
        use crate::conversation::message::Message;
        use rmcp::model::{ErrorCode, ErrorData};

        let messages = vec![
            Message::assistant().with_tool_request(
                "tool_1",
                Ok(CallToolRequestParams::new("calculator")
                    .with_arguments(object!({"expression": "2 + 2"}))),
            ),
            Message::user().with_tool_response(
                "tool_1",
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Tool failed".to_string(),
                    None,
                )),
            ),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 2);

        assert_eq!(spec[0]["role"], "assistant");
        assert_eq!(spec[0]["content"][0]["type"], "tool_use");
        assert_eq!(spec[0]["content"][0]["id"], "tool_1");
        assert_eq!(spec[0]["content"][0]["name"], "calculator");

        assert_eq!(spec[1]["role"], "user");
        assert_eq!(spec[1]["content"][0]["type"], "tool_result");
        assert_eq!(spec[1]["content"][0]["tool_use_id"], "tool_1");
        assert_eq!(
            spec[1]["content"][0]["content"],
            "Error: -32603: Tool failed"
        );
        assert_eq!(spec[1]["content"][0]["is_error"], true);
    }

    #[test]
    fn test_whitespace_only_text_blocks_are_skipped() {
        let messages = vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("").with_tool_request(
                "tool_1",
                Ok(CallToolRequestParams::new("search").with_arguments(object!({"query": "test"}))),
            ),
            Message::user()
                .with_tool_response("tool_1", Ok(rmcp::model::CallToolResult::success(vec![]))),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 3);

        let assistant_content = spec[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 1);
        assert_eq!(assistant_content[0]["type"], "tool_use");
    }

    #[test]
    fn test_tool_response_with_resource_content() {
        use rmcp::model::{CallToolResult, Content};

        let resource_content = Content::embedded_text(
            "file:///test/file.txt",
            "This is the file content from a resource",
        );

        let messages = vec![
            Message::assistant().with_tool_request(
                "tool_1",
                Ok(CallToolRequestParams::new("view_file")
                    .with_arguments(object!({"path": "/test/file.txt"}))),
            ),
            Message::user().with_tool_response(
                "tool_1",
                Ok(CallToolResult::success(vec![resource_content])),
            ),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 2);
        assert_eq!(spec[1]["role"], "user");
        assert_eq!(spec[1]["content"][0]["type"], "tool_result");
        assert_eq!(spec[1]["content"][0]["tool_use_id"], "tool_1");
        assert_eq!(
            spec[1]["content"][0]["content"],
            "This is the file content from a resource"
        );
    }

    #[test]
    fn test_tool_response_with_mixed_content() {
        use rmcp::model::{CallToolResult, Content};

        let text_content = Content::text("Summary: file loaded");
        let resource_content = Content::embedded_text("file:///test/file.txt", "File content here");

        let messages = vec![
            Message::assistant().with_tool_request(
                "tool_1",
                Ok(CallToolRequestParams::new("view_file")
                    .with_arguments(object!({"path": "/test/file.txt"}))),
            ),
            Message::user().with_tool_response(
                "tool_1",
                Ok(CallToolResult::success(vec![
                    text_content,
                    resource_content,
                ])),
            ),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec[1]["content"][0]["type"], "tool_result");
        assert_eq!(
            spec[1]["content"][0]["content"],
            "Summary: file loaded\nFile content here"
        );
    }

    fn cfg(name: &str) -> ModelConfig {
        ModelConfig {
            model_name: name.to_string(),
            ..Default::default()
        }
    }

    fn cfg_with_thinking(name: &str, tt: &str) -> ModelConfig {
        let mut params = std::collections::HashMap::new();
        params.insert("thinking_type".to_string(), json!(tt));
        ModelConfig {
            model_name: name.to_string(),
            request_params: Some(params),
            ..Default::default()
        }
    }

    #[test]
    fn test_thinking_type_explicit_params() {
        assert_eq!(
            thinking_type(&cfg_with_thinking("claude-opus-4-6", "adaptive")),
            ThinkingType::Adaptive
        );
        assert_eq!(
            thinking_type(&cfg_with_thinking("claude-opus-4-6", "disabled")),
            ThinkingType::Disabled
        );
        assert_eq!(
            thinking_type(&cfg_with_thinking("claude-3-7-sonnet-20250219", "enabled")),
            ThinkingType::Enabled
        );
        assert_eq!(
            thinking_type(&cfg_with_thinking("claude-3-7-sonnet-20250219", "adaptive")),
            ThinkingType::Disabled
        );
        assert_eq!(
            thinking_type(&cfg_with_thinking("claude-opus-4-6", "adapttive")),
            ThinkingType::Disabled
        );
    }

    #[test]
    fn test_thinking_type_non_claude_always_disabled() {
        assert_eq!(thinking_type(&cfg("gpt-4o")), ThinkingType::Disabled);
        assert_eq!(
            thinking_type(&cfg_with_thinking("gpt-4o", "enabled")),
            ThinkingType::Disabled
        );
    }

    #[test]
    fn test_thinking_type_env_var_override() {
        let _guard = env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", Some("adaptive")),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
        ]);
        assert_eq!(
            thinking_type(&cfg("claude-opus-4-6")),
            ThinkingType::Adaptive
        );
        assert_eq!(
            thinking_type(&cfg("claude-3-7-sonnet-20250219")),
            ThinkingType::Disabled
        );
    }

    #[derive(Default)]
    struct StreamedParts {
        thinking: Vec<(String, String)>,
        redacted_thinking: Vec<String>,
        text: Vec<String>,
        tool_calls: Vec<String>,
        usage: Option<crate::providers::base::ProviderUsage>,
    }

    async fn collect_stream(events: &str) -> StreamedParts {
        use futures::StreamExt;

        let lines: Vec<Result<String, anyhow::Error>> =
            events.lines().map(|l| Ok(l.to_string())).collect();
        let stream = Box::pin(futures::stream::iter(lines));
        let mut msg_stream = std::pin::pin!(response_to_streaming_message(stream));
        let mut parts = StreamedParts::default();

        while let Some(Ok((message, usage))) = msg_stream.next().await {
            if usage.is_some() {
                parts.usage = usage;
            }
            if let Some(msg) = message {
                for c in &msg.content {
                    match c {
                        MessageContent::Thinking(t) => {
                            parts
                                .thinking
                                .push((t.thinking.clone(), t.signature.clone()));
                        }
                        MessageContent::RedactedThinking(r) => {
                            parts.redacted_thinking.push(r.data.clone());
                        }
                        MessageContent::Text(t) => {
                            parts.text.push(t.text.clone());
                        }
                        MessageContent::ToolRequest(req) => {
                            if let Ok(call) = &req.tool_call {
                                parts.tool_calls.push(call.name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        parts
    }

    #[tokio::test]
    async fn test_streaming_thinking_and_text() {
        let events = concat!(
            r#"data: {"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"model":"claude-opus-4-6","usage":{"input_tokens":10,"output_tokens":0}}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me analyze"}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" this problem."}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig_abc"}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"123"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Here is the answer."}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":1}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":25}}"#,
            "\n",
            r#"data: {"type":"message_stop"}"#,
        );

        let parts = collect_stream(events).await;
        assert_eq!(parts.thinking.len(), 1);
        assert_eq!(parts.thinking[0].0, "Let me analyze this problem.");
        assert_eq!(parts.thinking[0].1, "sig_abc123");
        assert_eq!(parts.text, vec!["Here is the answer."]);
    }

    #[tokio::test]
    async fn streaming_message_delta_preserves_cache_usage_from_message_start() {
        let events = concat!(
            r#"data: {"type":"message_start","message":{"id":"msg_cache","role":"assistant","content":[],"model":"claude-haiku-4-5-20251001","usage":{"input_tokens":7,"output_tokens":0,"cache_creation_input_tokens":10000,"cache_read_input_tokens":5000}}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done."}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":50}}"#,
            "\n",
            r#"data: {"type":"message_stop"}"#,
        );

        let parts = collect_stream(events).await;
        let usage = parts.usage.expect("stream should yield final usage").usage;
        assert_eq!(usage.input_tokens, Some(15007));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(15057));
        assert_eq!(usage.cache_read_input_tokens, Some(5000));
        assert_eq!(usage.cache_write_input_tokens, Some(10000));
    }

    #[tokio::test]
    async fn sparse_message_stop_keeps_start_input_and_cache_usage() {
        let events = concat!(
            r#"data: {"type":"message_start","message":{"id":"msg_sparse","role":"assistant","content":[],"model":"claude-haiku-4-5","usage":{"input_tokens":12,"cache_read_input_tokens":30}}}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":8}}"#,
            "\n",
            r#"data: {"type":"message_stop","usage":{"output_tokens":8}}"#,
        );

        let parts = collect_stream(events).await;
        let usage = parts.usage.expect("stream should yield final usage").usage;
        assert_eq!(usage.input_tokens, Some(42));
        assert_eq!(usage.output_tokens, Some(8));
        assert_eq!(usage.total_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, Some(30));
    }

    #[tokio::test]
    async fn test_streaming_redacted_thinking() {
        let events = concat!(
            r#"data: {"type":"message_start","message":{"id":"msg_2","role":"assistant","content":[],"model":"claude-opus-4-6","usage":{"input_tokens":5,"output_tokens":0}}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque_base64_data"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Done."}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":1}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":10}}"#,
            "\n",
            r#"data: {"type":"message_stop"}"#,
        );

        let parts = collect_stream(events).await;
        assert_eq!(parts.redacted_thinking, vec!["opaque_base64_data"]);
        assert_eq!(parts.text, vec!["Done."]);
    }

    #[tokio::test]
    async fn test_streaming_thinking_text_then_tool_call() {
        let events = concat!(
            r#"data: {"type":"message_start","message":{"id":"msg_3","role":"assistant","content":[],"model":"claude-sonnet-4-6","usage":{"input_tokens":8,"output_tokens":0}}}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I should search for this."}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"tool_sig_xyz"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":0}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Let me search for that."}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":1}"#,
            "\n",
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tool_1","name":"search","input":{}}}"#,
            "\n",
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"rust\"}"}}"#,
            "\n",
            r#"data: {"type":"content_block_stop","index":2}"#,
            "\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":15}}"#,
            "\n",
            r#"data: {"type":"message_stop"}"#,
        );

        let parts = collect_stream(events).await;
        assert_eq!(parts.thinking.len(), 1);
        assert_eq!(
            parts.thinking[0],
            (
                "I should search for this.".to_string(),
                "tool_sig_xyz".to_string()
            )
        );
        assert_eq!(parts.text, vec!["Let me search for that."]);
        assert_eq!(parts.tool_calls, vec!["search"]);
    }

    // ── Prompt-cache prefix guards (#717 cache discipline; a cost lever) ─────
    //
    // Provider prompt caches are model-scoped and prefix-exact: a hit needs the
    // same model AND a byte-identical leading prefix, and a cached read bills at
    // ~10% of fresh input. These lock in the four cache-preserving invariants of
    // the Anthropic request builder against the canonical prefix policy in
    // `crate::cost_router::cache`, so a later edit can't silently bust the cache
    // by moving a breakpoint, reordering the prefix, or leaking volatile context
    // (the repo-map) into the conversation tail.
    use crate::cost_router::cache::{
        harness_prefix, prefix_is_cache_stable, PrefixSegment, CANONICAL_PREFIX, HARNESS_PREFIX,
    };

    const REPO_MAP_MARKER: &str = "<repo_map>";

    fn cache_guard_tools() -> Vec<Tool> {
        vec![
            Tool::new("alpha_read", "Read a file", object!({ "type": "object" })),
            Tool::new("bravo_edit", "Edit a file", object!({ "type": "object" })),
        ]
    }

    // The system prompt with the repo-map (#720) appended as a system extra,
    // exactly as the session builder assembles it: base prompt, then an
    // "Additional Instructions" section carrying the <repo_map> block.
    fn cache_guard_system() -> String {
        format!(
            "You are the Permagent coding harness.\n\n\
             # Additional Instructions:\n\n\
             {REPO_MAP_MARKER}\nA ranked-tags map of this repo's symbols…</repo_map>"
        )
    }

    // Hold the env lock (thinking config reads process env) so create_request is
    // deterministic and race-free alongside the thinking tests above. Cleared to
    // no thinking; the prefix assertions are independent of the thinking config.
    fn no_thinking_env() -> env_lock::EnvGuard<'static> {
        env_lock::lock_env([
            ("CLAUDE_THINKING_TYPE", None::<&str>),
            ("CLAUDE_THINKING_ENABLED", None::<&str>),
            ("CLAUDE_THINKING_EFFORT", None::<&str>),
        ])
    }

    #[test]
    fn cache_control_breakpoints_land_on_the_stable_prefix() {
        // Tools prefix: the breakpoint is on the LAST tool only, so all tool
        // definitions cache as one block — no per-tool volatility.
        let tools = format_tools(&cache_guard_tools());
        assert!(tools.last().unwrap().get("cache_control").is_some());
        for earlier in &tools[..tools.len() - 1] {
            assert!(
                earlier.get("cache_control").is_none(),
                "only the last tool spec anchors the tools prefix"
            );
        }

        // System prefix: the system block carries a breakpoint.
        let system = format_system(&cache_guard_system());
        let blocks = system.as_array().unwrap();
        assert!(
            blocks.last().unwrap().get("cache_control").is_some(),
            "the system block must anchor a cache breakpoint"
        );

        // The volatile tail: user messages carry the incremental-cache markers,
        // but the assistant turn between them (mutable, mid-conversation) must
        // NOT — a breakpoint there would move as the turn grows and bust reads.
        let messages = vec![
            Message::user().with_text("first"),
            Message::assistant().with_text("reply"),
            Message::user().with_text("second"),
        ];
        let formatted = format_messages(&messages);
        for msg in &formatted {
            if msg["role"] == json!("assistant") {
                let has_cc = msg["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c.get("cache_control").is_some());
                assert!(!has_cc, "assistant tail turns must not carry a breakpoint");
            }
        }
    }

    #[test]
    fn repo_map_lives_in_the_cached_prefix_not_the_conversation() {
        // The repo-map rides inside the cached system block…
        let system = format_system(&cache_guard_system());
        let first = &system.as_array().unwrap()[0];
        assert!(first["text"].as_str().unwrap().contains(REPO_MAP_MARKER));
        assert!(first.get("cache_control").is_some());

        // …and is NOT duplicated into any conversation message (which would put
        // it after the cache breakpoint, in the volatile tail — the anti-pattern
        // the #720 placement is designed to avoid).
        let messages = vec![
            Message::user().with_text("Add a unit test"),
            Message::assistant().with_text("On it."),
            Message::user().with_text("Now run verify"),
        ];
        let tail = serde_json::to_string(&format_messages(&messages)).unwrap();
        assert!(
            !tail.contains(REPO_MAP_MARKER),
            "repo-map leaked into the volatile message tail"
        );
    }

    #[test]
    fn cacheable_prefix_does_not_mutate_across_turns() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let system = cache_guard_system();
        let tools = cache_guard_tools();

        // Turn 1: one user message.
        let turn1 = create_request(
            &config,
            &system,
            &[Message::user().with_text("start")],
            &tools,
        )
        .unwrap();
        // Turn 2: same session, the conversation has grown by a round-trip.
        let turn2 = create_request(
            &config,
            &system,
            &[
                Message::user().with_text("start"),
                Message::assistant().with_text("working on it"),
                Message::user().with_text("continue"),
            ],
            &tools,
        )
        .unwrap();

        // The cached prefix (tools + system, repo-map included) is byte-identical
        // across turns — growing the conversation must never rewrite the prefix,
        // or every turn pays a fresh cache write instead of a cheap read.
        assert_eq!(
            turn1["system"], turn2["system"],
            "system prefix drifted across turns"
        );
        assert_eq!(
            turn1["tools"], turn2["tools"],
            "tools prefix drifted across turns"
        );
    }

    // Read-only context, exactly as a session would pin it: CLAUDE.md-type
    // project context plus a pinned source file. Stable order (never mutated),
    // so the cached prefix keeps hitting.
    fn cache_guard_read_only_files() -> Vec<ReadOnlyFile> {
        vec![
            ReadOnlyFile::new("CLAUDE.md", "Project rules: run cargo fmt before pushing."),
            ReadOnlyFile::new("src/lib.rs", "pub fn public_api() {}"),
        ]
    }

    // Discover, from the emitted payload, which prefix segments actually carry a
    // cache breakpoint and in what order — the runtime realization checked
    // against the #717 policy in `crate::cost_router::cache`.
    fn observe_prefix_segments(payload: &Value) -> Vec<PrefixSegment> {
        let mut observed = Vec::new();
        if payload["tools"]
            .as_array()
            .and_then(|t| t.last())
            .and_then(|t| t.get("cache_control"))
            .is_some()
        {
            observed.push(PrefixSegment::Tools);
        }
        if payload["system"]
            .as_array()
            .and_then(|s| s.last())
            .and_then(|b| b.get("cache_control"))
            .is_some()
        {
            observed.push(PrefixSegment::System);
            let sys_text = payload["system"][0]["text"].as_str().unwrap_or_default();
            // Repo-map, then read-only files, both *inside* the System block.
            if sys_text.contains(REPO_MAP_MARKER) {
                observed.push(PrefixSegment::RepoMap);
            }
            if sys_text.contains(READ_ONLY_FILES_OPEN) {
                observed.push(PrefixSegment::ReadOnlyFiles);
            }
        }
        observed
    }

    // Every `cache_control` marker anywhere in the payload — one per breakpoint.
    fn collect_cache_controls(payload: &Value) -> Vec<Value> {
        fn walk(value: &Value, out: &mut Vec<Value>) {
            match value {
                Value::Object(map) => {
                    if let Some(cc) = map.get("cache_control") {
                        out.push(cc.clone());
                    }
                    for v in map.values() {
                        walk(v, out);
                    }
                }
                Value::Array(arr) => {
                    for v in arr {
                        walk(v, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(payload, &mut out);
        out
    }

    #[test]
    fn real_prefix_realizes_the_canonical_cache_policy() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let payload = create_request(
            &config,
            &cache_guard_system(),
            &[Message::user().with_text("go")],
            &cache_guard_tools(),
        )
        .unwrap();

        // With no pinned read-only context the real emission is exactly the base
        // harness prefix, is cache-stable, and every segment is canonical.
        let observed = observe_prefix_segments(&payload);
        assert_eq!(observed, HARNESS_PREFIX.to_vec());
        assert_eq!(observed, harness_prefix(false).to_vec());
        assert!(prefix_is_cache_stable(&observed));
        assert!(observed.iter().all(|s| CANONICAL_PREFIX.contains(s)));
    }

    #[test]
    fn read_only_context_fills_the_slot_and_realizes_the_full_canonical_prefix() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let payload = create_request_cached(
            &config,
            &cache_guard_system(),
            &[Message::user().with_text("go")],
            &cache_guard_tools(),
            &cache_guard_read_only_files(),
            CacheTtl::FiveMinute,
        )
        .unwrap();

        // Pinning read-only context fills the reserved ReadOnlyFiles slot: the
        // realized prefix is now the FULL canonical prefix, still in canonical
        // order and cache-stable — the #717 policy for the read-only-present state.
        let observed = observe_prefix_segments(&payload);
        assert_eq!(observed, harness_prefix(true).to_vec());
        assert_eq!(observed, CANONICAL_PREFIX.to_vec());
        assert!(prefix_is_cache_stable(&observed));

        // The read-only slot rides *after* the repo-map, inside the system block.
        let sys_text = payload["system"][0]["text"].as_str().unwrap();
        let map_at = sys_text.find(REPO_MAP_MARKER).unwrap();
        let ro_at = sys_text.find(READ_ONLY_FILES_OPEN).unwrap();
        assert!(
            map_at < ro_at,
            "read-only context must sit after the repo-map"
        );
    }

    #[test]
    fn read_only_context_lands_in_the_cached_block_not_the_tail() {
        // Folds into the single cached system block…
        let system =
            format_system_with_read_only(&cache_guard_system(), &cache_guard_read_only_files());
        let blocks = system.as_array().unwrap();
        assert_eq!(
            blocks.len(),
            1,
            "read-only must not add a second system block"
        );
        let block = &blocks[0];
        let text = block["text"].as_str().unwrap();
        assert!(text.contains(READ_ONLY_FILES_OPEN));
        assert!(text.contains("Project rules"));
        assert!(
            block.get("cache_control").is_some(),
            "the read-only-bearing system block must anchor a cache breakpoint"
        );

        // …and is NOT duplicated into the volatile message tail (which sits after
        // the cache breakpoint — the anti-pattern the pinning is meant to fix).
        let messages = vec![
            Message::user().with_text("Add a test"),
            Message::assistant().with_text("On it."),
            Message::user().with_text("Now run verify"),
        ];
        let tail = serde_json::to_string(&format_messages(&messages)).unwrap();
        assert!(
            !tail.contains(READ_ONLY_FILES_OPEN),
            "read-only marker leaked into the volatile message tail"
        );
        assert!(
            !tail.contains("Project rules"),
            "read-only content leaked into the volatile message tail"
        );
    }

    #[test]
    fn cacheable_prefix_with_read_only_is_byte_stable_across_turns() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let system = cache_guard_system();
        let tools = cache_guard_tools();
        let read_only = cache_guard_read_only_files();

        // Turn 1: one user message.
        let turn1 = create_request_cached(
            &config,
            &system,
            &[Message::user().with_text("start")],
            &tools,
            &read_only,
            CacheTtl::FiveMinute,
        )
        .unwrap();
        // Turn 2: same session, the conversation has grown by a round-trip.
        let turn2 = create_request_cached(
            &config,
            &system,
            &[
                Message::user().with_text("start"),
                Message::assistant().with_text("working on it"),
                Message::user().with_text("continue"),
            ],
            &tools,
            &read_only,
            CacheTtl::FiveMinute,
        )
        .unwrap();

        // The cached prefix (tools + system, repo-map AND read-only files included)
        // is byte-identical across turns — pinning read-only context must never
        // rewrite the warm prefix as the conversation grows.
        assert_eq!(
            turn1["system"], turn2["system"],
            "system prefix drifted with read-only pinned"
        );
        assert_eq!(turn1["tools"], turn2["tools"], "tools prefix drifted");
        // …and the read-only content is actually present in that stable prefix.
        assert!(turn1["system"][0]["text"]
            .as_str()
            .unwrap()
            .contains(READ_ONLY_FILES_OPEN));
    }

    /// The split's payload contract: the volatile suffix becomes a SECOND system
    /// block that carries no breakpoint, so rewriting it every turn leaves the
    /// cached first block untouched.
    #[test]
    fn only_the_stable_prefix_carries_the_system_breakpoint() {
        let parts = SystemPromptParts::new(
            cache_guard_system(),
            "\n# Live Status\n- 3 pending\n".into(),
        );
        let spec = format_system_parts(&parts, &[]);
        let blocks = spec.as_array().unwrap();

        assert_eq!(
            blocks.len(),
            2,
            "expected a stable block and a volatile one"
        );
        assert!(
            blocks[0].get("cache_control").is_some(),
            "the stable prefix must anchor the breakpoint"
        );
        assert!(
            blocks[1].get("cache_control").is_none(),
            "a breakpoint on the volatile block would pay the 1.25x write premium \
             every turn for a read that can never hit"
        );
        assert_eq!(blocks[1]["text"], json!("\n# Live Status\n- 3 pending\n"));
        // The volatile half is behind the breakpoint, not deleted.
        assert!(!blocks[0]["text"].as_str().unwrap().contains("3 pending"));
    }

    /// An all-stable prompt must produce exactly the pre-split payload — the
    /// split is opt-in, not a silent rewrite of every provider's request. Pinned
    /// against the literal expected shape rather than against `format_system`,
    /// which now shares the same code path and would make this tautological.
    #[test]
    fn an_all_stable_prompt_is_the_single_block_payload() {
        let system = cache_guard_system();
        let spec = format_system_parts(&SystemPromptParts::all_stable(system.clone()), &[]);
        assert_eq!(
            spec,
            json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }]),
            "an all-stable prompt must be exactly one cached text block"
        );
        // …and with read-only context pinned, still ONE block (the read-only
        // content folds into the cached block, never a second one).
        let with_ro = format_system_parts(
            &SystemPromptParts::all_stable(system),
            &cache_guard_read_only_files(),
        );
        assert_eq!(with_ro.as_array().unwrap().len(), 1);
        assert!(with_ro[0]["text"]
            .as_str()
            .unwrap()
            .contains(READ_ONLY_FILES_OPEN));
    }

    /// The payoff, at the payload level: a turn whose live status changed still
    /// sends a byte-identical cached block. Before the split, the same change
    /// rewrote the single system block and discarded the whole warm prefix.
    #[test]
    fn a_changed_volatile_suffix_leaves_the_cached_block_untouched() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let tools = cache_guard_tools();
        let stable = cache_guard_system();

        let request = |volatile: &str| {
            create_request_split(
                &config,
                &SystemPromptParts::new(stable.clone(), volatile.to_string()),
                &[Message::user().with_text("go")],
                &tools,
                &[],
                CacheTtl::FiveMinute,
            )
            .unwrap()
        };

        let turn1 = request("\n# Live Status\n- 0 described, 0 pending\n");
        let turn2 = request("\n# Live Status\n- 4 described, 12 pending\n");

        assert_eq!(
            turn1["system"], turn2["system"],
            "the cached system block must not move when only live status changed"
        );
        // The request ends on a user message, so the volatile suffix rides at
        // the tail of that message (after the history), not in `system`.
        let tail = |payload: &Value| {
            payload["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .clone()
        };
        assert_ne!(tail(&turn1), tail(&turn2));
        assert!(tail(&turn2)["text"]
            .as_str()
            .unwrap()
            .contains("12 pending"));
        assert_eq!(turn1["tools"], turn2["tools"]);

        // Guard the reason this works: exactly one breakpoint in the system spec.
        let breakpoints = turn2["system"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b.get("cache_control").is_some())
            .count();
        assert_eq!(breakpoints, 1);
    }

    #[test]
    fn one_hour_ttl_is_set_when_configured_and_five_minute_otherwise() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let messages = [Message::user().with_text("go")];
        let tools = cache_guard_tools();
        let system = cache_guard_system();

        // Default (5-minute): every breakpoint is the bare `ephemeral` marker,
        // no `ttl` field.
        let five = create_request_cached(
            &config,
            &system,
            &messages,
            &tools,
            &[],
            CacheTtl::FiveMinute,
        )
        .unwrap();
        let five_markers = collect_cache_controls(&five);
        assert!(!five_markers.is_empty());
        for cc in &five_markers {
            assert_eq!(cc["type"], "ephemeral");
            assert!(
                cc.get("ttl").is_none(),
                "the 5-minute TTL must not carry a ttl field"
            );
        }

        // 1-hour: every breakpoint carries `"ttl":"1h"`, and the breakpoint COUNT
        // is unchanged (only re-annotated, never added).
        let hour =
            create_request_cached(&config, &system, &messages, &tools, &[], CacheTtl::OneHour)
                .unwrap();
        let hour_markers = collect_cache_controls(&hour);
        assert_eq!(
            hour_markers.len(),
            five_markers.len(),
            "the 1-hour TTL must not change the breakpoint count"
        );
        for cc in &hour_markers {
            assert_eq!(cc["type"], "ephemeral");
            assert_eq!(cc["ttl"], "1h", "the 1-hour TTL must set ttl=1h everywhere");
        }
    }

    #[test]
    fn breakpoint_count_never_exceeds_the_anthropic_cap() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        // Two user turns => the two rolling read breakpoints, plus tools + system.
        let messages = [
            Message::user().with_text("first"),
            Message::assistant().with_text("reply"),
            Message::user().with_text("second"),
        ];
        let tools = cache_guard_tools();
        let system = cache_guard_system();

        // Across both TTLs and with/without read-only pinned, the payload never
        // exceeds Anthropic's 4-breakpoint cap — read-only folds into the system
        // breakpoint and the TTL only re-annotates existing markers.
        for ttl in [CacheTtl::FiveMinute, CacheTtl::OneHour] {
            for read_only in [Vec::new(), cache_guard_read_only_files()] {
                let payload =
                    create_request_cached(&config, &system, &messages, &tools, &read_only, ttl)
                        .unwrap();
                let n = collect_cache_controls(&payload).len();
                assert!(
                    n <= 4,
                    "breakpoint count {n} exceeds Anthropic's 4-cap (ttl={ttl:?}, read_only={})",
                    read_only.len()
                );
                // Exactly the four canonical breakpoints: last tool, the system
                // block, and the two rolling user-message reads.
                assert_eq!(n, 4, "expected exactly 4 canonical breakpoints, got {n}");
            }
        }
    }

    /// Recursively drop every `cache_control` marker: the markers say where to
    /// write/read, they are not part of the cached content being matched.
    fn strip_cache_control(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(k, _)| k.as_str() != "cache_control")
                    .map(|(k, v)| (k.clone(), strip_cache_control(v)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(strip_cache_control).collect()),
            other => other.clone(),
        }
    }

    /// (message index, block index) of the LAST cache breakpoint in `messages`.
    fn last_message_breakpoint(payload: &Value) -> (usize, usize) {
        let mut found = None;
        for (m, message) in payload["messages"].as_array().unwrap().iter().enumerate() {
            if let Some(blocks) = message["content"].as_array() {
                for (b, block) in blocks.iter().enumerate() {
                    if block.get("cache_control").is_some() {
                        found = Some((m, b));
                    }
                }
            }
        }
        found.expect("the request carries a message-level breakpoint")
    }

    /// The bytes a message-level breakpoint caches: tools, system and every
    /// message block up to and including the breakpoint block.
    fn cached_prefix_at(payload: &Value, (m, b): (usize, usize)) -> Value {
        let messages = payload["messages"].as_array().unwrap();
        let mut prefix: Vec<Value> = messages[..m].to_vec();
        let mut last = messages[m].clone();
        let kept: Vec<Value> = last["content"].as_array().unwrap()[..=b].to_vec();
        last["content"] = Value::Array(kept);
        prefix.push(last);
        strip_cache_control(&json!({
            "tools": payload["tools"],
            "system": payload["system"],
            "messages": prefix,
        }))
    }

    fn moim(minute: u32) -> String {
        format!(
            "<info-msg>\nIt is currently 2026-09-25 11:{minute:02}:00\nWorking directory: /w\n\
             Context: ~{minute}k/400k tokens used\n</info-msg>"
        )
    }

    fn tool_turn(id: &str) -> [Message; 2] {
        [
            Message::assistant()
                .with_tool_request(id, Ok(CallToolRequestParams::new("developer__shell"))),
            Message::user().with_tool_response(
                id,
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text(format!("output of {id}")),
                ])),
            ),
        ]
    }

    /// The measured defect (sessions 20260925_5/_6, 20260924_12): every turn
    /// read only the ~8.9k-token tools+system prefix and re-wrote the whole
    /// history. Two consecutive turns — with a changed clock in MOIM AND a
    /// changed live status — must let turn N+1 read everything turn N's last
    /// breakpoint cached.
    #[test]
    fn consecutive_turns_share_the_whole_history_prefix() {
        let _env = no_thinking_env();
        let config = cfg("claude-haiku-4-5");
        let tools = cache_guard_tools();
        let stable = cache_guard_system();

        // Turn N: the agent loop appends MOIM to the newest user message.
        let mut history = vec![Message::user().with_text("fix the bug")];
        history.extend(tool_turn("t1"));
        let mut turn_n = history.clone();
        let last = turn_n.pop().unwrap();
        turn_n.push(last.with_text(moim(1)));

        // Turn N+1: history grew by one tool round; MOIM moved to the new tail.
        history.extend(tool_turn("t2"));
        let mut turn_n1 = history.clone();
        let last = turn_n1.pop().unwrap();
        turn_n1.push(last.with_text(moim(2)));

        let req = |messages: &[Message], status: &str| {
            create_request_split(
                &config,
                &SystemPromptParts::new(stable.clone(), status.to_string()),
                messages,
                &tools,
                &[],
                CacheTtl::FiveMinute,
            )
            .unwrap()
        };
        let a = req(&turn_n, "\n# Live Status (this turn)\n- worker running\n");
        let b = req(&turn_n1, "\n# Live Status (this turn)\n- worker finished\n");

        // Turn N's last breakpoint is on the tool result, not on MOIM or the
        // relocated status…
        let bp = last_message_breakpoint(&a);
        let block = &a["messages"][bp.0]["content"][bp.1];
        assert_eq!(block["type"], "tool_result", "breakpoint landed on {block}");

        // …and turn N+1 repeats that exact prefix and marks the same block, so
        // the read hits instead of re-writing the history.
        assert_eq!(
            cached_prefix_at(&a, bp),
            cached_prefix_at(&b, bp),
            "turn N+1 must replay turn N's cached prefix byte-for-byte"
        );
        assert!(
            b["messages"][bp.0]["content"][bp.1]
                .get("cache_control")
                .is_some(),
            "turn N+1 must carry a breakpoint on turn N's last cached block"
        );

        // Neither per-turn block ever carries a breakpoint.
        for payload in [&a, &b] {
            for message in payload["messages"].as_array().unwrap() {
                for block in message["content"].as_array().unwrap() {
                    if is_turn_context_block(block) {
                        assert!(block.get("cache_control").is_none(), "{block}");
                    }
                }
            }
            assert_eq!(collect_cache_controls(payload).len(), 4);
        }
    }

    /// The agent loop's tool-round shape: MOIM rides as its OWN user message
    /// after the tool results (inside the result message it broke the OpenAI
    /// chat format). The API folds the two user messages into one turn with the
    /// `tool_result` first; the MOIM-only message has no anchor and must not use
    /// up one of the two rolling breakpoints, or turn N+1 stops marking turn
    /// N's last cached block.
    #[test]
    fn a_moim_only_tail_message_does_not_use_up_a_rolling_breakpoint() {
        let _env = no_thinking_env();
        let config = cfg("claude-haiku-4-5");
        let tools = cache_guard_tools();
        let stable = cache_guard_system();

        let mut history = vec![Message::user().with_text("fix the bug")];
        history.extend(tool_turn("t1"));
        let mut turn_n = history.clone();
        turn_n.push(Message::user().with_text(moim(1)));
        history.extend(tool_turn("t2"));
        let mut turn_n1 = history.clone();
        turn_n1.push(Message::user().with_text(moim(2)));

        let req = |messages: &[Message], status: &str| {
            create_request_split(
                &config,
                &SystemPromptParts::new(stable.clone(), status.to_string()),
                messages,
                &tools,
                &[],
                CacheTtl::FiveMinute,
            )
            .unwrap()
        };
        let a = req(&turn_n, "\n# Live Status (this turn)\n- worker running\n");
        let b = req(&turn_n1, "\n# Live Status (this turn)\n- worker finished\n");

        // The tool results keep their own message, tool_result first; the tail
        // message is per-turn context only (MOIM plus the relocated status).
        let messages = a["messages"].as_array().unwrap();
        let tail = messages.last().unwrap();
        assert_eq!(tail["role"], "user");
        assert!(tail["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(is_turn_context_block));
        assert_eq!(
            messages[messages.len() - 2]["content"][0]["type"],
            "tool_result"
        );

        let bp = last_message_breakpoint(&a);
        let block = &a["messages"][bp.0]["content"][bp.1];
        assert_eq!(block["type"], "tool_result", "breakpoint landed on {block}");
        assert_eq!(
            cached_prefix_at(&a, bp),
            cached_prefix_at(&b, bp),
            "turn N+1 must replay turn N's cached prefix byte-for-byte"
        );
        assert!(
            b["messages"][bp.0]["content"][bp.1]
                .get("cache_control")
                .is_some(),
            "turn N+1 must carry a breakpoint on turn N's last cached block"
        );
        for payload in [&a, &b] {
            assert_eq!(collect_cache_controls(payload).len(), 4);
        }
    }

    /// A request that does not end on a user message cannot take a trailing
    /// block, so the volatile suffix stays the unmarked second system block.
    #[test]
    fn volatile_suffix_falls_back_to_system_when_the_request_ends_on_assistant() {
        let _env = no_thinking_env();
        let config = cfg("claude-opus-4-8");
        let payload = create_request_split(
            &config,
            &SystemPromptParts::new(cache_guard_system(), "\n# Live Status\n- 1\n".into()),
            &[
                Message::user().with_text("go"),
                Message::assistant().with_text("prefill"),
            ],
            &cache_guard_tools(),
            &[],
            CacheTtl::FiveMinute,
        )
        .unwrap();
        let system = payload["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert!(system[1].get("cache_control").is_none());
    }

    #[test]
    fn cache_ttl_opts_into_one_hour_only_when_configured() {
        let config = cfg("claude-opus-4-8");
        // Default: 5-minute everywhere.
        {
            let _g = env_lock::lock_env([("CLAUDE_CACHE_TTL", None::<&str>)]);
            assert_eq!(cache_ttl(&config), CacheTtl::FiveMinute);
        }
        // Explicit 1-hour opt-in (a few accepted spellings).
        for value in ["1h", "hour", "extended"] {
            let _g = env_lock::lock_env([("CLAUDE_CACHE_TTL", Some(value))]);
            assert_eq!(cache_ttl(&config), CacheTtl::OneHour, "value={value}");
        }
        // Anything unrecognized keeps the safe 5-minute default.
        {
            let _g = env_lock::lock_env([("CLAUDE_CACHE_TTL", Some("nonsense"))]);
            assert_eq!(cache_ttl(&config), CacheTtl::FiveMinute);
        }
    }
}
