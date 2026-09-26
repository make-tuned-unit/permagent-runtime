use crate::conversation::message::{Message, MessageContent};
use crate::mcp_utils::extract_text_from_resource;
use crate::model::ModelConfig;
use crate::providers::base::{ProviderUsage, Usage};
use crate::providers::utils::extract_reasoning_effort;
use anyhow::{anyhow, Error};
use async_stream::try_stream;
use chrono;
use futures::Stream;
use rmcp::model::{object, CallToolRequestParams, RawContent, Role, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ops::Deref;

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponsesApiResponse {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
}

/// Why a response stopped early, on a response whose `status` is
/// `"incomplete"` (for example `{"reason": "max_output_tokens"}`).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct IncompleteDetails {
    #[serde(default)]
    pub reason: Option<String>,
}

/// The `incomplete_details.reason` the Responses API gives when the output
/// cap ran out. For a reasoning model this is usually a response holding
/// only a reasoning item: the cap was spent thinking before any text or tool
/// call was written.
pub const INCOMPLETE_MAX_OUTPUT_TOKENS: &str = "max_output_tokens";

/// The reason a response is incomplete, or `None` for a finished one. An
/// incomplete response with no stated reason reports `"unknown"`, so it is
/// never mistaken for a normal completion.
pub fn incomplete_reason(status: &str, details: Option<&IncompleteDetails>) -> Option<String> {
    if status != "incomplete" {
        return None;
    }
    Some(
        details
            .and_then(|d| d.reason.clone())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "unknown".to_string()),
    )
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub struct SummaryText {
    pub text: String,
}

