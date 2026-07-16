use crate::conversation::message::{ActionRequiredData, MessageMetadata};
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::{merge_consecutive_messages, Conversation};
use crate::prompt_template::render_template;
#[cfg(test)]
use crate::providers::base::{stream_from_single_message, MessageStream};
use crate::providers::base::{Provider, ProviderUsage};
use crate::providers::errors::ProviderError;
use crate::{config::Config, token_counter::create_token_counter};
use anyhow::Result;
use indoc::indoc;
use rmcp::model::Role;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::info;
use tracing::log::warn;

pub const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8;

const TOOLCALL_SUMMARIZATION_BATCH_SIZE: usize = 10;

fn tool_pair_summarization_enabled() -> bool {
    Config::global()
        .get_param::<bool>("GOOSE_TOOL_PAIR_SUMMARIZATION")
        .unwrap_or(true)
}

const CONVERSATION_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context.";

const TOOL_LOOP_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Continue calling tools as necessary to complete the task.";

const MANUAL_COMPACT_CONTINUATION_TEXT: &str =
    "Your context was compacted at the user's request. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context.";

#[derive(Serialize)]
struct SummarizeContext {
    messages: String,
}

// ── Code-safe compaction ────────────────────────────────────────────────
//
// Lossy summarization silently corrupts source code — identifiers and
// whitespace get rewritten by the summarizer model (arXiv 2506.00307). So we
// carve code/diff-bearing content OUT of the summarizer path and preserve it
// byte-for-byte; only logs, search output, thinking, and prose are summarized.
// A compaction must never be able to eat a diff or mangle code — that is the
// load-bearing safety guarantee, independent of any token-savings claim.

/// Tool names whose request arguments or response payloads carry SOURCE CODE or
/// a DIFF verbatim (file writes, edits, reads). Matched against the *unprefixed
/// tail* of the tool name (`developer__edit` -> `edit`), so it is robust to the
/// `extension__tool` name-prefixing the extension manager applies.
///
/// Reads that go through the `shell` tool (e.g. `cat file.rs`) are deliberately
/// NOT listed: `shell` also carries build/test logs that SHOULD be summarized,
/// and a summarized read is recoverable by re-reading, whereas a corrupted diff
/// is silent, load-bearing data loss. The dedicated file tools are the
/// code-safe path.
const CODE_BEARING_TOOLS: &[&str] = &[
    "edit",
    "write",
    "read",
    "file_edit",
    "file_write",
    "file_read",
];

/// Eviction priority for a tool response with no explicit `priority` annotation.
/// Ranked *above* the explicit `0.0` stamped on search/shell/log output
/// (`with_priority(0.0)`), so those low-value responses are evicted first when
/// trimming to fit context.
const UNTAGGED_EVICTION_PRIORITY: f32 = 1.0;

/// A tool name minus any `extension__` prefix (`developer__edit` -> `edit`,
/// `edit` -> `edit`).
fn unprefixed_tool_name(name: &str) -> &str {
    name.rsplit_once("__").map_or(name, |(_, tail)| tail)
}

/// Whether a tool's payload is source code or a diff (see [`CODE_BEARING_TOOLS`]).
fn is_code_bearing_tool(name: &str) -> bool {
    CODE_BEARING_TOOLS.contains(&unprefixed_tool_name(name))
}

/// Ids of tool calls whose arguments/results are code or a diff, so their paired
/// responses can be recognized and preserved alongside the requests.
fn code_bearing_tool_ids(messages: &[Message]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for msg in messages {
        for content in &msg.content {
            if let MessageContent::ToolRequest(req) = content {
                if let Ok(call) = &req.tool_call {
                    if is_code_bearing_tool(&call.name) {
                        ids.insert(req.id.clone());
                    }
                }
            }
        }
    }
    ids
}

/// Whether a message must be preserved VERBATIM (routed around the lossy
/// summarizer): it either issues a code/diff tool call, or carries the response
/// to one.
fn is_code_bearing_message(msg: &Message, code_ids: &HashSet<String>) -> bool {
    msg.content.iter().any(|content| match content {
        MessageContent::ToolRequest(req) => req
            .tool_call
            .as_ref()
            .is_ok_and(|call| is_code_bearing_tool(&call.name)),
        MessageContent::ToolResponse(resp) => code_ids.contains(&resp.id),
        _ => false,
    })
}

