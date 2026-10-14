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

/// Env var bounding how long a delegated worker may go without yielding ANY
/// event (message, notification, outcome) before the delegating tool call gives
/// up. `0` disables. Distinct from the provider's own stream-stall window
/// (`PERMAGENT_PROVIDER_STREAM_STALL_SECS`), which bounds one model response;
/// this one backstops everything else the worker awaits (tools, nested calls).
pub const SUBAGENT_IDLE_SECS_ENV: &str = "PERMAGENT_SUBAGENT_IDLE_SECS";
const DEFAULT_SUBAGENT_IDLE_SECS: u64 = 1800;
/// After a cancellation the worker's own stream gets this long to wind down
/// before the delegate stops waiting for it.
#[cfg(not(test))]
const SUBAGENT_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const SUBAGENT_CANCEL_GRACE: std::time::Duration = std::time::Duration::from_millis(50);

fn parse_idle_secs(raw: Option<&str>) -> Option<std::time::Duration> {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SUBAGENT_IDLE_SECS);
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

fn subagent_idle_window() -> Option<std::time::Duration> {
    parse_idle_secs(std::env::var(SUBAGENT_IDLE_SECS_ENV).ok().as_deref())
}

enum SubagentNext<T> {
    Event(T),
    End,
    Cancelled,
    Idle(std::time::Duration),
}