fn reasoning_from_summary(summary: &[SummaryText]) -> Option<MessageContent> {
    let text: String = summary
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        None
    } else {
        Some(MessageContent::thinking(text, ""))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItem {
    Reasoning {
        id: String,
        #[serde(default)]
        summary: Vec<SummaryText>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ResponseContentBlock>,
    },
    FunctionCall {
        id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseContentBlock {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseReasoningInfo {
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
    // OpenAI reports the cache-read subset of `input_tokens` here, not as a
    // sibling of it — see `input_tokens_details.cached_tokens` in the
    // Responses API reference. Optional because older responses (and other
    // OpenAI-compatible hosts speaking this same shape) omit the object
    // entirely; that ABSENCE must stay distinguishable from an explicit
    // `cached_tokens: 0`, which is why this is `Option`, not a bare `i32`
    // defaulted to zero. Measured on the live ledger since 2026-09-01: every
    // other cache-reporting provider (anthropic, custom_deepseek, zai,
    // minimax) shows a real cache-hit rate while openai showed 0% across 87
    // calls — because this field was never read, not because caching never
    // hit. Coercing "unknown" to zero here would keep that blind spot and
    // additionally mis-price cached input at the full fresh-input rate
    // (`crates/goose/src/providers/canonical/cost.rs::cost_breakdown` only
    // carves out the cheaper cache-read rate when this is populated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<ResponseInputTokensDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseInputTokensDetails {
    // `Option` for the same reason as above: a present `input_tokens_details`
    // object with the `cached_tokens` key missing (rather than the whole
    // object being absent) should still read as "unknown", not "0".
    #[serde(default)]
    pub cached_tokens: Option<i32>,
}

/// The one place that turns a Responses-API `usage` block into our internal
/// `Usage`. Both the non-streaming path (`get_responses_usage`) and the
/// streaming `ResponseCompleted` branch used to build `Usage` independently
/// and neither read the cache breakdown — that duplication is exactly how the
/// gap went unnoticed on one path while the other was "fixed". Route both
/// through here so they can't drift apart again.
fn usage_from_response_usage(usage: &ResponseUsage) -> Usage {
    let cache_read_input_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);

    // No `cache_write_input_tokens` here: the Responses API (like OpenAI's
    // Chat Completions) has no write-premium field — prompt caching there is
    // automatic and reads are the only billed-differently category. Leaving
    // this `None` matches `formats/openai.rs::get_usage`, which does the same
    // for the same reason.
    Usage::new(
        Some(usage.input_tokens),
        Some(usage.output_tokens),
        Some(usage.total_tokens),
    )
    .with_cache_tokens(cache_read_input_tokens, None)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponsesStreamEvent {
    #[serde(rename = "response.created")]
    ResponseCreated {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    /// The terminal event for a response that stopped early — most often the
    /// output cap running out (`incomplete_details.reason ==
    /// "max_output_tokens"`). It carries the same `usage` and `output` as
    /// `response.completed`; before it was handled here it was skipped as an
    /// unknown event, so the call settled with no usage at all and the turn
    /// read as an ordinary, if empty, completion.
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.failed")]
    ResponseFailed { sequence_number: i32, error: Value },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        arguments: String,
    },
    #[serde(rename = "error")]
    Error { error: Value },
    #[serde(rename = "keepalive")]
    Keepalive {
        #[serde(default)]
        sequence_number: Option<i32>,
    },
}

fn is_known_responses_stream_event_type(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.created"
            | "response.in_progress"
            | "response.output_item.added"
            | "response.content_part.added"
            | "response.output_text.delta"
            | "response.output_item.done"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.completed"
            | "response.incomplete"
            | "response.failed"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "error"
            | "keepalive"
    )
}

fn parse_responses_stream_event(data_line: &str) -> anyhow::Result<Option<ResponsesStreamEvent>> {
    let raw_event: Value = serde_json::from_str(data_line).map_err(|e| {
        anyhow!(
            "Failed to parse Responses stream event: {}: {:?}",
            e,
            data_line
        )
    })?;

    let Some(event_type) = raw_event.get("type").and_then(Value::as_str) else {
        return Ok(None);
    };

    if !is_known_responses_stream_event_type(event_type) {
        return Ok(None);
    }

    let event = serde_json::from_value(raw_event).map_err(|e| {
        anyhow!(
            "Failed to parse Responses stream event: {}: {:?}",
            e,
            data_line
        )
    })?;
    Ok(Some(event))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItemInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItemInfo {
    Reasoning {
        id: String,
        #[serde(default)]
        summary: Vec<SummaryText>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ContentPart>,
    },
    FunctionCall {
        id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ContentPart {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
}

fn add_message_items(input_items: &mut Vec<Value>, messages: &[Message]) {
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        let mut text_items = Vec::new();

        for content in &message.content {
            match content {
                MessageContent::Text(text) if !text.text.is_empty() => {
                    let content_type = if message.role == Role::Assistant {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    text_items.push(json!({
                        "type": content_type,
                        "text": text.text
                    }));
                }
                MessageContent::ToolRequest(request) if message.role == Role::Assistant => {
                    if !text_items.is_empty() {
                        input_items.push(json!({
                            "role": role,
                            "content": text_items
                        }));
                        text_items = Vec::new();
                    }

                    if let Ok(tool_call) = &request.tool_call {
                        let arguments_str = tool_call
                            .arguments
                            .as_ref()
                            .map(|args| {
                                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                            })
                            .unwrap_or_else(|| "{}".to_string());

                        tracing::debug!(
                            "Replaying function_call with call_id: {}, name: {}",
                            request.id,
                            tool_call.name
                        );
                        input_items.push(json!({
                            "type": "function_call",
                            "call_id": request.id,
                            "name": tool_call.name,
                            "arguments": arguments_str
                        }));
                    }
                }
                MessageContent::ToolResponse(response) => {
                    if !text_items.is_empty() {
                        input_items.push(json!({
                            "role": role,
                            "content": text_items
                        }));
                        text_items = Vec::new();
                    }

                    match &response.tool_result {
                        Ok(contents) => {
                            let has_images = contents
                                .content
                                .iter()
                                .any(|c| matches!(c.deref(), RawContent::Image(_)));

                            let output = if has_images {
                                json!(contents
                                    .content
                                    .iter()
                                    .map(|c| match c.deref() {
                                        RawContent::Text(t) => json!({
                                            "type": "input_text", "text": t.text
                                        }),
                                        RawContent::Resource(r) => json!({
                                            "type": "input_text",
                                            "text": extract_text_from_resource(&r.resource)
                                        }),
                                        RawContent::Image(image) => json!({
                                            "type": "input_image",
                                            "image_url": format!(
                                                "data:{};base64,{}",
                                                image.mime_type, image.data
                                            )
                                        }),
                                        RawContent::Audio(_) => json!({
                                            "type": "input_text", "text": "[Audio content]"
                                        }),
                                        RawContent::ResourceLink(_) => json!({
                                            "type": "input_text", "text": "[Resource link]"
                                        }),
                                    })
                                    .collect::<Vec<Value>>())
                            } else {
                                json!(contents
                                    .content
                                    .iter()
                                    .filter_map(|c| match c.deref() {
                                        RawContent::Text(t) => Some(t.text.clone()),
                                        RawContent::Resource(r) => {
                                            Some(extract_text_from_resource(&r.resource))
                                        }
                                        RawContent::Audio(_) => Some("[Audio content]".into()),
                                        RawContent::ResourceLink(_) => {
                                            Some("[Resource link]".into())
                                        }
                                        RawContent::Image(_) => None,
                                    })
                                    .collect::<Vec<String>>()
                                    .join("\n"))
                            };

                            input_items.push(json!({
                                "type": "function_call_output",
                                "call_id": response.id,
                                "output": output
                            }));
                        }
                        Err(error_data) => {
                            tracing::debug!(
                                "Sending function_call_output error with call_id: {}",
                                response.id
                            );
                            input_items.push(json!({
                                "type": "function_call_output",
                                "call_id": response.id,
                                "output": format!("Error: {}", error_data.message)
                            }));
                        }
                    }
                }
                _ => {}
            }
        }

        if !text_items.is_empty() {
            input_items.push(json!({
                "role": role,
                "content": text_items
            }));
        }
    }
}

pub fn create_responses_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> anyhow::Result<Value, Error> {
    let mut input_items = Vec::new();

    if !system.is_empty() {
        input_items.push(json!({
            "role": "system",
            "content": [{
                "type": "input_text",
                "text": system
            }]
        }));
    }

    add_message_items(&mut input_items, messages);

    let (model_name, reasoning_effort) = extract_reasoning_effort(&model_config.model_name);
    let is_reasoning_model = reasoning_effort.is_some();

    let mut payload = json!({
        "model": model_name,
        "input": input_items,
        "store": false,
    });

    if let Some(effort) = reasoning_effort {
        payload.as_object_mut().unwrap().insert(
            "reasoning".to_string(),
            json!({
                "effort": effort,
                "summary": "auto",
            }),
        );
    }

    if !tools.is_empty() {
        let tools_spec: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();

        payload
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(tools_spec));
    }

    if !is_reasoning_model {
        if let Some(temp) = model_config.temperature {
            payload
                .as_object_mut()
                .unwrap()
                .insert("temperature".to_string(), json!(temp));
        }
    }

    payload.as_object_mut().unwrap().insert(
        "max_output_tokens".to_string(),
        json!(model_config.max_output_tokens()),
    );

    Ok(payload)
}

/// [`create_responses_request`] for a stable/volatile-split system prompt: the
/// stable prefix leads `input` as before, and the volatile suffix is emitted as
/// a trailing `system` item AFTER the conversation — the same placement
/// `formats::openai::create_request_split` uses for chat/completions.
///
/// OpenAI caches automatically on an exact prefix. Flattened into the leading
/// system item (the old `system.render()` call) the live-status block sat
/// upstream of the entire history, so any change to it — a worker finishing, a
/// briefing read — re-billed every prior turn at the full input rate. At the
/// tail it is never replayed: next turn's request repeats this one up to the
/// start of this item. An empty suffix produces exactly
/// [`create_responses_request`]'s payload.
pub fn create_responses_request_split(
    model_config: &ModelConfig,
    system: &crate::cost_router::cache::SystemPromptParts,
    messages: &[Message],
    tools: &[Tool],
) -> anyhow::Result<Value, Error> {
    let mut payload =
        create_responses_request(model_config, system.stable_prefix(), messages, tools)?;
    if !system.volatile_suffix().is_empty() {
        if let Some(input) = payload.get_mut("input").and_then(Value::as_array_mut) {
            input.push(json!({
                "role": "system",
                "content": [{
                    "type": "input_text",
                    "text": system.volatile_suffix()
                }]
            }));
        }
    }
    Ok(payload)
}

pub fn responses_api_to_message(response: &ResponsesApiResponse) -> anyhow::Result<Message> {
    let mut content = Vec::new();

    for item in &response.output {
        match item {
            ResponseOutputItem::Reasoning { summary, .. } => {
                content.extend(reasoning_from_summary(summary));
            }
            ResponseOutputItem::Message {
                content: msg_content,
                ..
            } => {
                for block in msg_content {
                    match block {
                        ResponseContentBlock::OutputText { text, .. } => {
                            if !text.is_empty() {
                                content.push(MessageContent::text(text));
                            }
                        }
                        ResponseContentBlock::ToolCall { id, name, input } => {
                            content.push(MessageContent::tool_request(
                                id.clone(),
                                Ok(CallToolRequestParams::new(name.clone())
                                    .with_arguments(object(input.clone()))),
                            ));
                        }
                    }
                }
            }
            ResponseOutputItem::FunctionCall {
                id,
                call_id,
                name,
                arguments,
                ..
            } => {
                let request_id = call_id.as_ref().unwrap_or(id).clone();
                let parsed_args = if arguments.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}))
                };

                content.push(MessageContent::tool_request(
                    request_id,
                    Ok(CallToolRequestParams::new(name.clone())
                        .with_arguments(object(parsed_args))),
                ));
            }
        }
    }

    let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);

    message = message.with_id(response.id.clone());

    Ok(message)
}