/// A message's eviction priority: the lowest `priority` among its tool-response
/// content blocks (an unannotated block counts as [`UNTAGGED_EVICTION_PRIORITY`]).
/// Lower evicts first; a message with no tool-response content returns
/// [`UNTAGGED_EVICTION_PRIORITY`].
fn eviction_priority(msg: &Message) -> f32 {
    msg.content
        .iter()
        .filter_map(|content| match content {
            MessageContent::ToolResponse(resp) => resp.tool_result.as_ref().ok(),
            _ => None,
        })
        .flat_map(|result| result.content.iter())
        .map(|block| block.priority().unwrap_or(UNTAGGED_EVICTION_PRIORITY))
        .fold(UNTAGGED_EVICTION_PRIORITY, f32::min)
}

/// The order in which tool responses are considered for eviction when their
/// priorities tie: middle-out (the middle of the trajectory first), matching the
/// historical behavior so equal-priority conversations compact identically.
fn middle_out_order(tool_indices: &[usize]) -> Vec<usize> {
    let len = tool_indices.len();
    let middle = len / 2;
    let mut ordered = Vec::with_capacity(len);
    let mut i = 0usize;
    while ordered.len() < len {
        if i.is_multiple_of(2) {
            if let Some(m) = middle.checked_sub(i / 2 + 1) {
                ordered.push(tool_indices[m]);
            }
        } else if middle + i / 2 < len {
            ordered.push(tool_indices[middle + i / 2]);
        }
        i += 1;
        if i > 2 * len + 2 {
            break; // safety: the interleaving always fills `len` well before this
        }
    }
    ordered
}

/// Compact messages by summarizing them
///
/// This function performs the actual compaction by summarizing messages and updating
/// their visibility metadata. It does not check thresholds - use `check_if_compaction_needed`
/// first to determine if compaction is necessary.
///
/// # Arguments
/// * `provider` - The provider to use for summarization
/// * `session_id` - The session to use for summarization
/// * `conversation` - The current conversation history
/// * `manual_compact` - If true, this is a manual compaction (don't preserve user message)
///
/// # Returns
/// * A tuple containing:
///   - `Conversation`: The compacted messages
///   - `ProviderUsage`: Provider usage from summarization
pub async fn compact_messages(
    provider: &dyn Provider,
    session_id: &str,
    conversation: &Conversation,
    manual_compact: bool,
) -> Result<(Conversation, ProviderUsage)> {
    info!("Performing message compaction");

    let messages = conversation.messages();

    let has_text_only = |msg: &Message| {
        let has_text = msg
            .content
            .iter()
            .any(|c| matches!(c, MessageContent::Text(_)));
        let has_tool_content = msg.content.iter().any(|c| {
            matches!(
                c,
                MessageContent::ToolRequest(_) | MessageContent::ToolResponse(_)
            )
        });
        has_text && !has_tool_content
    };

    let extract_text = |msg: &Message| -> Option<String> {
        let text_parts: Vec<String> = msg
            .content
            .iter()
            .filter_map(|c| {
                if let MessageContent::Text(text) = c {
                    Some(text.text.clone())
                } else {
                    None
                }
            })
            .collect();

        if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join("\n"))
        }
    };

    // Find and preserve the most recent user message for non-manual compacts
    let (preserved_user_message, is_most_recent) = if !manual_compact {
        let found_msg = messages.iter().enumerate().rev().find(|(_, msg)| {
            msg.is_agent_visible()
                && matches!(msg.role, rmcp::model::Role::User)
                && has_text_only(msg)
        });

        if let Some((idx, msg)) = found_msg {
            let is_last = idx == messages.len() - 1;
            (Some(msg.clone()), is_last)
        } else {
            (None, false)
        }
    } else {
        (None, false)
    };

    let messages_to_compact = messages.as_slice();

    // Structurally route code/diff-bearing content AROUND the lossy summarizer:
    // only the non-code messages (logs, search output, thinking, prose) are
    // summarized. The code/diff messages are re-inserted verbatim below, so a
    // compaction can never corrupt them.
    let code_ids = code_bearing_tool_ids(messages_to_compact);
    let summarizable: Vec<Message> = messages_to_compact
        .iter()
        .filter(|msg| !is_code_bearing_message(msg, &code_ids))
        .cloned()
        .collect();

    let (summary_message, summarization_usage) =
        do_compact(provider, session_id, &summarizable).await?;

    // Create the final message list with updated visibility metadata:
    // 1. Original messages become user_visible but not agent_visible
    // 2. Summary message becomes agent_visible but not user_visible
    // 3. Assistant messages to continue the conversation are also agent_visible but not user_visible
    let mut final_messages = Vec::new();

    for (idx, msg) in messages_to_compact.iter().enumerate() {
        let updated_metadata = if is_most_recent
            && idx == messages_to_compact.len() - 1
            && preserved_user_message.is_some()
        {
            // This is the most recent message and we're preserving it by adding a fresh copy
            MessageMetadata::invisible()
        } else {
            msg.metadata.with_agent_invisible()
        };
        let updated_msg = msg.clone().with_metadata(updated_metadata);
        final_messages.push(updated_msg);
    }

    let summary_msg = summary_message.with_metadata(MessageMetadata::agent_only());

    let mut continuation_messages = vec![summary_msg];

    // Re-insert every code/diff-bearing message VERBATIM into the agent context
    // (agent-only; the user still sees the untouched originals above), in
    // original order so tool request/response pairs stay ordered. This content
    // never passed through the summarizer, so it survives byte-identical — the
    // load-bearing code-safety guarantee.
    for msg in messages_to_compact
        .iter()
        .filter(|msg| msg.is_agent_visible() && is_code_bearing_message(msg, &code_ids))
    {
        continuation_messages.push(msg.clone().with_metadata(MessageMetadata::agent_only()));
    }

    let continuation_text = if manual_compact {
        MANUAL_COMPACT_CONTINUATION_TEXT
    } else if is_most_recent {
        CONVERSATION_CONTINUATION_TEXT
    } else {
        TOOL_LOOP_CONTINUATION_TEXT
    };

    let continuation_msg = Message::assistant()
        .with_text(continuation_text)
        .with_metadata(MessageMetadata::agent_only());
    continuation_messages.push(continuation_msg);

    let (merged_continuation, _issues) = merge_consecutive_messages(continuation_messages);
    final_messages.extend(merged_continuation);

    if let Some(user_msg) = preserved_user_message {
        if let Some(text) = extract_text(&user_msg) {
            final_messages.push(Message::user().with_text(&text));
        }
    }

    Ok((
        Conversation::new_unvalidated(final_messages),
        summarization_usage,
    ))
}

