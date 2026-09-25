use crate::{
    agents::{
        subagent_task_config::TaskConfig, Agent, AgentEvent, AgentRunnerConfig,
        AgentRuntimeOutcome, SessionConfig,
    },
    conversation::{
        message::{Message, MessageContent},
        Conversation,
    },
    prompt_template::render_template,
    recipe::Recipe,
};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use rmcp::model::{
    ErrorCode, ErrorData, LoggingLevel, LoggingMessageNotificationParam, Notification,
    ServerNotification,
};
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

pub type OnMessageCallback = Arc<dyn Fn(&Message) + Send + Sync>;

#[derive(Serialize)]
pub struct SubagentPromptContext {
    pub max_turns: usize,
    pub subagent_id: String,
    pub task_instructions: String,
    pub tool_count: usize,
    pub available_tools: String,
}

type AgentMessagesFuture =
    Pin<Box<dyn Future<Output = Result<(Conversation, Option<String>)>> + Send>>;

pub struct SubagentRunParams {
    pub config: AgentRunnerConfig,
    pub recipe: Recipe,
    pub task_config: TaskConfig,
    pub return_last_only: bool,
    pub session_id: String,
    pub cancellation_token: Option<CancellationToken>,
    pub on_message: Option<OnMessageCallback>,
    pub notification_tx: Option<tokio::sync::mpsc::UnboundedSender<ServerNotification>>,
    /// Resolved persona block and display name for the subagent.
    /// When set, prepended to the subagent's system instructions.
    pub persona_override: Option<(String, String)>,
}

pub async fn run_subagent_task(params: SubagentRunParams) -> Result<String, anyhow::Error> {
    crate::providers::inflight::background(run_subagent_task_inner(params)).await
}

async fn run_subagent_task_inner(params: SubagentRunParams) -> Result<String, anyhow::Error> {
    let return_last_only = params.return_last_only;
    let (messages, final_output) = get_agent_messages(params).await.map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to execute task: {}", e),
            None,
        )
    })?;

    if let Some(output) = final_output {
        return Ok(output);
    }

    Ok(extract_response_text(&messages, return_last_only))
}

/// Spawn [`run_subagent_task`] without awaiting. The caller joins later so
/// parallel review/audit workers can be outstanding at once.
pub fn spawn_subagent_task(params: SubagentRunParams) -> JoinHandle<Result<String, anyhow::Error>> {
    tokio::spawn(run_subagent_task(params))
}

/// Spawn any Send future as subagent work. Used by parallel review fan-out
/// and by unit tests that must not construct a full agent.
pub fn spawn_subagent_work<F>(work: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(crate::providers::inflight::background(work))
}