/// [`incomplete_reason`] for a non-streamed response.
pub fn get_responses_incomplete_reason(response: &ResponsesApiResponse) -> Option<String> {
    incomplete_reason(&response.status, response.incomplete_details.as_ref())
}

pub fn get_responses_usage(response: &ResponsesApiResponse) -> Usage {
    response
        .usage
        .as_ref()
        .map_or_else(Usage::default, usage_from_response_usage)
}

fn process_streaming_output_items(
    output_items: Vec<ResponseOutputItemInfo>,
    is_text_response: bool,
) -> Vec<MessageContent> {
    let mut content = Vec::new();

    for item in output_items {
        match item {
            ResponseOutputItemInfo::Reasoning { summary, .. } => {
                content.extend(reasoning_from_summary(&summary));
            }
            ResponseOutputItemInfo::Message { content: parts, .. } => {
                for part in parts {
                    match part {
                        ContentPart::OutputText { text, .. } => {
                            if !text.is_empty() && !is_text_response {
                                content.push(MessageContent::text(&text));
                            }
                        }
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            let parsed_args = if arguments.is_empty() {
                                json!({})
                            } else {
                                serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}))
                            };

                            content.push(MessageContent::tool_request(
                                id,
                                Ok(CallToolRequestParams::new(name)
                                    .with_arguments(object(parsed_args))),
                            ));
                        }
                    }
                }
            }
            ResponseOutputItemInfo::FunctionCall {
                id,
                call_id,
                name,
                arguments,
                ..
            } => {
                let request_id = call_id.unwrap_or(id);
                let parsed_args = if arguments.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}))
                };

                content.push(MessageContent::tool_request(
                    request_id,
                    Ok(CallToolRequestParams::new(name).with_arguments(object(parsed_args))),
                ));
            }
        }
    }

    content
}