/// Check if messages exceed the auto-compaction threshold
pub async fn check_if_compaction_needed(
    provider: &dyn Provider,
    conversation: &Conversation,
    threshold_override: Option<f64>,
    session: &crate::session::Session,
) -> Result<bool> {
    if provider.manages_own_context() {
        return Ok(false);
    }

    let messages = conversation.messages();
    let config = Config::global();
    let threshold = threshold_override.unwrap_or_else(|| {
        config
            .get_param::<f64>("GOOSE_AUTO_COMPACT_THRESHOLD")
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD)
    });

    let context_limit = provider.get_model_config().context_limit();

    let (current_tokens, _token_source) = match session.total_tokens {
        Some(tokens) => (tokens as usize, "session metadata"),
        None => {
            let token_counter = create_token_counter()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create token counter: {}", e))?;

            let token_counts: Vec<_> = messages
                .iter()
                .filter(|m| m.is_agent_visible())
                .map(|msg| token_counter.count_chat_tokens("", std::slice::from_ref(msg), &[]))
                .collect();

            (token_counts.iter().sum(), "estimated")
        }
    };

    let usage_ratio = current_tokens as f64 / context_limit as f64;

    let needs_compaction = if threshold <= 0.0 || threshold >= 1.0 {
        false // Auto-compact is disabled.
    } else {
        usage_ratio > threshold
    };
    Ok(needs_compaction)
}