fn extract_response_text(messages: &Conversation, return_last_only: bool) -> String {
    if return_last_only {
        messages
            .messages()
            .last()
            .and_then(|message| {
                message.content.iter().find_map(|content| match content {
                    crate::conversation::message::MessageContent::Text(text_content) => {
                        Some(text_content.text.clone())
                    }
                    _ => None,
                })
            })
            .unwrap_or_else(|| String::from("No text content in last message"))
    } else {
        let all_text_content: Vec<String> = messages
            .iter()
            .flat_map(|message| {
                message.content.iter().filter_map(|content| match content {
                    crate::conversation::message::MessageContent::Text(text_content) => {
                        Some(text_content.text.clone())
                    }
                    crate::conversation::message::MessageContent::ToolResponse(tool_response) => {
                        if let Ok(result) = &tool_response.tool_result {
                            let texts: Vec<String> = result
                                .content
                                .iter()
                                .filter_map(|content| {
                                    if let rmcp::model::RawContent::Text(raw_text_content) =
                                        &content.raw
                                    {
                                        Some(raw_text_content.text.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if !texts.is_empty() {
                                Some(format!("Tool result: {}", texts.join("\n")))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
            })
            .collect();

        all_text_content.join("\n")
    }
}

pub const SUBAGENT_TOOL_REQUEST_TYPE: &str = "subagent_tool_request";

/// Build the [`Agent`] every subagent run through this module shares —
/// goal-dispatch workers (`InternalSubagentEngine::spawn`), `summon.rs`
/// delegates, and best-of-N candidates alike. All of them run as
/// `SessionType::SubAgent`, and `SessionType::is_interactive` never returns
/// `true` for that type: there is no human who could ever answer a parked
/// tool-approval confirmation for a session built here. Mark it headless up
/// front, the same way `scheduler.rs` marks a scheduled-recipe job, so a
/// scanner-flagged tool call is auto-denied with a recorded skip
/// (`HEADLESS_SKIPPED_RESPONSE`) instead of parking on a confirmation that
/// structurally can never arrive — the mechanism traced in
/// HARNESS_LOOP_LOG.md's A28 analysis, miss 2 (`is_headless()` was
/// previously only ever set by `scheduler.rs`, never by this dispatch path).
fn build_subagent_agent(config: AgentRunnerConfig) -> Arc<Agent> {
    let agent = Arc::new(Agent::with_config(config));
    agent.set_headless(true);
    agent
}

fn get_agent_messages(params: SubagentRunParams) -> AgentMessagesFuture {
    Box::pin(async move {
        let SubagentRunParams {
            config,
            recipe,
            task_config,
            session_id,
            cancellation_token,
            on_message,
            notification_tx,
            persona_override,
            ..
        } = params;

        let mut system_instructions = recipe.instructions.clone().unwrap_or_default();
        if let Some((ref persona_block, _)) = persona_override {
            system_instructions = format!("{}\n\n{}", persona_block, system_instructions);
        }
        let user_task = recipe
            .prompt
            .clone()
            .unwrap_or_else(|| "Begin.".to_string());

        let compact_worker_profile = config.compact_worker_profile;
        let agent = build_subagent_agent(config);

        agent
            .update_provider(task_config.provider.clone(), &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set provider on sub agent: {}", e))?;

        for extension in &task_config.extensions {
            if let Err(e) = agent.add_extension(extension.clone(), &session_id).await {
                debug!(
                    "Failed to add extension '{}' to subagent: {}",
                    extension.name(),
                    e
                );
            }
        }

        let has_response_schema = recipe.response.is_some();
        agent
            .apply_recipe_components(recipe.response.clone(), true)
            .await?;

        let subagent_prompt =
            build_subagent_prompt(&agent, &task_config, &session_id, system_instructions).await?;
        agent.override_system_prompt(subagent_prompt.clone()).await;

        let user_message = Message::user().with_text(user_task);

        // Before the first turn: a provider that already knows its window
        // (a local Ollama model) says now whether this first request fits.
        // A refusal ends the run here with the classified `context_exceeded`
        // the orchestrator's local→cloud fallback acts on. Left to the reply
        // loop, the same refusal is answered by compacting a conversation
        // with nothing in it on the same too-small model: 2026-09-25, goal
        // 01a0d8d8, `qwen2.5-coder:7b` refused a 6,361-token first request
        // against 4096, and the attempt then stayed "running" (no turn ever
        // completed) until the goal's whole 182 s wall budget was gone,
        // leaving no budget for the cloud retry.
        preflight_first_request(
            &agent,
            &task_config,
            &session_id,
            &subagent_prompt,
            &user_message,
            compact_worker_profile,
        )
        .await?;
        let mut conversation = Conversation::new_unvalidated(vec![user_message.clone()]);

        if let Some(activities) = recipe.activities {
            for activity in activities {
                info!("Recipe activity: {}", activity);
            }
        }
        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: task_config.max_turns.map(|v| v as u32),
            retry_config: recipe.retry,
        };

        let mut stream =
            crate::session_context::with_session_id(Some(session_id.to_string()), async {
                agent
                    .reply(user_message, session_config, cancellation_token)
                    .await
            })
            .await
            .map_err(|e| anyhow!("Failed to get reply from agent: {}", e))?;

        let mut runtime_outcome = None;
        while let Some(message_result) = stream.next().await {
            match message_result {
                Ok(AgentEvent::Message(msg)) => {
                    if let Some(ref callback) = on_message {
                        callback(&msg);
                    }
                    if let Some(ref tx) = notification_tx {
                        for content in &msg.content {
                            if let Some(notif) = create_tool_notification(content, &session_id) {
                                if tx.send(notif).is_err() {
                                    debug!(
                                        "Notification receiver dropped for subagent {}",
                                        session_id
                                    );
                                }
                            }
                        }
                    }
                    conversation.push(msg);
                }
                Ok(AgentEvent::McpNotification(_)) => {}
                Ok(AgentEvent::HistoryReplaced(updated_conversation)) => {
                    conversation = updated_conversation;
                }
                Ok(AgentEvent::RuntimeOutcome(outcome)) => {
                    runtime_outcome = Some(outcome);
                }
                Err(e) => {
                    tracing::error!("Error receiving message from subagent: {}", e);
                    return Err(anyhow!("Subagent stream failed: {e}"));
                }
            }
        }

        if let Some(outcome) = runtime_outcome {
            // R46a (harness loop, "A43 analysis" lever 3): gather what the
            // failed turn itself knows about WHY before it collapses into a
            // bare `-32603` — `Agent::take_context_length_failure` for the
            // exact regression A43 found (`delegate`/`delegate_many`
            // overflowing a ≤32k-ctx tier), and the failed turn's own last
            // yielded message (already captured in `conversation`, above)
            // as a fallback for the OTHER provider-unavailable classes.
            let context_length_failure = agent.take_context_length_failure().await;
            let last_text = last_message_failure_text(&conversation);
            if let Some(error) =
                runtime_outcome_error(outcome, context_length_failure, last_text.as_deref())
            {
                return Err(error);
            }
        }

        let final_output = get_final_output(&agent, has_response_schema).await;

        Ok((conversation, final_output))
    })
}

/// The failed turn's own last yielded message text, covering both an
/// ordinary `.with_text(...)` message and the `.with_system_notification(
/// ...)` shape most of `agent.rs`'s give-up arms actually use (whose text
/// `Message::as_concat_text` does not see, since it only reads `Text`
/// content). `None` when the conversation is empty or the last message
/// carries neither — `classified_subagent_failure_message` falls back to
/// the generic reason in that case.
fn last_message_failure_text(conversation: &Conversation) -> Option<String> {
    let msg = conversation.last()?;
    let text = msg.as_concat_text();
    if !text.is_empty() {
        return Some(text);
    }
    msg.content
        .iter()
        .find_map(|c| c.as_system_notification())
        .map(|n| n.msg.clone())
}

/// Convert the typed terminal outcome into a worker error. The caller's
/// cancellation token remains the authority for classifying cancellation in a
/// fan-out, while a failed runtime must never be mistaken for a successful
/// last assistant message.
fn runtime_outcome_error(
    outcome: AgentRuntimeOutcome,
    context_length_failure: Option<String>,
    last_assistant_text: Option<&str>,
) -> Option<anyhow::Error> {
    match outcome {
        AgentRuntimeOutcome::Succeeded => None,
        AgentRuntimeOutcome::Failed => Some(anyhow!(classified_subagent_failure_message(
            context_length_failure,
            last_assistant_text,
        ))),
        AgentRuntimeOutcome::Cancelled => Some(anyhow!("Subagent runtime cancelled")),
    }
}

/// R46a (harness loop, "A43 analysis" lever 3): turn a bare
/// `AgentRuntimeOutcome::Failed` into a reason a coordinator model can act
/// on, instead of the opaque `-32603: Subagent runtime failed` A43's
/// MiniCPM5-2B probe got twice in a row from `delegate`/`delegate_many` — a
/// message with no way to distinguish "the sub-session's context
/// overflowed" from "some unrelated bug", so even a frontier coordinator
/// could not self-correct (the analysis's own ranked lever 3).
///
/// `context_length_failure` (from `Agent::take_context_length_failure`)
/// takes priority: it is the exact regression A43 found, and the only
/// source that can carry the real `n_prompt_tokens`/`n_ctx` numbers
/// (`providers::errors::extract_context_length_token_counts`). Otherwise
/// falls back to classifying `last_assistant_text` — the failed turn's own
/// final message — via `providers::errors::classify_unavailable`, covering
/// the OTHER provider-unavailable classes (billing/auth/rate-limit/server/
/// network/timeout) the lever also named. Neither source available (or
/// neither classifies) → the original bare message, unchanged.
fn classified_subagent_failure_message(
    context_length_failure: Option<String>,
    last_assistant_text: Option<&str>,
) -> String {
    if let Some(raw) = context_length_failure {
        debug_assert!(
            raw.starts_with(crate::providers::errors::CONTEXT_LENGTH_EXCEEDED_MARKER),
            "Agent::record_context_length_failure only ever stores a \
             ProviderError::ContextLengthExceeded's own Display text, got: {raw:?}"
        );
        return match crate::providers::errors::extract_context_length_token_counts(&raw) {
            Some((prompt_tokens, limit)) => format!(
                "subagent failed: context_exceeded (prompt {prompt_tokens} tokens > limit \
                 {limit}); use goal_advance dispatch instead"
            ),
            None => "subagent failed: context_exceeded (the sub-session's prompt overflowed \
                      its model's context window); use goal_advance dispatch instead"
                .to_string(),
        };
    }

    if let Some(text) = last_assistant_text {
        if let Some(class) = crate::providers::errors::classify_unavailable(text) {
            return format!("subagent failed: {class} ({text}); use goal_advance dispatch instead");
        }
    }

    "Subagent runtime failed".to_string()
}

async fn build_subagent_prompt(
    agent: &Agent,
    task_config: &TaskConfig,
    session_id: &str,
    system_instructions: String,
) -> Result<String> {
    let tools: Vec<_> = agent
        .list_tools(session_id, None)
        .await
        .into_iter()
        .filter(super::reply_parts::is_tool_visible_to_model)
        .collect();
    render_template(
        "subagent_system.md",
        &SubagentPromptContext {
            max_turns: task_config
                .max_turns
                .expect("TaskConfig always sets max_turns"),
            subagent_id: session_id.to_string(),
            task_instructions: system_instructions,
            tool_count: tools.len(),
            available_tools: tools
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        },
    )
    .map_err(|e| anyhow!("Failed to render subagent system prompt: {}", e))
}

/// Pure: what a pre-send check's outcome means for the run. A context
/// refusal becomes the run's error, worded by
/// [`classified_subagent_failure_message`] exactly as a refusal inside the
/// reply loop would be — `"subagent failed: context_exceeded (prompt N
/// tokens > limit M)…"` — so the orchestrator's
/// `classify_worker_fallback_reason` reads it as `ContextExceeded` and the
/// next dispatch goes to the cloud tier. Anything else — it fits, or the
/// check itself failed for another reason — is not a verdict and lets the
/// run proceed: the send reports its own errors.
pub(crate) fn first_request_refusal_message(
    preflight: &Result<(), crate::providers::errors::ProviderError>,
) -> Option<String> {
    match preflight {
        Err(err @ crate::providers::errors::ProviderError::ContextLengthExceeded(_)) => Some(
            classified_subagent_failure_message(Some(err.to_string()), None),
        ),
        _ => None,
    }
}

/// Run the provider's pre-send window check on the worker's first request
/// (see [`crate::providers::base::Provider::preflight_request`]), and for a
/// provider whose window is known, remember how big that request's fixed
/// part (system prompt + tool schemas) was, for routing to size the next
/// local dispatch against
/// ([`crate::cost_router::fallback::record_worker_first_request_overhead`]).
///
/// The system prompt checked is the subagent template the run installs;
/// the agent may append hints to it, so this is a floor, and the send's own
/// preflight still refuses whatever this lets through.
async fn preflight_first_request(
    agent: &Agent,
    task_config: &TaskConfig,
    session_id: &str,
    system: &str,
    user_message: &Message,
    compact_worker_profile: bool,
) -> Result<()> {
    let provider = &task_config.provider;
    let model_config = provider.get_model_config();
    let tools: Vec<_> = agent
        .list_tools(session_id, None)
        .await
        .into_iter()
        .filter(super::reply_parts::is_tool_visible_to_model)
        .collect();
    let window = provider.request_context_window(&model_config).await;
    if window.is_some() {
        if let Ok(counter) = crate::token_counter::create_token_counter().await {
            let tools_json = serde_json::to_string(&tools).unwrap_or_default();
            let overhead = counter
                .count_tokens(system)
                .saturating_add(counter.count_tokens(&tools_json));
            crate::cost_router::fallback::record_worker_first_request_overhead(
                compact_worker_profile,
                overhead,
            );
        }
    }
    let preflight = provider
        .preflight_request(
            &model_config,
            system,
            std::slice::from_ref(user_message),
            &tools,
        )
        .await;
    match first_request_refusal_message(&preflight) {
        Some(message) => {
            tracing::warn!(
                target: "permagent::cost_router",
                session_id,
                provider = provider.get_name(),
                model = %model_config.model_name,
                window = ?window,
                reason = "first_request_exceeds_window",
                "worker's first request does not fit its model's window; ending the attempt \
                 before any turn so the local\u{2192}cloud fallback can take it"
            );
            Err(anyhow!(message))
        }
        None => Ok(()),
    }
}

async fn get_final_output(agent: &Agent, has_response_schema: bool) -> Option<String> {
    if has_response_schema {
        agent
            .final_output_tool
            .lock()
            .await
            .as_ref()
            .and_then(|tool| tool.final_output.clone())
    } else {
        None
    }
}

pub fn create_tool_notification(
    content: &MessageContent,
    subagent_id: &str,
) -> Option<ServerNotification> {
    if let MessageContent::ToolRequest(req) = content {
        let tool_call = req.tool_call.as_ref().ok()?;

        Some(ServerNotification::LoggingMessageNotification(
            Notification::new(
                LoggingMessageNotificationParam::new(
                    LoggingLevel::Info,
                    serde_json::json!({
                        "type": SUBAGENT_TOOL_REQUEST_TYPE,
                        "subagent_id": subagent_id,
                        "tool_call": {
                            "name": tool_call.name,
                            "arguments": tool_call.arguments
                        }
                    }),
                )
                .with_logger(format!("subagent:{}", subagent_id)),
            ),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_subagent_agent, create_tool_notification, runtime_outcome_error,
        SUBAGENT_TOOL_REQUEST_TYPE,
    };
    use crate::agents::{AgentRunnerConfig, AgentRuntimeOutcome, GoosePlatform};
    use crate::config::permission::PermissionManager;
    use crate::config::GooseMode;
    use crate::conversation::message::MessageContent;
    use rmcp::model::{CallToolRequestParams, ServerNotification};
    use serde_json::json;

    /// The seam this lane fixes: every agent built through
    /// `build_subagent_agent` (goal-dispatch workers, `summon.rs`,
    /// best-of-N) must come up headless, since none of them ever has a
    /// human able to answer a parked tool-approval confirmation. See the
    /// function's own doc comment and HARNESS_LOOP_LOG.md's A28 analysis,
    /// miss 2.
    #[tokio::test]
    async fn subagent_agents_are_always_headless() {
        let tmp = tempfile::tempdir().unwrap();
        let sm = std::sync::Arc::new(crate::session::SessionManager::new(
            tmp.path().to_path_buf(),
        ));
        let config = AgentRunnerConfig::new(
            sm,
            PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true,
            GoosePlatform::GooseCli,
        );
        let agent = build_subagent_agent(config);
        assert!(
            agent.is_headless(),
            "a subagent built through this module must run headless — no \
             approver can ever answer its tool confirmations"
        );
    }

    #[test]
    fn create_tool_notification_for_tool_request() {
        let tool_call = CallToolRequestParams::new("developer__shell".to_string())
            .with_arguments(json!({"command": "ls"}).as_object().unwrap().clone());
        let content = MessageContent::tool_request("req1", Ok(tool_call));
        let notification =
            create_tool_notification(&content, "session_1").expect("expected notification");

        let ServerNotification::LoggingMessageNotification(log_notif) = notification else {
            panic!("expected logging notification");
        };
        let data = log_notif
            .params
            .data
            .as_object()
            .expect("expected object data");
        assert_eq!(
            data.get("type").and_then(|v| v.as_str()),
            Some(SUBAGENT_TOOL_REQUEST_TYPE)
        );
        assert_eq!(
            data.get("subagent_id").and_then(|v| v.as_str()),
            Some("session_1")
        );
        let tool_call = data
            .get("tool_call")
            .and_then(|v| v.as_object())
            .expect("expected tool_call object");
        assert_eq!(
            tool_call.get("name").and_then(|v| v.as_str()),
            Some("developer__shell")
        );
    }

    #[test]
    fn create_tool_notification_ignores_non_tool_request() {
        let content = MessageContent::text("hello");
        assert!(create_tool_notification(&content, "session_1").is_none());
    }

    #[test]
    fn failed_and_cancelled_runtime_outcomes_are_not_successes() {
        assert!(runtime_outcome_error(AgentRuntimeOutcome::Succeeded, None, None).is_none());
        assert_eq!(
            runtime_outcome_error(AgentRuntimeOutcome::Failed, None, None)
                .expect("failed runtime must propagate")
                .to_string(),
            "Subagent runtime failed",
            "no classifiable detail available must fall back to the original bare message"
        );
        assert_eq!(
            runtime_outcome_error(AgentRuntimeOutcome::Cancelled, None, None)
                .expect("cancelled runtime must propagate")
                .to_string(),
            "Subagent runtime cancelled"
        );
    }

    /// R46a (harness loop, "A43 analysis" lever 3): the exact regression —
    /// `delegate`/`delegate_many` overflowing a ≤32k-ctx tier — must surface
    /// as a classified reason naming the real prompt/limit numbers, not the
    /// bare `-32603`-shaped "Subagent runtime failed" A43's MiniCPM5-2B probe
    /// actually got.
    #[test]
    fn a_context_length_failure_is_classified_with_the_real_token_counts() {
        let raw = crate::providers::errors::ProviderError::ContextLengthExceeded(
            r#"{"error":{"type":"exceed_context_size_error","n_prompt_tokens":34930,"n_ctx":32768}}"#
                .to_string(),
        )
        .to_string();

        let error = runtime_outcome_error(AgentRuntimeOutcome::Failed, Some(raw), None)
            .expect("a failed runtime must propagate an error");
        let message = error.to_string();
        assert!(message.contains("context_exceeded"), "{message}");
        assert!(message.contains("34930"), "{message}");
        assert!(message.contains("32768"), "{message}");
        assert!(
            message.contains("goal_advance dispatch"),
            "must point the coordinator at the sanctioned path, got: {message}"
        );
    }

    /// A context-length failure whose raw body doesn't carry parseable
    /// `n_prompt_tokens`/`n_ctx` numbers (a different provider's shape) must
    /// still classify as `context_exceeded` — never silently fall through to
    /// the bare generic message just because the numbers are missing.
    #[test]
    fn a_context_length_failure_without_parseable_numbers_still_classifies() {
        let raw = crate::providers::errors::ProviderError::ContextLengthExceeded(
            "This model's maximum context length is 128000 tokens.".to_string(),
        )
        .to_string();

        let message = runtime_outcome_error(AgentRuntimeOutcome::Failed, Some(raw), None)
            .expect("a failed runtime must propagate an error")
            .to_string();
        assert!(message.contains("context_exceeded"), "{message}");
        assert!(!message.contains("-32603"), "{message}");
    }

    /// No context-length failure recorded, but the failed turn's own last
    /// message classifies as a provider-unavailable class (here: billing) —
    /// falls back to `classify_unavailable` against that text.
    #[test]
    fn a_provider_unavailable_class_is_classified_from_the_last_assistant_text() {
        let message = runtime_outcome_error(
            AgentRuntimeOutcome::Failed,
            None,
            Some("Your credit balance is too low to access the API"),
        )
        .expect("a failed runtime must propagate an error")
        .to_string();
        assert!(message.contains("billing"), "{message}");
        assert!(message.contains("goal_advance dispatch"), "got: {message}");
    }

    /// Neither source classifies (an ordinary bug, not a context/provider
    /// failure) — the original bare message, unchanged, never a fabricated
    /// classification.
    #[test]
    fn an_unclassifiable_failure_falls_back_to_the_original_bare_message() {
        assert_eq!(
            runtime_outcome_error(
                AgentRuntimeOutcome::Failed,
                None,
                Some("A tool call could not be parsed.")
            )
            .expect("a failed runtime must propagate an error")
            .to_string(),
            "Subagent runtime failed"
        );
    }

    #[tokio::test]
    async fn two_spawns_can_be_outstanding_before_either_join() {
        use crate::agents::subagent_handler::spawn_subagent_work;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let started = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = tokio::sync::watch::channel(false);

        let s1 = started.clone();
        let mut r1 = rx.clone();
        let h1 = spawn_subagent_work(async move {
            s1.fetch_add(1, Ordering::SeqCst);
            let _ = r1.changed().await;
            "one"
        });
        let s2 = started.clone();
        let mut r2 = rx;
        let h2 = spawn_subagent_work(async move {
            s2.fetch_add(1, Ordering::SeqCst);
            let _ = r2.changed().await;
            "two"
        });

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        while started.load(Ordering::SeqCst) < 2 {
            if tokio::time::Instant::now() > deadline {
                panic!("both spawns should start before either join");
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !h1.is_finished() && !h2.is_finished(),
            "joins must still be outstanding"
        );
        tx.send(true).unwrap();
        let (a, b) = tokio::join!(h1, h2);
        assert_eq!(a.unwrap(), "one");
        assert_eq!(b.unwrap(), "two");
    }
}