pub fn responses_api_to_streaming_message<S>(
    mut stream: S,
) -> impl Stream<Item = anyhow::Result<(Option<Message>, Option<ProviderUsage>)>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    try_stream! {
        use futures::StreamExt;

        let mut accumulated_text = String::new();
        let mut response_id: Option<String> = None;
        let mut model_name: Option<String> = None;
        let mut final_usage: Option<ProviderUsage> = None;
        let mut output_items: Vec<ResponseOutputItemInfo> = Vec::new();
        let mut is_text_response = false;

        'outer: while let Some(response) = stream.next().await {
            let response_str = response?;

            // Skip empty lines
            if response_str.trim().is_empty() {
                continue;
            }
            if response_str.starts_with(':') {
                continue;
            }

            // Parse SSE format: "event: <type>\ndata: <json>"
            // For now, we only care about the data line
            // SSE spec allows both "data: value" and "data:value" (space after colon is optional)
            let data_line = if response_str.starts_with("data: ") {
                response_str.strip_prefix("data: ").unwrap()
            } else if response_str.starts_with("data:") {
                response_str.strip_prefix("data:").unwrap()
            } else if response_str.starts_with("event: ") || response_str.starts_with("event:") {
                // Skip event type lines
                continue;
            } else {
                // Try to parse as-is when there's no prefix
                &response_str
            };

            if data_line == "[DONE]" {
                break 'outer;
            }

            let Some(event) = parse_responses_stream_event(data_line)? else {
                continue;
            };

            match event {
                ResponsesStreamEvent::ResponseCreated { response, .. } |
                ResponsesStreamEvent::ResponseInProgress { response, .. } => {
                    response_id = Some(response.id);
                    model_name = Some(response.model);
                }

                ResponsesStreamEvent::OutputTextDelta { delta, .. } => {
                    is_text_response = true;
                    if !delta.is_empty() {
                        accumulated_text.push_str(&delta);

                        // Yield incremental text updates for true streaming
                        let mut msg = Message::new(
                            Role::Assistant,
                            chrono::Utc::now().timestamp(),
                            vec![MessageContent::text(&delta)],
                        );

                        // Add ID so desktop client knows these deltas are part of the same message
                        if let Some(id) = &response_id {
                            msg = msg.with_id(id.clone());
                        }

                        yield (Some(msg), None);
                    }
                }

                ResponsesStreamEvent::OutputItemDone { item, .. } => {
                    output_items.push(item);
                }

                ResponsesStreamEvent::OutputTextDone { .. } => {
                    // Text is already complete from deltas, this is just a summary event
                }

                ResponsesStreamEvent::ResponseCompleted { response, .. }
                | ResponsesStreamEvent::ResponseIncomplete { response, .. } => {
                    let model = model_name.as_ref().unwrap_or(&response.model);
                    let usage = response
                        .usage
                        .as_ref()
                        .map_or_else(Usage::default, usage_from_response_usage);
                    let incomplete =
                        incomplete_reason(&response.status, response.incomplete_details.as_ref());
                    if let Some(reason) = incomplete.as_deref() {
                        tracing::warn!(
                            model = %model,
                            reason,
                            output_tokens = ?usage.output_tokens,
                            "Responses API returned an incomplete response"
                        );
                    }
                    final_usage = Some(ProviderUsage {
                        usage,
                        model: model.clone(),
                        invocation_id: None,
                        incomplete_reason: incomplete,
                    });

                    // For complete output, use the response output items
                    if !response.output.is_empty() {
                        output_items = response.output;
                    }

                    break 'outer;
                }

                ResponsesStreamEvent::FunctionCallArgumentsDelta { .. } => {
                    // Function call arguments are being streamed, but we'll get the complete
                    // arguments in the OutputItemDone event, so we can ignore deltas for now
                }

                ResponsesStreamEvent::FunctionCallArgumentsDone { .. } => {
                    // Arguments are complete, will be in the OutputItemDone event
                }

                // Typed by the event's own `code`/`type`, so an in-band rate
                // limit or 5xx reaches the agent as one (with the provider's
                // requested wait) instead of as a dropped stream. The callers'
                // `ProviderError::from_stream_failure` keeps the type.
                ResponsesStreamEvent::ResponseFailed { error, .. } => {
                    Err(anyhow::Error::new(
                        crate::providers::errors::ProviderError::from_stream_error_event(
                            "Responses API failed",
                            &error,
                        ),
                    ))?;
                }

                ResponsesStreamEvent::Error { error } => {
                    Err(anyhow::Error::new(
                        crate::providers::errors::ProviderError::from_stream_error_event(
                            "Responses API error",
                            &error,
                        ),
                    ))?;
                }

                _ => {
                    // Ignore other event types (OutputItemAdded, ContentPartAdded, ContentPartDone)
                }
            }
        }

        // Process final output items and yield usage data
        let content = process_streaming_output_items(output_items, is_text_response);

        if !content.is_empty() {
            let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);
            if let Some(id) = response_id {
                message = message.with_id(id);
            }
            yield (Some(message), final_usage);
        } else if let Some(usage) = final_usage {
            yield (None, Some(usage));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use crate::model::ModelConfig;
    use futures::StreamExt;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;

    #[tokio::test]
    async fn test_responses_stream_ignores_keepalive_event() -> anyhow::Result<()> {
        let lines = vec![
            r#"data: {"type":"response.created","sequence_number":1,"response":{"id":"resp_1","object":"response","created_at":1737368310,"status":"in_progress","model":"gpt-5.2-pro","output":[]}}"#.to_string(),
            r#"data: {"type":"keepalive"}"#.to_string(),
            r#"data: {"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello"}"#.to_string(),
            r#"data: {"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":0,"content_index":0,"delta":" world"}"#.to_string(),
            r#"data: {"type":"response.completed","sequence_number":4,"response":{"id":"resp_1","object":"response","created_at":1737368310,"status":"completed","model":"gpt-5.2-pro","output":[],"usage":{"input_tokens":10,"output_tokens":4,"total_tokens":14}}}"#.to_string(),
            "data: [DONE]".to_string(),
        ];

        let response_stream = tokio_stream::iter(lines.into_iter().map(Ok));
        let messages = responses_api_to_streaming_message(response_stream);
        futures::pin_mut!(messages);

        let mut text_parts = Vec::new();
        let mut usage: Option<ProviderUsage> = None;

        while let Some(item) = messages.next().await {
            let (message, maybe_usage) = item?;
            if let Some(msg) = message {
                for content in msg.content {
                    if let MessageContent::Text(text) = content {
                        text_parts.push(text.text.clone());
                    }
                }
            }
            if let Some(final_usage) = maybe_usage {
                usage = Some(final_usage);
            }
        }

        assert_eq!(text_parts.concat(), "Hello world");
        let usage = usage.expect("usage should be present at completion");
        assert_eq!(usage.model, "gpt-5.2-pro");
        assert_eq!(usage.usage.input_tokens, Some(10));
        assert_eq!(usage.usage.output_tokens, Some(4));
        assert_eq!(usage.usage.total_tokens, Some(14));

        Ok(())
    }

    #[test]
    fn test_responses_api_to_message_captures_reasoning_summary() -> anyhow::Result<()> {
        let response: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1737368310,
            "status": "completed",
            "model": "gpt-5",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "summary": [
                        { "type": "summary_text", "text": "Thinking about the question..." },
                        { "type": "summary_text", "text": "The answer is straightforward." }
                    ]
                },
                {
                    "type": "message",
                    "id": "msg_1",
                    "status": "completed",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "The capital of France is Paris." }
                    ]
                }
            ]
        }))?;

        let message = responses_api_to_message(&response)?;

        let thinking = message.content.iter().find_map(|c| c.as_thinking());
        assert!(thinking.is_some(), "should contain thinking content");
        assert_eq!(
            thinking.unwrap().thinking,
            "Thinking about the question...\nThe answer is straightforward."
        );

        let text = message.content.iter().find_map(|c| c.as_text());
        assert_eq!(text, Some("The capital of France is Paris."));

        Ok(())
    }

    #[tokio::test]
    async fn test_responses_stream_captures_reasoning_summary() -> anyhow::Result<()> {
        let reasoning_item = serde_json::json!({
            "type": "reasoning",
            "id": "rs_1",
            "summary": [
                { "type": "summary_text", "text": "Let me think step by step." }
            ]
        });
        let message_item = serde_json::json!({
            "type": "message",
            "id": "msg_1",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": "Paris." }]
        });

        let lines = vec![
            format!(
                r#"data: {{"type":"response.created","sequence_number":1,"response":{{"id":"resp_1","object":"response","created_at":1737368310,"status":"in_progress","model":"gpt-5","output":[]}}}}"#
            ),
            format!(
                r#"data: {{"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_1","output_index":1,"content_index":0,"delta":"Paris."}}"#
            ),
            format!(
                r#"data: {{"type":"response.output_item.done","sequence_number":3,"output_index":0,"item":{}}}"#,
                serde_json::to_string(&reasoning_item)?
            ),
            format!(
                r#"data: {{"type":"response.output_item.done","sequence_number":4,"output_index":1,"item":{}}}"#,
                serde_json::to_string(&message_item)?
            ),
            format!(
                r#"data: {{"type":"response.completed","sequence_number":5,"response":{{"id":"resp_1","object":"response","created_at":1737368310,"status":"completed","model":"gpt-5","output":[{},{}],"usage":{{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}}}}"#,
                serde_json::to_string(&reasoning_item)?,
                serde_json::to_string(&message_item)?
            ),
            "data: [DONE]".to_string(),
        ];

        let response_stream = tokio_stream::iter(lines.into_iter().map(Ok));
        let messages = responses_api_to_streaming_message(response_stream);
        futures::pin_mut!(messages);

        let mut thinking_parts = Vec::new();
        let mut text_parts = Vec::new();

        while let Some(item) = messages.next().await {
            let (message, _) = item?;
            if let Some(msg) = message {
                for content in msg.content {
                    match &content {
                        MessageContent::Thinking(t) => thinking_parts.push(t.thinking.clone()),
                        MessageContent::Text(t) => text_parts.push(t.text.clone()),
                        _ => {}
                    }
                }
            }
        }

        assert!(
            !thinking_parts.is_empty(),
            "should capture thinking from stream"
        );
        assert_eq!(thinking_parts.join(""), "Let me think step by step.");
        assert!(text_parts.concat().contains("Paris."));

        Ok(())
    }

    #[tokio::test]
    async fn test_responses_stream_error_event_still_returns_error() -> anyhow::Result<()> {
        let lines = vec![
            r#"data: {"type":"error","error":{"message":"boom"}}"#.to_string(),
            "data: [DONE]".to_string(),
        ];

        let response_stream = tokio_stream::iter(lines.into_iter().map(Ok));
        let messages = responses_api_to_streaming_message(response_stream);
        futures::pin_mut!(messages);

        let first = messages
            .next()
            .await
            .expect("stream should emit an error item");

        assert!(first.is_err());
        assert!(first
            .expect_err("expected error")
            .to_string()
            .contains("Responses API error"));

        Ok(())
    }

    #[test]
    fn test_history_preserves_chronological_order() {
        let model_config = ModelConfig {
            model_name: "gpt-5.2-codex".to_string(),
            context_limit: None,
            temperature: None,
            max_tokens: None,
            toolshim: false,
            toolshim_model: None,
            fast_model_config: None,
            request_params: None,
            reasoning: None,
        };

        let messages = vec![
            Message::assistant()
                .with_text("I'll create that file.")
                .with_tool_request(
                    "call_1",
                    Ok(CallToolRequestParams::new("shell")
                        .with_arguments(object!({"command": "echo hello"}))),
                ),
            Message::assistant()
                .with_text("Now let me verify.")
                .with_tool_request(
                    "call_2",
                    Ok(CallToolRequestParams::new("shell")
                        .with_arguments(object!({"command": "cat file.txt"}))),
                ),
        ];

        let result = create_responses_request(&model_config, "", &messages, &[]).unwrap();
        let input = result["input"].as_array().unwrap();

        let types: Vec<&str> = input
            .iter()
            .map(|item| {
                item.get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| item["role"].as_str().unwrap())
            })
            .collect();

        assert_eq!(
            types,
            vec!["assistant", "function_call", "assistant", "function_call"]
        );
    }

    #[test]
    fn test_responses_api_to_message_uses_call_id_for_tool_request_id() {
        let response = ResponsesApiResponse {
            id: "resp_1".to_string(),
            object: "response".to_string(),
            created_at: 0,
            status: "completed".to_string(),
            model: "gpt-5.3-codex".to_string(),
            output: vec![ResponseOutputItem::FunctionCall {
                id: "fc_123".to_string(),
                status: "completed".to_string(),
                call_id: Some("call_abc".to_string()),
                name: "test__get_person_zip_code".to_string(),
                arguments: r#"{"name":"Alice Burns"}"#.to_string(),
            }],
            reasoning: None,
            usage: None,
            incomplete_details: None,
        };

        let message = responses_api_to_message(&response).unwrap();
        assert_eq!(message.content.len(), 1);
        let MessageContent::ToolRequest(tool_request) = &message.content[0] else {
            panic!("expected tool request content");
        };
        assert_eq!(tool_request.id, "call_abc");
    }

    #[test]
    fn test_deserialize_reasoning_info_with_null_effort() {
        let json = r#"{"effort": null}"#;
        let info: ResponseReasoningInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.effort, None);
        assert_eq!(info.summary, None);
    }

    #[test]
    fn test_deserialize_reasoning_info_with_effort() {
        let json = r#"{"effort": "high", "summary": "Thought deeply"}"#;
        let info: ResponseReasoningInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.effort.as_deref(), Some("high"));
        assert_eq!(info.summary.as_deref(), Some("Thought deeply"));
    }

    // Regression coverage for the 0%-cache-hit-rate bug: `ResponseUsage` used
    // to keep only the three plain counts, so `input_tokens_details.cached_tokens`
    // was parsed and then thrown away on both the non-streaming and streaming
    // paths. These tests pin down that the cache split now survives parsing on
    // both paths, that a genuinely absent `input_tokens_details` reads as
    // "unknown" rather than "0", and that the cached portion is billed at the
    // cache-read rate instead of the full input rate.

    #[test]
    fn test_get_responses_usage_preserves_cache_read_tokens() {
        let response: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "model": "gpt-5.4-mini",
            "output": [],
            "usage": {
                "input_tokens": 10000,
                "output_tokens": 200,
                "total_tokens": 10200,
                "input_tokens_details": { "cached_tokens": 8000 }
            }
        }))
        .unwrap();

        let usage = get_responses_usage(&response);
        assert_eq!(usage.input_tokens, Some(10000));
        assert_eq!(usage.output_tokens, Some(200));
        assert_eq!(usage.total_tokens, Some(10200));
        assert_eq!(usage.cache_read_input_tokens, Some(8000));
    }

    #[tokio::test]
    async fn test_responses_stream_completed_preserves_cache_read_tokens() -> anyhow::Result<()> {
        let lines = vec![
            r#"data: {"type":"response.created","sequence_number":1,"response":{"id":"resp_1","object":"response","created_at":0,"status":"in_progress","model":"gpt-5.4-mini","output":[]}}"#.to_string(),
            r#"data: {"type":"response.completed","sequence_number":2,"response":{"id":"resp_1","object":"response","created_at":0,"status":"completed","model":"gpt-5.4-mini","output":[],"usage":{"input_tokens":10000,"output_tokens":200,"total_tokens":10200,"input_tokens_details":{"cached_tokens":8000}}}}"#.to_string(),
            "data: [DONE]".to_string(),
        ];

        let response_stream = tokio_stream::iter(lines.into_iter().map(Ok));
        let messages = responses_api_to_streaming_message(response_stream);
        futures::pin_mut!(messages);

        let mut usage: Option<ProviderUsage> = None;
        while let Some(item) = messages.next().await {
            let (_, maybe_usage) = item?;
            if let Some(final_usage) = maybe_usage {
                usage = Some(final_usage);
            }
        }

        let usage = usage.expect("usage should be present at completion").usage;
        assert_eq!(usage.input_tokens, Some(10000));
        assert_eq!(usage.output_tokens, Some(200));
        assert_eq!(usage.total_tokens, Some(10200));
        assert_eq!(usage.cache_read_input_tokens, Some(8000));

        Ok(())
    }

    /// 2026-09-25: a gpt-5.4-mini worker spent its whole output cap
    /// reasoning. The stream ended with `response.incomplete`, which used to
    /// be skipped as an unknown event: no usage settled, and the turn read as
    /// an ordinary completion holding only a reasoning item.
    #[tokio::test]
    async fn test_responses_stream_incomplete_is_surfaced_with_usage() -> anyhow::Result<()> {
        let lines = vec![
            r#"data: {"type":"response.created","sequence_number":1,"response":{"id":"resp_1","object":"response","created_at":0,"status":"in_progress","model":"gpt-5.4-mini","output":[]}}"#.to_string(),
            r#"data: {"type":"response.output_item.done","sequence_number":2,"output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Considering the event ids"}]}}"#.to_string(),
            r#"data: {"type":"response.incomplete","sequence_number":3,"response":{"id":"resp_1","object":"response","created_at":0,"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"model":"gpt-5.4-mini","output":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Considering the event ids"}]}],"usage":{"input_tokens":27000,"output_tokens":4096,"total_tokens":31096}}}"#.to_string(),
        ];

        let response_stream = tokio_stream::iter(lines.into_iter().map(Ok));
        let messages = responses_api_to_streaming_message(response_stream);
        futures::pin_mut!(messages);

        let mut last_message: Option<Message> = None;
        let mut usage: Option<ProviderUsage> = None;
        while let Some(item) = messages.next().await {
            let (message, maybe_usage) = item?;
            if message.is_some() {
                last_message = message;
            }
            if maybe_usage.is_some() {
                usage = maybe_usage;
            }
        }

        let usage = usage.expect("an incomplete response still reports its usage");
        assert_eq!(
            usage.incomplete_reason.as_deref(),
            Some(INCOMPLETE_MAX_OUTPUT_TOKENS)
        );
        assert_eq!(usage.usage.output_tokens, Some(4096));
        let message = last_message.expect("the reasoning item is still returned");
        assert!(message
            .content
            .iter()
            .all(|c| matches!(c, MessageContent::Thinking(_))));
        Ok(())
    }

    #[test]
    fn test_incomplete_reason_distinguishes_incomplete_from_completed() {
        let incomplete: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 0,
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "model": "gpt-5.4-mini",
            "output": []
        }))
        .unwrap();
        assert_eq!(
            get_responses_incomplete_reason(&incomplete).as_deref(),
            Some("max_output_tokens")
        );

        let completed: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_2",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "model": "gpt-5.4-mini",
            "output": []
        }))
        .unwrap();
        assert_eq!(get_responses_incomplete_reason(&completed), None);

        // Incomplete with no stated reason is still incomplete.
        assert_eq!(
            incomplete_reason("incomplete", None).as_deref(),
            Some("unknown")
        );
    }

    #[test]
    fn test_get_responses_usage_without_details_is_unknown_not_zero() {
        let response: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "model": "gpt-5.4-mini",
            "output": [],
            "usage": {
                "input_tokens": 500,
                "output_tokens": 50,
                "total_tokens": 550
            }
        }))
        .unwrap();

        let usage = get_responses_usage(&response);
        // No `input_tokens_details` at all in this fixture (older responses, or
        // an OpenAI-compatible host that doesn't send it) — the cache read
        // count must come back `None`, not `Some(0)`. Coercing to zero here is
        // exactly how a real cache hit reads as "0% cached" on the ledger.
        assert_eq!(usage.cache_read_input_tokens, None);
    }

    #[test]
    fn test_cached_portion_priced_at_cache_read_rate() {
        use crate::providers::canonical::{cost_breakdown, Pricing};

        let response: ResponsesApiResponse = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "model": "gpt-5.4-mini",
            "output": [],
            "usage": {
                "input_tokens": 10000,
                "output_tokens": 0,
                "total_tokens": 10000,
                "input_tokens_details": { "cached_tokens": 8000 }
            }
        }))
        .unwrap();

        let usage = get_responses_usage(&response);

        // Real published rate for gpt-5.4-mini (USD per 1M tokens): input
        // 0.75, cache read 0.075 — a 10x discount. If the cached count were
        // dropped (or coerced to 0), all 10,000 tokens would bill at the
        // fresh-input rate instead of 2,000 fresh + 8,000 at the cheap rate.
        let pricing = Pricing {
            input: Some(0.75),
            output: Some(4.5),
            cache_read: Some(0.075),
            cache_write: None,
        };

        let breakdown = cost_breakdown(&usage, &pricing).expect("pricing is fully specified");

        let expected_fresh_input_cost = 2000.0 / 1_000_000.0 * 0.75;
        let expected_cache_read_cost = 8000.0 / 1_000_000.0 * 0.075;
        assert!((breakdown.input_cost - expected_fresh_input_cost).abs() < 1e-12);
        assert!((breakdown.cache_read_cost - expected_cache_read_cost).abs() < 1e-12);

        // Sanity: pricing the whole 10,000 tokens at the plain input rate
        // would cost noticeably more than what we actually bill once the
        // cache discount is applied to the 8,000 cached tokens.
        let naive_full_price_cost = 10000.0 / 1_000_000.0 * 0.75;
        assert!(breakdown.total_cost < naive_full_price_cost);
    }

    /// Two consecutive turns whose live status changed: turn N's request minus
    /// its trailing volatile item must be a byte-prefix of turn N+1's request,
    /// so OpenAI's automatic prefix cache covers the whole history.
    #[test]
    fn volatile_suffix_trails_the_conversation_and_keeps_the_prefix_stable() {
        use crate::cost_router::cache::SystemPromptParts;
        let model_config = ModelConfig {
            model_name: "gpt-5.4-mini".to_string(),
            context_limit: None,
            temperature: None,
            max_tokens: None,
            toolshim: false,
            toolshim_model: None,
            fast_model_config: None,
            request_params: None,
            reasoning: None,
        };
        let stable = "You are Henry.\n# Tools\n...".to_string();
        let mut history = vec![Message::user().with_text("instrument the page")];
        let turn_n = history.clone();
        history.push(Message::assistant().with_text("reading the page"));
        history.push(Message::user().with_text("continue"));

        let a = create_responses_request_split(
            &model_config,
            &SystemPromptParts::new(stable.clone(), "\n# Live Status\n- worker running\n".into()),
            &turn_n,
            &[],
        )
        .unwrap();
        let b = create_responses_request_split(
            &model_config,
            &SystemPromptParts::new(stable.clone(), "\n# Live Status\n- worker done\n".into()),
            &history,
            &[],
        )
        .unwrap();

        let a_items = a["input"].as_array().unwrap();
        let b_items = b["input"].as_array().unwrap();
        // The leading system item is the stable prefix alone.
        assert_eq!(a_items[0]["content"][0]["text"], json!(stable));
        assert_eq!(a_items[0], b_items[0]);
        // The volatile block is the LAST item.
        assert!(a_items.last().unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("worker running"));
        // Everything before it reappears unchanged at the head of the next turn.
        let replayed = &a_items[..a_items.len() - 1];
        assert_eq!(&b_items[..replayed.len()], replayed);

        // An empty suffix is exactly the unsplit payload.
        let flat = create_responses_request(&model_config, &stable, &turn_n, &[]).unwrap();
        let split = create_responses_request_split(
            &model_config,
            &SystemPromptParts::all_stable(stable.clone()),
            &turn_n,
            &[],
        )
        .unwrap();
        assert_eq!(flat, split);
    }
}