fn filter_tool_responses(messages: &[Message], remove_percent: u32) -> Vec<&Message> {
    fn has_tool_response(msg: &Message) -> bool {
        msg.content
            .iter()
            .any(|c| matches!(c, MessageContent::ToolResponse(_)))
    }

    if remove_percent == 0 {
        return messages.iter().collect();
    }

    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, msg)| has_tool_response(msg))
        .map(|(i, _)| i)
        .collect();

    if tool_indices.is_empty() {
        return messages.iter().collect();
    }

    let num_to_remove = ((tool_indices.len() * remove_percent as usize) / 100).max(1);

    // Evict by ASCENDING priority (lowest first) so the search/shell/log output
    // stamped `with_priority(0.0)` is dropped before higher-priority responses;
    // ties keep the historical middle-out order (via a stable sort), so
    // equal-priority conversations compact exactly as before. Code/diff-bearing
    // responses are preserved verbatim upstream and never reach this pool.
    let mut ordered = middle_out_order(&tool_indices);
    ordered.sort_by(|&a, &b| {
        eviction_priority(&messages[a])
            .partial_cmp(&eviction_priority(&messages[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let indices_to_remove: HashSet<usize> = ordered.into_iter().take(num_to_remove).collect();

    messages
        .iter()
        .enumerate()
        .filter(|(i, _)| !indices_to_remove.contains(i))
        .map(|(_, msg)| msg)
        .collect()
}

async fn do_compact(
    provider: &dyn Provider,
    session_id: &str,
    messages: &[Message],
) -> Result<(Message, ProviderUsage), anyhow::Error> {
    let agent_visible_messages: Vec<Message> = messages
        .iter()
        .filter(|msg| msg.is_agent_visible())
        .map(|msg| msg.agent_visible_content())
        .collect();

    // Try progressively removing more tool response messages from the middle to reduce context length
    let removal_percentages = [0, 10, 20, 50, 100];

    for (attempt, &remove_percent) in removal_percentages.iter().enumerate() {
        let filtered_messages = filter_tool_responses(&agent_visible_messages, remove_percent);

        let messages_text = filtered_messages
            .iter()
            .map(|&msg| format_message_for_compacting(msg))
            .collect::<Vec<_>>()
            .join("\n");

        let context = SummarizeContext {
            messages: messages_text,
        };

        let system_prompt = render_template("compaction.md", &context)?;

        let user_message = Message::user()
            .with_text("Please summarize the conversation history provided in the system prompt.");
        let summarization_request = vec![user_message];

        match provider
            .complete_fast(session_id, &system_prompt, &summarization_request, &[])
            .await
        {
            Ok((mut response, mut provider_usage)) => {
                response.role = Role::User;

                provider_usage
                    .ensure_tokens(&system_prompt, &summarization_request, &response, &[])
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to ensure usage tokens: {}", e))?;

                return Ok((response, provider_usage));
            }
            Err(e) => {
                if matches!(e, ProviderError::ContextLengthExceeded(_)) {
                    if attempt < removal_percentages.len() - 1 {
                        continue;
                    } else {
                        return Err(anyhow::anyhow!(
                            "Failed to compact: context limit exceeded even after removing all tool responses"
                        ));
                    }
                }
                return Err(e.into());
            }
        }
    }

    Err(anyhow::anyhow!(
        "Unexpected: exhausted all attempts without returning"
    ))
}

pub fn format_message_for_compacting(msg: &Message) -> String {
    let content_parts: Vec<String> = msg
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.clone()),
            MessageContent::Image(img) => Some(format!("[image: {}]", img.mime_type)),
            MessageContent::ToolRequest(req) => {
                if let Ok(call) = &req.tool_call {
                    Some(format!(
                        "tool_request({}): {}",
                        call.name,
                        serde_json::to_string(&call.arguments)
                            .unwrap_or_else(|_| "<<invalid json>>".to_string())
                    ))
                } else {
                    Some("tool_request: [error]".to_string())
                }
            }
            MessageContent::ToolResponse(res) => {
                if let Ok(result) = &res.tool_result {
                    let text_items: Vec<String> = result
                        .content
                        .iter()
                        .filter_map(|content| {
                            content.as_text().map(|text_str| text_str.text.clone())
                        })
                        .collect();

                    if !text_items.is_empty() {
                        Some(format!("tool_response: {}", text_items.join("\n")))
                    } else {
                        Some("tool_response: [non-text content]".to_string())
                    }
                } else {
                    Some("tool_response: [error]".to_string())
                }
            }
            MessageContent::ToolConfirmationRequest(req) => {
                Some(format!("tool_confirmation_request: {}", req.tool_name))
            }
            MessageContent::ActionRequired(action) => match &action.data {
                ActionRequiredData::ToolConfirmation { tool_name, .. } => {
                    Some(format!("action_required(tool_confirmation): {}", tool_name))
                }
                ActionRequiredData::Elicitation { message, .. } => {
                    Some(format!("action_required(elicitation): {}", message))
                }
                ActionRequiredData::ElicitationResponse { id, .. } => {
                    Some(format!("action_required(elicitation_response): {}", id))
                }
            },
            MessageContent::FrontendToolRequest(req) => {
                if let Ok(call) = &req.tool_call {
                    Some(format!("frontend_tool_request: {}", call.name))
                } else {
                    Some("frontend_tool_request: [error]".to_string())
                }
            }
            MessageContent::Thinking(_) => None,
            MessageContent::RedactedThinking(_) => None,
            MessageContent::SystemNotification(notification) => {
                Some(format!("system_notification: {}", notification.msg))
            }
        })
        .collect();

    let role_str = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    if content_parts.is_empty() {
        format!("[{}]: <empty message>", role_str)
    } else {
        format!("[{}]: {}", role_str, content_parts.join("\n"))
    }
}

pub fn compute_tool_call_cutoff(context_limit: usize, compaction_threshold: f64) -> usize {
    let threshold = if compaction_threshold > 0.0 && compaction_threshold <= 1.0 {
        compaction_threshold
    } else {
        DEFAULT_COMPACTION_THRESHOLD
    };
    let effective_limit = (context_limit as f64 * threshold) as usize;
    (3 * effective_limit / 20_000).clamp(10, 500)
}

pub fn tool_ids_to_summarize(
    conversation: &Conversation,
    cutoff: usize,
    protect_last_n: usize,
) -> Vec<String> {
    let messages = conversation.messages();

    let mut tool_call_ids: Vec<String> = Vec::new();

    for msg in messages.iter() {
        if !msg.is_agent_visible() {
            continue;
        }

        for content in &msg.content {
            if let MessageContent::ToolRequest(req) = content {
                tool_call_ids.push(req.id.clone());
            }
        }
    }

    // Never summarize the last N tool calls (current turn)
    let eligible = tool_call_ids.len().saturating_sub(protect_last_n);
    if eligible <= cutoff + TOOLCALL_SUMMARIZATION_BATCH_SIZE {
        return Vec::new();
    }

    tool_call_ids
        .into_iter()
        .take(TOOLCALL_SUMMARIZATION_BATCH_SIZE)
        .collect()
}

pub async fn summarize_tool_call(
    provider: &dyn Provider,
    session_id: &str,
    conversation: &Conversation,
    tool_id: &str,
) -> Result<Message> {
    let messages = conversation.messages();

    let matching_messages: Vec<&Message> = messages
        .iter()
        .filter(|m| {
            m.content.iter().any(|c| match c {
                MessageContent::ToolRequest(req) => req.id == tool_id,
                MessageContent::ToolResponse(resp) => resp.id == tool_id,
                _ => false,
            })
        })
        .collect();

    if matching_messages.is_empty() {
        return Err(anyhow::anyhow!(
            "No messages found for tool id: {}",
            tool_id
        ));
    }

    let formatted = matching_messages
        .iter()
        .map(|msg| format_message_for_compacting(msg))
        .collect::<Vec<_>>()
        .join("\n");

    let user_message = Message::user().with_text(formatted);
    let summarization_request = vec![user_message];

    let system_prompt = indoc! {r#"
                Your task is to summarize a tool call & response pair to save tokens.

                Reply with a single message that describes what happened. Typically a tool call
                asks for something using a bunch of parameters and then the result is also some
                structured output. So the tool might ask to look up something on github and the
                reply might be a json document. So you could reply with something like:

                "A call to github was made to get the project status"

                if that is what it was.
            "#};

    let (mut response, _) = provider
        .complete_fast(session_id, system_prompt, &summarization_request, &[])
        .await?;

    response.role = Role::User;
    response.created = matching_messages.last().unwrap().created;
    response.metadata = MessageMetadata::agent_only();

    Ok(response.with_generated_id())
}

pub fn maybe_summarize_tool_pairs(
    provider: Arc<dyn Provider>,
    session_id: String,
    conversation: Conversation,
    cutoff: usize,
    protect_last_n: usize,
) -> JoinHandle<Vec<(Message, String)>> {
    tokio::spawn(async move {
        if !tool_pair_summarization_enabled() || provider.manages_own_context() {
            return Vec::new();
        }

        let tool_ids = tool_ids_to_summarize(&conversation, cutoff, protect_last_n);
        let mut results = Vec::new();
        for tool_id in tool_ids {
            match summarize_tool_call(provider.as_ref(), &session_id, &conversation, &tool_id).await
            {
                Ok(summary) => results.push((summary, tool_id)),
                Err(e) => {
                    warn!("Failed to summarize tool pair: {}", e);
                }
            }
        }
        results
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::ModelConfig,
        providers::{base::Usage, errors::ProviderError},
    };
    use async_trait::async_trait;
    use rmcp::model::{
        AnnotateAble, CallToolRequestParams, CallToolResult, Content, RawContent, Tool,
    };
    use rmcp::object;

    fn create_tool_pair(
        call_id: &str,
        response_id: &str,
        tool_name: &str,
        response_text: &str,
    ) -> Vec<Message> {
        vec![
            Message::assistant()
                .with_tool_request(
                    call_id,
                    Ok(CallToolRequestParams::new(tool_name.to_string())),
                )
                .with_id(call_id),
            Message::user()
                .with_tool_response(
                    call_id,
                    Ok(rmcp::model::CallToolResult::success(vec![
                        RawContent::text(response_text).no_annotation(),
                    ])),
                )
                .with_id(response_id),
        ]
    }

    struct MockProvider {
        message: Message,
        config: ModelConfig,
        max_tool_responses: Option<usize>,
    }

    impl MockProvider {
        fn new(message: Message, context_limit: usize) -> Self {
            Self {
                message,
                config: ModelConfig {
                    model_name: "test".to_string(),
                    context_limit: Some(context_limit),
                    temperature: None,
                    max_tokens: None,
                    toolshim: false,
                    toolshim_model: None,
                    fast_model_config: None,
                    request_params: None,
                    reasoning: None,
                },
                max_tool_responses: None,
            }
        }

        fn with_max_tool_responses(mut self, max: usize) -> Self {
            self.max_tool_responses = Some(max);
            self
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn get_name(&self) -> &str {
            "mock"
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _session_id: &str,
            _system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            // If max_tool_responses is set, fail if we have too many
            if let Some(max) = self.max_tool_responses {
                let tool_response_count = messages
                    .iter()
                    .filter(|m| {
                        m.content
                            .iter()
                            .any(|c| matches!(c, MessageContent::ToolResponse(_)))
                    })
                    .count();

                if tool_response_count > max {
                    return Err(ProviderError::ContextLengthExceeded(format!(
                        "Too many tool responses: {} > {}",
                        tool_response_count, max
                    )));
                }
            }

            let message = self.message.clone();
            let usage = ProviderUsage::new("mock-model".to_string(), Usage::default());
            Ok(stream_from_single_message(message, usage))
        }

        fn get_model_config(&self) -> ModelConfig {
            self.config.clone()
        }
    }

    #[tokio::test]
    async fn test_keeps_tool_request() {
        let response_message = Message::assistant().with_text("<mock summary>");
        let provider = MockProvider::new(response_message, 1);
        let basic_conversation = vec![
            Message::user().with_text("read hello.txt"),
            Message::assistant()
                .with_tool_request("tool_0", Ok(CallToolRequestParams::new("read_file"))),
            Message::user().with_tool_response(
                "tool_0",
                Ok(rmcp::model::CallToolResult::success(vec![
                    RawContent::text("hello, world").no_annotation(),
                ])),
            ),
        ];

        let conversation = Conversation::new_unvalidated(basic_conversation);
        let (compacted_conversation, _usage) =
            compact_messages(&provider, "test-session-id", &conversation, false)
                .await
                .unwrap();

        let agent_conversation = compacted_conversation.agent_visible_messages();

        let _ = Conversation::new(agent_conversation)
            .expect("compaction should produce a valid conversation");
    }

    #[tokio::test]
    async fn test_progressive_removal_on_context_exceeded() {
        let response_message = Message::assistant().with_text("<mock summary>");
        // Set max to 2 tool responses - will trigger progressive removal
        let provider = MockProvider::new(response_message, 1000).with_max_tool_responses(2);

        // Create a conversation with many tool responses
        let mut messages = vec![Message::user().with_text("start")];
        for i in 0..10 {
            messages.push(Message::assistant().with_tool_request(
                format!("tool_{}", i),
                Ok(CallToolRequestParams::new("read_file")),
            ));
            messages.push(Message::user().with_tool_response(
                format!("tool_{}", i),
                Ok(rmcp::model::CallToolResult::success(vec![
                    RawContent::text(format!("response{}", i)).no_annotation(),
                ])),
            ));
        }

        let conversation = Conversation::new_unvalidated(messages);
        let result = compact_messages(&provider, "test-session-id", &conversation, false).await;

        assert!(
            result.is_ok(),
            "Should succeed with progressive removal: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_compute_tool_call_cutoff_scales_with_context() {
        // Default threshold (0.8)
        assert_eq!(compute_tool_call_cutoff(128_000, 0.8), 15); // 102K effective
        assert_eq!(compute_tool_call_cutoff(200_000, 0.8), 24); // 160K effective
        assert_eq!(compute_tool_call_cutoff(1_000_000, 0.8), 120); // 800K effective
                                                                   // Clamp at minimum
        assert_eq!(compute_tool_call_cutoff(50_000, 0.8), 10);
        assert_eq!(compute_tool_call_cutoff(10_000, 0.8), 10);
        // Clamp at maximum (500)
        assert_eq!(compute_tool_call_cutoff(10_000_000, 0.8), 500);
        // Lower compaction threshold means earlier summarization
        assert_eq!(compute_tool_call_cutoff(200_000, 0.3), 10); // 60K effective
        assert_eq!(compute_tool_call_cutoff(1_000_000, 0.5), 75); // 500K effective
                                                                  // Invalid threshold falls back to default 0.8
        assert_eq!(compute_tool_call_cutoff(200_000, 0.0), 24); // falls back to 0.8
        assert_eq!(compute_tool_call_cutoff(200_000, -1.0), 24); // falls back to 0.8
    }

    #[test]
    fn test_tool_ids_to_summarize_triggers_at_cutoff_plus_batch() {
        // cutoff=5, so we need >5+10=15 to trigger. 15 exactly should NOT trigger.
        let mut messages = vec![Message::user().with_text("hello")];
        for i in 0..15 {
            messages.extend(create_tool_pair(
                &format!("call{}", i),
                &format!("resp{}", i),
                "read_file",
                "content",
            ));
        }
        let conversation = Conversation::new_unvalidated(messages);
        let result = tool_ids_to_summarize(&conversation, 5, 0);
        assert!(result.is_empty(), "Exactly cutoff+batch should not trigger");

        // 16 tool calls: now exceeds cutoff+10, should return a batch of 10
        let mut messages = vec![Message::user().with_text("hello")];
        for i in 0..16 {
            messages.extend(create_tool_pair(
                &format!("call{}", i),
                &format!("resp{}", i),
                "read_file",
                "content",
            ));
        }
        let conversation = Conversation::new_unvalidated(messages);
        let result = tool_ids_to_summarize(&conversation, 5, 0);
        assert_eq!(result.len(), TOOLCALL_SUMMARIZATION_BATCH_SIZE);
        assert_eq!(result[0], "call0");
        assert_eq!(result[9], "call9");
    }

    #[test]
    fn test_tool_ids_to_summarize_protects_current_turn() {
        // 20 tool pairs, cutoff=2 → 20 > 12, would normally trigger
        let mut messages = vec![Message::user().with_text("hello")];
        for i in 0..20 {
            messages.extend(create_tool_pair(
                &format!("call{}", i),
                &format!("resp{}", i),
                "read_file",
                "content",
            ));
        }
        let conversation = Conversation::new_unvalidated(messages);

        // No protection: 20 eligible, 20 > 12 → batch of 10
        let result = tool_ids_to_summarize(&conversation, 2, 0);
        assert_eq!(result.len(), TOOLCALL_SUMMARIZATION_BATCH_SIZE);

        // Protect last 8: 12 eligible, 12 <= 12 → nothing
        let result = tool_ids_to_summarize(&conversation, 2, 8);
        assert!(
            result.is_empty(),
            "Should not summarize when protected count leaves eligible <= cutoff + batch"
        );

        // Protect last 7: 13 eligible, 13 > 12 → batch of 10
        let result = tool_ids_to_summarize(&conversation, 2, 7);
        assert_eq!(result.len(), TOOLCALL_SUMMARIZATION_BATCH_SIZE);
        assert_eq!(result[0], "call0");
    }

    // ── Code-safe compaction ─────────────────────────────────────────────

    /// LOAD-BEARING: a trajectory carrying a diff (an `edit` tool call's
    /// before/after) survives compaction BYTE-IDENTICAL, while a bulky
    /// priority-0.0 shell log is summarized away. Lossy summarization of code
    /// silently corrupts identifiers/whitespace (arXiv 2506.00307); this proves
    /// the summarizer can never touch a diff.
    #[tokio::test]
    async fn code_and_diffs_survive_compaction_byte_identical() {
        let before = "fn add(a: i32, b: i32) -> i32 {\n    a - b // BUG\n}";
        let after = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}";
        let expected_args = object!({
            "path": "/repo/src/math.rs",
            "before": before,
            "after": after,
        });

        let convo = vec![
            Message::user().with_text("fix the add bug"),
            // The diff: an `edit` tool call whose args carry the code verbatim.
            Message::assistant()
                .with_tool_request(
                    "edit1",
                    Ok(CallToolRequestParams::new("developer__edit".to_string())
                        .with_arguments(expected_args.clone())),
                )
                .with_id("edit1"),
            Message::user()
                .with_tool_response(
                    "edit1",
                    Ok(CallToolResult::success(vec![Content::text(
                        "Edited /repo/src/math.rs (3 lines -> 3 lines)",
                    )
                    .with_priority(0.0)])),
                )
                .with_id("edit1_resp"),
            // A bulky low-value shell log (priority 0.0) — must be summarized away.
            Message::assistant()
                .with_tool_request(
                    "sh1",
                    Ok(CallToolRequestParams::new("developer__shell".to_string())),
                )
                .with_id("sh1"),
            Message::user()
                .with_tool_response(
                    "sh1",
                    Ok(CallToolResult::success(vec![Content::text(
                        "SHELL_LOG_MARKER: 4000 lines of cargo build output ...",
                    )
                    .with_priority(0.0)])),
                )
                .with_id("sh1_resp"),
        ];

        let provider = MockProvider::new(Message::assistant().with_text("<mock summary>"), 100_000);
        let (compacted, _usage) = compact_messages(
            &provider,
            "test-session-id",
            &Conversation::new_unvalidated(convo),
            false,
        )
        .await
        .unwrap();

        let agent_msgs = compacted.agent_visible_messages();

        // The edit tool call — and thus the diff — survives byte-identical.
        let recovered = agent_msgs
            .iter()
            .flat_map(|m| &m.content)
            .find_map(|c| match c {
                MessageContent::ToolRequest(req) => match req.tool_call.as_ref() {
                    Ok(call) if unprefixed_tool_name(&call.name) == "edit" => Some(call.clone()),
                    _ => None,
                },
                _ => None,
            })
            .expect("the edit tool call must survive compaction verbatim");
        assert_eq!(
            recovered.arguments,
            Some(expected_args),
            "the diff (edit before/after) must survive BYTE-IDENTICAL"
        );

        // The summary replaced the prose/logs; the bulky shell log is gone.
        let agent_text = agent_msgs
            .iter()
            .map(format_message_for_compacting)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            agent_text.contains("<mock summary>"),
            "the non-code summary must be present"
        );
        assert!(
            !agent_text.contains("SHELL_LOG_MARKER"),
            "the priority-0.0 shell log must be summarized away, not preserved verbatim"
        );
    }

    /// `filter_tool_responses` evicts the lowest-priority responses first:
    /// search/shell output stamped `with_priority(0.0)` goes before untagged
    /// (`None`, higher) responses, consuming the priority tags for ordering.
    #[test]
    fn filter_tool_responses_evicts_lowest_priority_first() {
        let response = |id: &str, text: &str, priority: Option<f32>| {
            let content = match priority {
                Some(p) => Content::text(text).with_priority(p),
                None => RawContent::text(text).no_annotation(),
            };
            Message::user()
                .with_tool_response(id, Ok(CallToolResult::success(vec![content])))
                .with_id(id)
        };

        // One 0.0 response between two untagged (None) ones; removing one must
        // drop the 0.0 log, never the untagged responses.
        let msgs = vec![
            response("a", "KEEP_A", None),
            response("b", "LOW_LOG", Some(0.0)),
            response("c", "KEEP_C", None),
        ];
        let kept: Vec<String> = filter_tool_responses(&msgs, 1) // num_to_remove = 1
            .into_iter()
            .map(format_message_for_compacting)
            .collect();
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|t| t.contains("KEEP_A")));
        assert!(kept.iter().any(|t| t.contains("KEEP_C")));
        assert!(
            !kept.iter().any(|t| t.contains("LOW_LOG")),
            "the priority-0.0 response must be evicted before untagged ones"
        );

        // Two 0.0 responses and one untagged; removing two drops BOTH logs and
        // keeps the higher-priority untagged response.
        let msgs = vec![
            response("a", "KEEP_HI", None),
            response("b", "LOW_1", Some(0.0)),
            response("c", "LOW_2", Some(0.0)),
        ];
        let kept: Vec<String> = filter_tool_responses(&msgs, 67) // num_to_remove = 2
            .into_iter()
            .map(format_message_for_compacting)
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(
            kept[0].contains("KEEP_HI"),
            "both priority-0.0 logs evict before the untagged response"
        );
    }

    /// The code/diff classifier keys on the (unprefixed) tool name: file
    /// read/write/edit tools are code-bearing; shell/search are not.
    #[test]
    fn code_bearing_classifier_matches_file_tools_and_ignores_logs() {
        assert_eq!(unprefixed_tool_name("developer__edit"), "edit");
        assert_eq!(unprefixed_tool_name("edit"), "edit");
        assert_eq!(unprefixed_tool_name("a__b__write"), "write");

        assert!(is_code_bearing_tool("edit"));
        assert!(is_code_bearing_tool("developer__write"));
        assert!(is_code_bearing_tool("file_read"));
        assert!(!is_code_bearing_tool("shell"));
        assert!(!is_code_bearing_tool("search"));
        assert!(!is_code_bearing_tool("read_file")); // not one of our read names

        let ids = code_bearing_tool_ids(&[
            Message::assistant().with_tool_request(
                "e",
                Ok(CallToolRequestParams::new("developer__edit".to_string())),
            ),
            Message::assistant().with_tool_request(
                "s",
                Ok(CallToolRequestParams::new("developer__shell".to_string())),
            ),
        ]);
        assert!(ids.contains("e"));
        assert!(!ids.contains("s"));

        // A response is code-bearing iff it answers a code-bearing request.
        let edit_resp = Message::user().with_tool_response(
            "e",
            Ok(CallToolResult::success(vec![
                Content::text("ok").with_priority(0.0)
            ])),
        );
        assert!(is_code_bearing_message(&edit_resp, &ids));
        let shell_resp = Message::user().with_tool_response(
            "s",
            Ok(CallToolResult::success(vec![
                Content::text("log").with_priority(0.0)
            ])),
        );
        assert!(!is_code_bearing_message(&shell_resp, &ids));
    }
}