/// One `next()` on a worker's event stream that can neither outlive the
/// delegate's cancellation token nor wait past the idle window.
async fn next_subagent_event<S>(
    stream: &mut S,
    cancellation: Option<&CancellationToken>,
    idle: Option<std::time::Duration>,
) -> SubagentNext<S::Item>
where
    S: futures::Stream + Unpin,
{
    let wait = async {
        match idle {
            Some(window) => match tokio::time::timeout(window, stream.next()).await {
                Ok(Some(event)) => SubagentNext::Event(event),
                Ok(None) => SubagentNext::End,
                Err(_) => SubagentNext::Idle(window),
            },
            None => match stream.next().await {
                Some(event) => SubagentNext::Event(event),
                None => SubagentNext::End,
            },
        }
    };
    tokio::pin!(wait);
    let Some(token) = cancellation else {
        return wait.await;
    };
    tokio::select! {
        out = &mut wait => out,
        _ = token.cancelled() => {
            // Give the worker's own cancellation handling a moment to end the
            // stream cleanly; otherwise stop waiting on it.
            match tokio::time::timeout(SUBAGENT_CANCEL_GRACE, &mut wait).await {
                Ok(SubagentNext::Event(_)) | Err(_) => SubagentNext::Cancelled,
                Ok(other) => other,
            }
        }
    }
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
        agent.override_system_prompt(subagent_prompt).await;

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
            &user_message,
            compact_worker_profile,
            persona_override.as_ref().map(|(block, _)| block.as_str()),
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
                    .reply(user_message, session_config, cancellation_token.clone())
                    .await
            })
            .await
            .map_err(|e| anyhow!("Failed to get reply from agent: {}", e))?;

        let mut runtime_outcome = None;
        let idle_window = subagent_idle_window();
        loop {
            let message_result = match next_subagent_event(
                &mut stream,
                cancellation_token.as_ref(),
                idle_window,
            )
            .await
            {
                SubagentNext::Event(event) => event,
                SubagentNext::End => break,
                SubagentNext::Cancelled => return Err(anyhow!("Subagent runtime cancelled")),
                SubagentNext::Idle(window) => {
                    return Err(anyhow!(
                        "Subagent produced no event for {}s, so the run was stopped instead of \
                         waiting forever on a stuck model request or tool. Raise or disable \
                         {} (0 = unlimited) if this worker legitimately goes quiet for longer.",
                        window.as_secs(),
                        SUBAGENT_IDLE_SECS_ENV
                    ));
                }
            };
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
/// part (system prompt + tool schemas) was for this worker's surface, for
/// routing to size the next local dispatch of the same surface against
/// ([`crate::cost_router::fallback::record_worker_first_request_overhead`]).
///
/// The request checked is the one the reply loop's first send builds:
/// system prompt and tools from the same `Agent::prepare_tools_and_prompt`
/// the loop calls, so the saved skills, the project hints (`.goosehints`,
/// `AGENTS.md`), the model-identity line, the model-family overlay and the
/// live-status block are all counted — the subagent template alone left
/// every one of them out, so the check passed requests the send refused.
/// Building it acknowledges the informational briefings its prompt carries
/// (as the first turn always has); the first turn then builds it again
/// without them, so what is checked is never smaller than what is sent.
/// Only the per-turn info message extensions add to the history is left
/// out: computing it advances their per-turn state. The send's own check
/// still refuses whatever this lets through.
async fn preflight_first_request(
    agent: &Agent,
    task_config: &TaskConfig,
    session_id: &str,
    user_message: &Message,
    compact_worker_profile: bool,
    persona_block: Option<&str>,
) -> Result<()> {
    let provider = &task_config.provider;
    let model_config = provider.get_model_config();
    // A provider that cannot say what window it serves has nothing to judge
    // the request against (its `preflight_request` answers `Ok` too), so a
    // cloud worker's request is not built twice.
    let Some(window) = provider.request_context_window(&model_config).await else {
        return Ok(());
    };
    let working_dir = match agent
        .config
        .session_manager
        .get_session(session_id, false)
        .await
    {
        Ok(session) => session.working_dir,
        Err(e) => {
            debug!("worker first-request check skipped: no session {session_id}: {e}");
            return Ok(());
        }
    };
    // Not a verdict when the request cannot be built here: the reply loop
    // builds it again and reports its own error.
    let (tools, _toolshim_tools, system_prompt) = match agent
        .prepare_tools_and_prompt(session_id, &working_dir)
        .await
    {
        Ok(prepared) => prepared,
        Err(e) => {
            debug!("worker first-request check skipped: the request could not be built: {e}");
            return Ok(());
        }
    };
    let system = system_prompt.render();
    if let Ok(counter) = crate::token_counter::create_token_counter().await {
        let tools_json = serde_json::to_string(&tools).unwrap_or_default();
        let overhead = counter
            .count_tokens(&system)
            .saturating_add(counter.count_tokens(&tools_json));
        let project_hints = crate::cost_router::fallback::worker_project_hints(&working_dir);
        let surface = crate::cost_router::fallback::WorkerSurface {
            compact_profile: compact_worker_profile,
            persona_block,
            extensions: &task_config.extensions,
            provider: provider.get_name(),
            model: &model_config.model_name,
            project_hints: &project_hints,
        };
        crate::cost_router::fallback::record_worker_first_request_overhead(
            &surface.key(),
            overhead,
        );
    }
    let preflight = provider
        .preflight_request(
            &model_config,
            &system,
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
                window,
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
    use super::{next_subagent_event, parse_idle_secs, SubagentNext, DEFAULT_SUBAGENT_IDLE_SECS};

    #[test]
    fn idle_window_default_override_and_disable() {
        assert_eq!(
            parse_idle_secs(None),
            Some(std::time::Duration::from_secs(DEFAULT_SUBAGENT_IDLE_SECS))
        );
        assert_eq!(
            parse_idle_secs(Some("45")),
            Some(std::time::Duration::from_secs(45))
        );
        assert_eq!(parse_idle_secs(Some("0")), None);
        assert_eq!(
            parse_idle_secs(Some("junk")),
            Some(std::time::Duration::from_secs(DEFAULT_SUBAGENT_IDLE_SECS))
        );
    }

    #[tokio::test]
    async fn a_silent_worker_stream_hits_the_idle_window() {
        let mut stream = futures::stream::pending::<u8>();
        let out =
            next_subagent_event(&mut stream, None, Some(std::time::Duration::from_millis(30))).await;
        assert!(matches!(out, SubagentNext::Idle(d) if d.as_millis() == 30));
    }

    #[tokio::test]
    async fn cancellation_stops_waiting_on_a_stuck_worker_stream() {
        let mut stream = futures::stream::pending::<u8>();
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let out = next_subagent_event(&mut stream, Some(&token), None).await;
        assert!(matches!(out, SubagentNext::Cancelled));
    }

    #[tokio::test]
    async fn a_live_worker_stream_yields_events_then_ends() {
        let mut stream = futures::stream::iter(vec![7u8]);
        assert!(matches!(
            next_subagent_event(&mut stream, None, None).await,
            SubagentNext::Event(7)
        ));
        assert!(matches!(
            next_subagent_event(&mut stream, None, None).await,
            SubagentNext::End
        ));
    }
    use super::{
        build_subagent_agent, create_tool_notification, preflight_first_request,
        runtime_outcome_error, SUBAGENT_TOOL_REQUEST_TYPE,
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

    /// Stands in for a local model whose window the project hints push the
    /// worker's first request past: it knows its window, and its pre-send
    /// check refuses any system prompt that carries [`HINTS_MARKER`].
    struct HintSensitiveProvider {
        model_config: crate::model::ModelConfig,
        seen_system: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    }

    const HINTS_MARKER: &str = "SUBAGENT-HANDLER-TEST-PROJECT-HINTS";

    #[async_trait::async_trait]
    impl crate::providers::base::Provider for HintSensitiveProvider {
        fn get_name(&self) -> &str {
            "hint-sensitive-mock"
        }

        fn get_model_config(&self) -> crate::model::ModelConfig {
            self.model_config.clone()
        }

        async fn stream(
            &self,
            _model_config: &crate::model::ModelConfig,
            _session_id: &str,
            _system: &str,
            _messages: &[crate::conversation::message::Message],
            _tools: &[rmcp::model::Tool],
        ) -> Result<crate::providers::base::MessageStream, crate::providers::errors::ProviderError>
        {
            Err(crate::providers::errors::ProviderError::ExecutionError(
                "the pre-send check must not send anything".to_string(),
            ))
        }

        async fn request_context_window(
            &self,
            _model_config: &crate::model::ModelConfig,
        ) -> Option<usize> {
            Some(4096)
        }

        async fn preflight_request(
            &self,
            _model_config: &crate::model::ModelConfig,
            system: &str,
            _messages: &[crate::conversation::message::Message],
            _tools: &[rmcp::model::Tool],
        ) -> Result<(), crate::providers::errors::ProviderError> {
            *self.seen_system.lock().unwrap() = Some(system.to_string());
            if system.contains(HINTS_MARKER) {
                Err(
                    crate::providers::errors::ProviderError::ContextLengthExceeded(
                        "{\"n_prompt_tokens\": 6361, \"n_ctx\": 4096}".to_string(),
                    ),
                )
            } else {
                Ok(())
            }
        }
    }

    /// The reported gap: the worker's pre-send check measured the rendered
    /// subagent template, while the reply loop's first send also carries the
    /// project hints, saved skills, model line and overlay. A project whose
    /// `AGENTS.md` pushes the real first request past a local window passed
    /// the check, and the send was then refused inside the reply loop. The
    /// check now judges the prompt the loop builds, and the overhead routing
    /// reads for this worker's surface counts the hints too.
    #[tokio::test]
    async fn the_first_request_check_measures_the_prompt_the_reply_loop_sends() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let hints = format!(
            "{HINTS_MARKER}\n{}",
            "Run the whole test suite before every commit. ".repeat(400)
        );
        std::fs::write(project.join("AGENTS.md"), &hints).unwrap();
        let sm = std::sync::Arc::new(crate::session::SessionManager::new(
            tmp.path().join("sessions"),
        ));
        let session = sm
            .create_session(
                project.clone(),
                "first-request check".to_string(),
                crate::session::SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        let config = AgentRunnerConfig::new(
            sm,
            PermissionManager::instance(),
            None,
            GooseMode::Auto,
            true,
            GoosePlatform::GooseCli,
        );
        let agent = build_subagent_agent(config);
        let seen_system = std::sync::Arc::new(std::sync::Mutex::new(None));
        let provider: std::sync::Arc<dyn crate::providers::base::Provider> =
            std::sync::Arc::new(HintSensitiveProvider {
                model_config: crate::model::ModelConfig::new_or_fail("hint-sensitive-local"),
                seen_system: seen_system.clone(),
            });
        agent
            .update_provider(provider.clone(), &session.id)
            .await
            .unwrap();
        agent
            .override_system_prompt("You are a goal worker. Do the task you are given.".to_string())
            .await;
        let task_config = crate::agents::subagent_task_config::TaskConfig::new(
            provider,
            &session.id,
            &session.working_dir,
            Vec::new(),
        );
        let persona = "Persona block for the first-request check test.";

        let error = preflight_first_request(
            &agent,
            &task_config,
            &session.id,
            &crate::conversation::message::Message::user().with_text("Goal: fix the flaky test"),
            true,
            Some(persona),
        )
        .await
        .expect_err("the project hints push the real first request past the window");
        assert!(error.to_string().contains("context_exceeded"), "{error}");
        let system = seen_system
            .lock()
            .unwrap()
            .clone()
            .expect("the check judged a system prompt");
        assert!(
            system.contains("You are a goal worker."),
            "the worker's own template is still the base: {system}"
        );
        assert!(
            system.contains(HINTS_MARKER),
            "the check must judge the project hints the send carries: {system}"
        );

        let project_hints =
            crate::cost_router::fallback::worker_project_hints(&session.working_dir);
        let surface = crate::cost_router::fallback::WorkerSurface {
            compact_profile: true,
            persona_block: Some(persona),
            extensions: &[],
            provider: "hint-sensitive-mock",
            model: "hint-sensitive-local",
            project_hints: &project_hints,
        };
        let overhead = crate::cost_router::fallback::worker_first_request_overhead(&surface.key())
            .expect("measured for this worker's surface");
        let counter = crate::token_counter::create_token_counter().await.unwrap();
        assert!(
            overhead > counter.count_tokens(&hints),
            "the measured overhead ({overhead}) must count the hints the prompt carries"
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
