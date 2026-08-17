use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures::stream::BoxStream;
use futures::{stream, Stream, StreamExt, TryStreamExt};
use tracing_futures::Instrument;
use uuid::Uuid;

use super::container::Container;
use super::final_output_tool::FinalOutputTool;
use super::platform_tools;
use super::tool_confirmation_router::ToolConfirmationRouter;
use super::tool_execution::{ToolCallResult, CHAT_MODE_TOOL_SKIPPED_RESPONSE, DECLINED_RESPONSE};
use crate::action_required_manager::ActionRequiredManager;
use crate::agents::extension::{ExtensionConfig, ExtensionResult, ToolInfo};
use crate::agents::extension_manager::{
    get_parameter_names, ExtensionManager, ExtensionManagerCapabilities,
};
use crate::agents::final_output_tool::{FINAL_OUTPUT_CONTINUATION_MESSAGE, FINAL_OUTPUT_TOOL_NAME};
use crate::agents::platform_extensions::MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE;
use crate::agents::platform_tools::PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME;
use crate::agents::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
use crate::agents::prompt_manager::PromptManager;
use crate::agents::retry::{RetryManager, RetryResult};
use crate::agents::types::{FrontendTool, SessionConfig, SharedProvider, ToolResultReceiver};
use crate::config::permission::PermissionManager;
use crate::config::{get_enabled_extensions, Config, GooseMode};
use crate::context_mgmt::{
    check_if_compaction_needed, compact_messages, DEFAULT_COMPACTION_THRESHOLD,
};
use crate::conversation::message::{
    ActionRequiredData, Message, MessageContent, ProviderMetadata, SystemNotificationType,
    ToolRequest,
};
use crate::conversation::{debug_conversation_fix, fix_conversation, Conversation};
use crate::cost_router::cache::SystemPromptParts;
use crate::mcp_utils::ToolResult;
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;
use crate::permission::PermissionConfirmation;
use crate::providers::base::{PermissionRouting, Provider};
use crate::providers::errors::ProviderError;
use crate::recipe::{Author, Recipe, Response, Settings};
use crate::scheduler_trait::SchedulerTrait;
use crate::security::adversary_inspector::AdversaryInspector;
use crate::security::egress_inspector::EgressInspector;
use crate::security::security_inspector::SecurityInspector;
use crate::security::write_jail::WriteJailInspector;
use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
use crate::session::{Session, SessionManager};
use crate::tool_inspection::ToolInspectionManager;
use crate::tool_monitor::{assess_monologue, LoopAction, ProgressMonitor};
use crate::utils::is_token_cancelled;
use regex::Regex;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorCode, ErrorData, GetPromptResult, Prompt,
    ServerNotification, Tool,
};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

// Runaway-loop safety (non-negotiable B), signal S7: the interactive turn
// budget is the last-resort backstop that ALWAYS bounds a loop. It was 1000 —
// at ~$0.05/turn that is ~$50 before it trips, far too loose to be a real
// guard. Lowered to 50 (dispatched subagents already cap at 25). The
// ProgressMonitor (S1–S5) catches most loops far earlier and more precisely;
// this cap is the floor. Overridable per-session (`max_turns`) or via
// `GOOSE_MAX_TURNS` for the rare legitimately-long autonomous run.
const DEFAULT_MAX_TURNS: u32 = 50;
/// Consecutive turns whose tool calls could not be parsed, before giving up.
/// Two: one retry with the error echoed back fixes the ordinary malformed
/// call; a model that cannot recover twice will not recover on the third.
///
/// CONSECUTIVE is the load-bearing word. The counter answers exactly one
/// question — "did the model fail to recover from the parse error I just
/// showed it?" — so a turn that parses clears it. A running total instead
/// meant two unrelated malformed calls forty turns apart ended the session,
/// which is the failure this guard was written to remove.
const MAX_CONSECUTIVE_PARSE_FAILURE_TURNS: u32 = 2;

/// Characters of the parse error echoed back to the model. The error embeds
/// the model's own raw argument blob, which can run to hundreds of KB on a
/// truncated call; unbounded, it is persisted into the conversation and
/// re-sent on every subsequent turn. The log keeps the full text.
const MAX_PARSE_ERROR_ECHO_CHARS: usize = 2_000;

const COMPACTION_THINKING_TEXT: &str = "goose is compacting the conversation...";

/// The recovery observation handed back after an unparseable tool call.
///
/// Elides the MIDDLE rather than the tail: the dominant failure mode is a
/// response cut off by the output window, so the malformed region is at the
/// END of the blob — head-only truncation would keep the harmless opening and
/// discard the one part the model needs in order to correct itself.
fn parse_recovery_text(parse_err: &str) -> String {
    const HEAD: usize = 1_200;
    const TAIL: usize = MAX_PARSE_ERROR_ECHO_CHARS - HEAD;
    let total = parse_err.chars().count();
    let shown = if total > MAX_PARSE_ERROR_ECHO_CHARS {
        let head: String = parse_err.chars().take(HEAD).collect();
        let tail: String = parse_err.chars().skip(total - TAIL).collect();
        format!(
            "{head}\n[… truncated: {} of {total} characters omitted …]\n{tail}",
            total - MAX_PARSE_ERROR_ECHO_CHARS
        )
    } else {
        parse_err.to_string()
    };
    format!(
        "Your last tool call could not be parsed and did not run.\n\n\
         Parse error: {shown}\n\n\
         Nothing was executed. Re-issue exactly one tool call, with valid JSON \
         arguments matching the tool's schema. If the arguments were long, \
         shorten them or split the work into smaller calls."
    )
}

fn redacted_tool_input_summary(
    tool_name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
) -> String {
    crate::privacy::redact(
        &serde_json::json!({
            "tool": tool_name,
            "arguments": arguments,
        })
        .to_string(),
    )
}

/// Best-effort error text from a tool result that carries `is_error`.
///
/// The failure is in the CONTENT, not the transport, so there is no `Err` to
/// stringify. Falls back to a named placeholder rather than an empty string —
/// `tasks.error_message` is a diagnostic column and a blank one is the same
/// error-dilution this whole change exists to remove.
fn tool_error_text(result: &rmcp::model::CallToolResult) -> String {
    let joined = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = joined.trim();
    if trimmed.is_empty() {
        "tool reported is_error with no text content".to_string()
    } else {
        trimmed.chars().take(2000).collect()
    }
}

fn tool_task_description(
    tool_name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
) -> String {
    let Some(arguments) = arguments else {
        return tool_name.to_string();
    };

    let detail = if let Some(command) = arguments.get("command").and_then(Value::as_str) {
        command.chars().take(80).collect::<String>()
    } else if let Some((key, value)) = arguments
        .iter()
        .find_map(|(key, value)| value.as_str().map(|value| (key, value)))
    {
        format!("{key}={}", value.chars().take(60).collect::<String>())
    } else {
        return tool_name.to_string();
    };

    format!("{tool_name}: {}", crate::privacy::redact(&detail))
}

/// Context needed for the reply function
pub struct ReplyContext {
    pub conversation: Conversation,
    pub tools: Vec<Tool>,
    pub toolshim_tools: Vec<Tool>,
    pub system_prompt: SystemPromptParts,
    pub goose_mode: GooseMode,
    pub tool_call_cut_off: usize,
    pub initial_messages: Vec<Message>,
}

pub struct ToolCategorizeResult {
    pub frontend_requests: Vec<ToolRequest>,
    pub remaining_requests: Vec<ToolRequest>,
    pub filtered_response: Message,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ExtensionLoadResult {
    pub name: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum GoosePlatform {
    GooseDesktop,
    GooseCli,
}

impl fmt::Display for GoosePlatform {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GoosePlatform::GooseCli => write!(f, "goose-cli"),
            GoosePlatform::GooseDesktop => write!(f, "goose-desktop"),
        }
    }
}

#[derive(Clone)]
pub struct AgentRunnerConfig {
    pub session_manager: Arc<SessionManager>,
    pub permission_manager: Arc<PermissionManager>,
    pub scheduler_service: Option<Arc<dyn SchedulerTrait>>,
    pub goose_mode: GooseMode,
    pub disable_session_naming: bool,
    pub goose_platform: GoosePlatform,
}

impl AgentRunnerConfig {
    pub fn new(
        session_manager: Arc<SessionManager>,
        permission_manager: Arc<PermissionManager>,
        scheduler_service: Option<Arc<dyn SchedulerTrait>>,
        goose_mode: GooseMode,
        disable_session_naming: bool,
        goose_platform: GoosePlatform,
    ) -> Self {
        Self {
            session_manager,
            permission_manager,
            scheduler_service,
            goose_mode,
            disable_session_naming,
            goose_platform,
        }
    }
}

/// The main goose Agent
pub struct Agent {
    pub(super) provider: SharedProvider,
    pub config: AgentRunnerConfig,
    pub(super) current_goose_mode: Mutex<GooseMode>,
    /// True for agents with no interactive approver (scheduled-recipe jobs).
    /// A headless agent must NEVER park on tool approval — nobody exists to
    /// answer, so a park is a permanent hang — and must NEVER file a
    /// `tool_approval` Decision-Inbox row. Instead, approval-required tools
    /// are auto-DENIED with a recorded skip (already always-allowed tools run
    /// normally). See `tool_execution.rs` for both enforcement points.
    pub(super) headless: std::sync::atomic::AtomicBool,

    pub extension_manager: Arc<ExtensionManager>,
    pub(super) final_output_tool: Arc<Mutex<Option<FinalOutputTool>>>,
    pub(super) frontend_tools: Mutex<HashMap<String, FrontendTool>>,
    pub(super) frontend_instructions: Mutex<Option<String>>,
    pub(super) prompt_manager: Mutex<PromptManager>,
    pub tool_confirmation_router: ToolConfirmationRouter,
    pub(super) tool_result_tx: mpsc::Sender<(String, ToolResult<CallToolResult>)>,
    pub(super) tool_result_rx: ToolResultReceiver,

    pub(super) retry_manager: RetryManager,
    pub(super) tool_inspection_manager: ToolInspectionManager,
    container: Mutex<Option<Container>>,
    /// mtime of the config file at the last extension sync. Lets the
    /// resident-agent path answer "anything new?" with a `stat` instead of a
    /// full config read + YAML parse on every request. See
    /// `sync_extensions_with_config`.
    last_extension_config_mtime: Mutex<Option<std::time::SystemTime>>,
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Message(Message),
    McpNotification((String, ServerNotification)),
    HistoryReplaced(Conversation),
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

pub enum ToolStreamItem<T> {
    Message(ServerNotification),
    Result(T),
}

pub type ToolStream =
    Pin<Box<dyn Stream<Item = ToolStreamItem<ToolResult<CallToolResult>>> + Send>>;

// tool_stream combines a stream of ServerNotifications with a future representing the
// final result of the tool call. MCP notifications are not request-scoped, but
// this lets us capture all notifications emitted during the tool call for
// simpler consumption
pub fn tool_stream<S, F>(rx: S, done: F) -> ToolStream
where
    S: Stream<Item = ServerNotification> + Send + Unpin + 'static,
    F: Future<Output = ToolResult<CallToolResult>> + Send + 'static,
{
    Box::pin(async_stream::stream! {
        tokio::pin!(done);
        let mut rx = rx;

        loop {
            tokio::select! {
                Some(msg) = rx.next() => {
                    yield ToolStreamItem::Message(msg);
                }
                r = &mut done => {
                    yield ToolStreamItem::Result(r);
                    break;
                }
            }
        }
    })
}

/// Persist an assistant message that ENDS the turn, before it is yielded.
///
/// The provider-error arms below yield a message and then `break`, so they
/// never reach the `add_message` call at the bottom of the reply loop. The
/// message therefore streamed to the client and was never saved — and the UI
/// reloads the persisted transcript on `Finish`, which replaced the streamed
/// text with a transcript that did not contain it. The net effect was that a
/// provider error rendered for one frame and then vanished: the user sent a
/// message and the chat showed nothing at all. That is the worst of both
/// worlds, because the turn genuinely failed and the only account of why was
/// discarded.
///
/// A failure to persist is warned and swallowed rather than propagated: the
/// user still needs to see the message, and turning "we could not save the
/// error" into a second error would replace one silence with another.
async fn persist_turn_ending_message(
    session_manager: &Arc<crate::session::SessionManager>,
    session_id: &str,
    message: &Message,
) {
    if let Err(e) = session_manager.add_message(session_id, message).await {
        warn!("Failed to persist the turn-ending provider message: {e}");
    }
}

impl Agent {
    pub fn new() -> Self {
        let config = Config::global();
        Self::with_config(AgentRunnerConfig::new(
            Arc::new(SessionManager::instance()),
            PermissionManager::instance(),
            None,
            config.get_goose_mode().unwrap_or_default(),
            config.get_goose_disable_session_naming().unwrap_or(false),
            GoosePlatform::GooseCli,
        ))
    }

    pub fn with_config(config: AgentRunnerConfig) -> Self {
        let (tool_tx, tool_rx) = mpsc::channel(32);
        let provider = Arc::new(Mutex::new(None));

        let goose_platform = config.goose_platform.clone();
        let initial_mode = config.goose_mode;
        let capabilities = match config.goose_platform {
            GoosePlatform::GooseDesktop => ExtensionManagerCapabilities { mcpui: true },
            GoosePlatform::GooseCli => ExtensionManagerCapabilities { mcpui: false },
        };
        let session_manager = Arc::clone(&config.session_manager);
        let session_manager_for_inspectors = Arc::clone(&config.session_manager);
        let permission_manager = Arc::clone(&config.permission_manager);
        Self {
            provider: provider.clone(),
            config,
            current_goose_mode: Mutex::new(initial_mode),
            headless: std::sync::atomic::AtomicBool::new(false),
            extension_manager: Arc::new(ExtensionManager::new(
                provider.clone(),
                session_manager,
                goose_platform.to_string(),
                capabilities,
            )),
            final_output_tool: Arc::new(Mutex::new(None)),
            frontend_tools: Mutex::new(HashMap::new()),
            frontend_instructions: Mutex::new(None),
            prompt_manager: Mutex::new(PromptManager::new()),
            tool_confirmation_router: ToolConfirmationRouter::new(),
            tool_result_tx: tool_tx,
            tool_result_rx: Arc::new(Mutex::new(tool_rx)),
            retry_manager: RetryManager::new(),
            tool_inspection_manager: Self::create_tool_inspection_manager(
                permission_manager,
                provider.clone(),
                session_manager_for_inspectors,
            ),
            container: Mutex::new(None),
            last_extension_config_mtime: Mutex::new(None),
        }
    }

    /// Create a tool inspection manager with default inspectors
    fn create_tool_inspection_manager(
        permission_manager: Arc<PermissionManager>,
        provider: SharedProvider,
        session_manager: Arc<SessionManager>,
    ) -> ToolInspectionManager {
        let mut tool_inspection_manager = ToolInspectionManager::new();

        // Add security inspector (highest priority - runs first)
        tool_inspection_manager.add_inspector(Box::new(SecurityInspector::new()));
        tool_inspection_manager.add_inspector(Box::new(EgressInspector::new()));

        // Add adversary inspector (LLM-based review, enabled by ~/.config/goose/adversary.md)
        tool_inspection_manager.add_inspector(Box::new(AdversaryInspector::new(provider.clone())));

        // Write jail (C3): file writes/edits outside the session working dir
        // (and outside the temp/worktree/config allowlist) require approval,
        // answered through the Decision Inbox like any other confirmation.
        tool_inspection_manager.add_inspector(Box::new(WriteJailInspector::new(Some(
            session_manager.clone(),
        ))));

        // Add permission inspector (medium-high priority)
        tool_inspection_manager.add_inspector(Box::new(PermissionInspector::new(
            permission_manager,
            provider,
        )));

        // Runaway-loop safety (non-negotiable B): the ProgressMonitor replaces
        // the old exact-consecutive RepetitionInspector and is ENABLED BY
        // DEFAULT. It detects stalls (S1–S4) over a rolling per-session window,
        // blocks the offending call (L1), and — with the session manager for
        // pool access — escalates a stuck goal worker to the Decision Inbox
        // (L3), preserving its work.
        tool_inspection_manager
            .add_inspector(Box::new(ProgressMonitor::new(Some(session_manager))));

        tool_inspection_manager
    }

    /// Reset the retry attempts counter to 0
    pub async fn reset_retry_attempts(&self) {
        self.retry_manager.reset_attempts().await;
    }

    /// Increment the retry attempts counter and return the new value
    pub async fn increment_retry_attempts(&self) -> u32 {
        self.retry_manager.increment_attempts().await
    }

    /// Get the current retry attempts count
    pub async fn get_retry_attempts(&self) -> u32 {
        self.retry_manager.get_attempts().await
    }

    async fn handle_retry_logic(
        &self,
        messages: &mut Conversation,
        session_config: &SessionConfig,
        initial_messages: &[Message],
    ) -> Result<bool> {
        let result = self
            .retry_manager
            .handle_retry_logic(
                messages,
                session_config,
                initial_messages,
                &self.final_output_tool,
            )
            .await?;

        match result {
            RetryResult::Retried => Ok(true),
            RetryResult::Skipped
            | RetryResult::MaxAttemptsReached
            | RetryResult::SuccessChecksPassed => Ok(false),
        }
    }
    async fn drain_elicitation_messages(&self, session_id: &str) -> Vec<Message> {
        let mut messages = Vec::new();
        let manager = self.config.session_manager.clone();
        let mut elicitation_rx = ActionRequiredManager::global().request_rx.lock().await;
        while let Ok(mut elicitation_message) = elicitation_rx.try_recv() {
            if elicitation_message.id.is_none() {
                elicitation_message = elicitation_message.with_generated_id();
            }
            if let Err(e) = manager.add_message(session_id, &elicitation_message).await {
                warn!("Failed to save elicitation message to session: {}", e);
            }
            messages.push(elicitation_message);
        }
        messages
    }

    async fn prepare_reply_context(
        &self,
        session_id: &str,
        unfixed_conversation: Conversation,
        working_dir: &std::path::Path,
    ) -> Result<ReplyContext> {
        let unfixed_messages = unfixed_conversation.messages().clone();
        let (conversation, issues) = fix_conversation(unfixed_conversation.clone());
        if !issues.is_empty() {
            debug!(
                "Conversation issue fixed: {}",
                debug_conversation_fix(
                    unfixed_messages.as_slice(),
                    conversation.messages(),
                    &issues
                )
            );
        }
        let initial_messages = conversation.messages().clone();

        let (tools, toolshim_tools, system_prompt) = self
            .prepare_tools_and_prompt(session_id, working_dir)
            .await?;

        let goose_mode = *self.current_goose_mode.lock().await;

        if goose_mode == GooseMode::SmartApprove {
            self.tool_inspection_manager.apply_tool_annotations(&tools);
        }

        let tool_call_cut_off = match Config::global().get_param::<usize>("GOOSE_TOOL_CALL_CUTOFF")
        {
            Ok(v) => v,
            Err(_) => {
                let context_limit = self
                    .provider()
                    .await
                    .map(|p| p.get_model_config().context_limit())
                    .unwrap_or(crate::model::DEFAULT_CONTEXT_LIMIT);
                let compaction_threshold = Config::global()
                    .get_param::<f64>("GOOSE_AUTO_COMPACT_THRESHOLD")
                    .unwrap_or(crate::context_mgmt::DEFAULT_COMPACTION_THRESHOLD);
                crate::context_mgmt::compute_tool_call_cutoff(context_limit, compaction_threshold)
            }
        };

        Ok(ReplyContext {
            conversation,
            tools,
            toolshim_tools,
            system_prompt,
            goose_mode,
            tool_call_cut_off,
            initial_messages,
        })
    }

    async fn categorize_tools(
        &self,
        response: &Message,
        tools: &[rmcp::model::Tool],
        suppress_replayed_thinking: bool,
    ) -> ToolCategorizeResult {
        // Categorize tool requests
        let (frontend_requests, remaining_requests, filtered_response) = self
            .categorize_tool_requests(response, tools, suppress_replayed_thinking)
            .await;

        ToolCategorizeResult {
            frontend_requests,
            remaining_requests,
            filtered_response,
        }
    }

    async fn handle_approved_and_denied_tools(
        &self,
        permission_check_result: &PermissionCheckResult,
        request_to_response_map: &mut HashMap<String, Message>,
        cancel_token: Option<tokio_util::sync::CancellationToken>,
        session: &Session,
        inspection_results: &[crate::tool_inspection::InspectionResult],
    ) -> Result<Vec<(String, ToolStream)>> {
        let mut tool_futures: Vec<(String, ToolStream)> = Vec::new();

        // Handle pre-approved and read-only tools
        for request in &permission_check_result.approved {
            if let Ok(tool_call) = request.tool_call.clone() {
                let (req_id, tool_result) = self
                    .dispatch_tool_call(
                        tool_call,
                        request.id.clone(),
                        cancel_token.clone(),
                        session,
                    )
                    .await;

                tool_futures.push((
                    req_id,
                    match tool_result {
                        Ok(result) => tool_stream(
                            result
                                .notification_stream
                                .unwrap_or_else(|| Box::new(stream::empty())),
                            result.result,
                        ),
                        Err(e) => {
                            tool_stream(Box::new(stream::empty()), futures::future::ready(Err(e)))
                        }
                    },
                ));
            }
        }

        Self::handle_denied_tools(
            permission_check_result,
            request_to_response_map,
            inspection_results,
        );
        Ok(tool_futures)
    }

    /// A ProgressMonitor block (L1/L3) carries an actionable "try a different
    /// approach" reason. Surface it to the model in place of the generic
    /// declined text so it can self-correct — the whole point of the L1 nudge.
    /// Every other denial keeps `DECLINED_RESPONSE`.
    fn progress_monitor_block_reason(
        request_id: &str,
        inspection_results: &[crate::tool_inspection::InspectionResult],
    ) -> Option<String> {
        inspection_results
            .iter()
            .find(|r| {
                r.tool_request_id == request_id
                    && r.inspector_name == crate::tool_monitor::PROGRESS_MONITOR_NAME
                    && r.action == crate::tool_inspection::InspectionAction::Deny
            })
            .map(|r| r.reason.clone())
    }

    fn handle_denied_tools(
        permission_check_result: &PermissionCheckResult,
        request_to_response_map: &mut HashMap<String, Message>,
        inspection_results: &[crate::tool_inspection::InspectionResult],
    ) {
        for request in &permission_check_result.denied {
            if let Some(response) = request_to_response_map.get_mut(&request.id) {
                let text = Self::progress_monitor_block_reason(&request.id, inspection_results)
                    .unwrap_or_else(|| DECLINED_RESPONSE.to_string());
                response.add_tool_response_with_metadata(
                    request.id.clone(),
                    Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                        text,
                    )])),
                    request.metadata.as_ref(),
                );
            }
        }
    }

    /// Get a reference count clone to the provider
    pub async fn provider(&self) -> Result<Arc<dyn Provider>, anyhow::Error> {
        match &*self.provider.lock().await {
            Some(provider) => Ok(Arc::clone(provider)),
            None => Err(anyhow!("Provider not set")),
        }
    }

    /// When set, all stdio extensions will be started via `docker exec` in the specified container.
    pub async fn set_container(&self, container: Option<Container>) {
        *self.container.lock().await = container.clone();
    }

    pub async fn container(&self) -> Option<Container> {
        self.container.lock().await.clone()
    }

    /// Check if a tool is a frontend tool
    pub async fn is_frontend_tool(&self, name: &str) -> bool {
        self.frontend_tools.lock().await.contains_key(name)
    }

    /// Get a reference to a frontend tool
    pub async fn get_frontend_tool(&self, name: &str) -> Option<FrontendTool> {
        self.frontend_tools.lock().await.get(name).cloned()
    }

    /// Install the recipe final-output tool. Fails (instead of panicking the
    /// daemon) when the recipe's `response.json_schema` is missing, not a
    /// non-empty JSON object, or fails JSON Schema meta-validation.
    pub async fn add_final_output_tool(&self, response: Response) -> anyhow::Result<()> {
        let created_final_output_tool = FinalOutputTool::new(response)?;
        let final_output_system_prompt = created_final_output_tool.system_prompt();
        let mut final_output_tool = self.final_output_tool.lock().await;
        *final_output_tool = Some(created_final_output_tool);
        drop(final_output_tool);
        self.extend_system_prompt("final_output".to_string(), final_output_system_prompt)
            .await;
        Ok(())
    }

    pub async fn apply_recipe_components(
        &self,
        response: Option<Response>,
        include_final_output: bool,
    ) -> anyhow::Result<()> {
        if include_final_output {
            if let Some(response) = response {
                self.add_final_output_tool(response).await?;
            }
        }
        Ok(())
    }

    /// Dispatch a single tool call to the appropriate client
    #[instrument(skip(self, tool_call, request_id, cancellation_token, session), fields(input, output, session.id = %session.id))]
    pub async fn dispatch_tool_call(
        &self,
        tool_call: CallToolRequestParams,
        request_id: String,
        cancellation_token: Option<CancellationToken>,
        session: &Session,
    ) -> (String, Result<ToolCallResult, ErrorData>) {
        let input_summary =
            redacted_tool_input_summary(&tool_call.name, tool_call.arguments.as_ref());
        tracing::Span::current().record("input", tracing::field::display(&input_summary));
        // Redacted args for task-completion logging (never the raw arguments,
        // which can carry secrets — see the redacted summary above).
        let args_value: Option<serde_json::Value> =
            Some(serde_json::json!({ "input": input_summary.clone() }));

        self.prompt_manager
            .lock()
            .await
            .record_tool_arguments(&tool_call.arguments, &session.working_dir);

        if tool_call.name == PLATFORM_MANAGE_SCHEDULE_TOOL_NAME {
            let arguments = tool_call
                .arguments
                .map(Value::Object)
                .unwrap_or(Value::Object(serde_json::Map::new()));
            let result = self
                .handle_schedule_management(arguments, request_id.clone())
                .await;
            let wrapped_result = result.map(CallToolResult::success);
            return (request_id, Ok(ToolCallResult::from(wrapped_result)));
        }

        if tool_call.name == PLATFORM_LOAD_FEATURE_LESSON_TOOL_NAME {
            let feature_id = tool_call
                .arguments
                .as_ref()
                .and_then(|a| a.get("feature_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let result = self.handle_load_feature_lesson(&feature_id);
            let wrapped_result = result.map(CallToolResult::success);
            return (request_id, Ok(ToolCallResult::from(wrapped_result)));
        }

        if tool_call.name == FINAL_OUTPUT_TOOL_NAME {
            return if let Some(final_output_tool) = self.final_output_tool.lock().await.as_mut() {
                let result = final_output_tool.execute_tool_call(tool_call.clone()).await;
                (request_id, Ok(result))
            } else {
                (
                    request_id,
                    Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        "Final output tool not defined".to_string(),
                        None,
                    )),
                )
            };
        }

        let ctx = super::tool_execution::ToolCallContext::new(
            session.id.clone(),
            Some(session.working_dir.clone()),
            Some(request_id.clone()),
        )
        .with_model(
            self.provider()
                .await
                .ok()
                .map(|p| p.get_model_config().model_name.clone()),
        );

        debug!("WAITING_TOOL_START: {}", tool_call.name);

        // Task logging: log_task_created before, log_task_completed/failed after
        let tool_name_str = tool_call.name.to_string();
        let task_description = tool_task_description(&tool_name_str, tool_call.arguments.as_ref());
        let task_id = if let Some(logger) = crate::tasks::global() {
            let tid = logger
                .log_task_created(&task_description, Some(&tool_name_str))
                .await;
            logger.log_task_started(&tid, &session.id).await;
            Some(tid)
        } else {
            None
        };
        let task_start = std::time::Instant::now();

        let result: ToolCallResult = if self.is_frontend_tool(&tool_call.name).await {
            ToolCallResult::from(Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Frontend tool execution required".to_string(),
                None,
            )))
        } else {
            let result = self
                .extension_manager
                .dispatch_tool_call(
                    &ctx,
                    tool_call.clone(),
                    cancellation_token.unwrap_or_default(),
                )
                .await;
            result.unwrap_or_else(|e| {
                #[cfg(feature = "telemetry")]
                crate::posthog::emit_error(
                    "tool_execution_failed",
                    &format!("{}: {}", tool_call.name, e),
                );
                let error_data = e.downcast::<ErrorData>().unwrap_or_else(|e| {
                    ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None)
                });
                ToolCallResult::from(Err(error_data))
            })
        };

        // C3 injection posture (layer 1): results from untrusted-origin tools
        // (web fetch/reader, browser content bridges, third-party feeds) carry
        // the data-not-instructions frame before they enter the conversation.
        // Trusted tools (the user's own files, Brain recall) pass unchanged.
        let result = crate::security::untrusted_content::apply_untrusted_result_framing(
            tool_call.name.as_ref(),
            result,
        );

        // Auto-skills: repetition detection keys on the argument SHAPE, which is
        // known at dispatch and does not depend on how the call turns out.
        // Deliberately left here rather than moved into the resolved future,
        // which cannot hold `&self`.
        if task_id.is_some() {
            if let Some(logger) = crate::tasks::global() {
                let skills_config = crate::tasks::SkillsConfig::from_config();
                if let Some(proposal_prompt) =
                    logger.check_repetition_candidates(&skills_config).await
                {
                    self.extend_system_prompt("skill_proposals".to_string(), proposal_prompt)
                        .await;
                }
            }
        }

        debug!("WAITING_TOOL_END: {}", tool_call.name);

        // Task outcome is recorded when the result RESOLVES, from the result
        // itself.
        //
        // This used to log `completed` unconditionally at dispatch, before the
        // boxed future had run, with an empty output blob — annotated in-source
        // as a Phase 1 trade-off. The consequence reached much further than the
        // tasks table: `log_task_completed` drives
        // `recognition::write_back_task_outcome`, which stamps `Positive` over
        // every still-unattributed recall in the session. So a failing tool call
        // recorded a success, and `recognition_events.outcome_label` — the
        // useful/ignored/wrong column that is the closest thing to ground truth
        // about whether recall helped — could essentially never become `wrong`.
        // Any evaluation or training signal built on that label was reading a
        // constant.
        //
        // A tool call counts as failed when the future errors OR the result
        // carries `is_error` (the MCP-level failure the transport reports as a
        // successful round-trip — the case a naive `Result` check misses).
        let task_outcome = task_id.map(|tid| (tid, tool_name_str.clone(), args_value.clone()));
        // Split the stream off before the future moves into the async block.
        let ToolCallResult {
            notification_stream,
            result: result_future,
        } = result;
        let resolved = async move {
            let out = result_future.await;
            let out = super::large_response_handler::process_tool_response(out);

            if let Some((tid, tool_name, args)) = task_outcome {
                if let Some(logger) = crate::tasks::global() {
                    let duration_ms = task_start.elapsed().as_millis() as u64;
                    match &out {
                        Ok(call_result) if call_result.is_error != Some(true) => {
                            logger
                                .log_task_completed(
                                    &tid,
                                    Some(&tool_name),
                                    args.as_ref(),
                                    &serde_json::json!({}),
                                    duration_ms,
                                )
                                .await;
                        }
                        Ok(call_result) => {
                            logger
                                .log_task_failed_with_shape(
                                    &tid,
                                    Some(&tool_name),
                                    args.as_ref(),
                                    &tool_error_text(call_result),
                                )
                                .await;
                        }
                        Err(e) => {
                            logger
                                .log_task_failed_with_shape(
                                    &tid,
                                    Some(&tool_name),
                                    args.as_ref(),
                                    &e.to_string(),
                                )
                                .await;
                        }
                    }
                }
            }
            out
        };

        (
            request_id,
            Ok(ToolCallResult {
                notification_stream,
                // The slot wants `Unpin`; an async block is not, so pin it first
                // (`Pin<Box<F>>` is both `Future` and `Unpin`).
                result: Box::new(Box::pin(resolved)),
            }),
        )
    }

    /// Save current extension state to session metadata
    /// Should be called after any extension add/remove operation
    pub async fn save_extension_state(&self, session: &SessionConfig) -> Result<()> {
        let extension_configs = self.extension_manager.get_extension_configs().await;

        let extensions_state = EnabledExtensionsState::new(extension_configs);

        let session_manager = self.config.session_manager.clone();
        let mut session_data = session_manager.get_session(&session.id, false).await?;

        if let Err(e) = extensions_state.to_extension_data(&mut session_data.extension_data) {
            warn!("Failed to serialize extension state: {}", e);
            return Err(anyhow!("Extension state serialization failed: {}", e));
        }

        session_manager
            .update(&session.id)
            .extension_data(session_data.extension_data)
            .apply()
            .await?;

        Ok(())
    }

    /// Save current extension state to session by session_id
    pub async fn persist_extension_state(&self, session_id: &str) -> Result<()> {
        let extension_configs = self.extension_manager.get_extension_configs().await;
        let extensions_state = EnabledExtensionsState::new(extension_configs);

        let session_manager = self.config.session_manager.clone();
        let session = session_manager.get_session(session_id, false).await?;
        let mut extension_data = session.extension_data.clone();

        extensions_state
            .to_extension_data(&mut extension_data)
            .map_err(|e| anyhow!("Failed to serialize extension state: {}", e))?;

        session_manager
            .update(session_id)
            .extension_data(extension_data)
            .apply()
            .await?;

        Ok(())
    }

    /// Load extensions from session into the agent
    /// Skips extensions that are already loaded
    /// Uses the session's working_dir for extension initialization
    pub async fn load_extensions_from_session(
        self: &Arc<Self>,
        session: &Session,
    ) -> Vec<ExtensionLoadResult> {
        let config = Config::global();
        let enabled_configs =
            EnabledExtensionsState::extensions_or_default(Some(&session.extension_data), config);

        let session_id = session.id.clone();

        let extension_futures = enabled_configs
            .into_iter()
            .map(|config| {
                let config_clone = config.clone();
                let agent_ref = self.clone();
                let session_id_clone = session_id.clone();

                async move {
                    let name = config_clone.name().to_string();

                    if agent_ref
                        .extension_manager
                        .is_extension_enabled(&name)
                        .await
                    {
                        tracing::debug!("Extension {} already loaded, skipping", name);
                        return ExtensionLoadResult {
                            name,
                            success: true,
                            error: None,
                        };
                    }

                    match agent_ref
                        .add_extension_inner(config_clone, &session_id_clone)
                        .await
                    {
                        Ok(_) => ExtensionLoadResult {
                            name,
                            success: true,
                            error: None,
                        },
                        Err(e) => {
                            let error_msg = e.to_string();
                            warn!("Failed to load extension {}: {}", name, error_msg);
                            ExtensionLoadResult {
                                name,
                                success: false,
                                error: Some(error_msg),
                            }
                        }
                    }
                }
            })
            .collect::<Vec<_>>();

        let results = futures::future::join_all(extension_futures).await;

        // Persist once after all extensions are loaded
        if results.iter().any(|r| r.success) {
            if let Err(e) = self.persist_extension_state(&session_id).await {
                warn!("Failed to persist extension state after bulk load: {}", e);
            }
        }

        results
    }

    /// Bring a RESIDENT agent's extension set up to date with config.
    ///
    /// `extensions_or_default` already merges config-enabled extensions into a
    /// session's stored snapshot — but only when an agent is CONSTRUCTED. A
    /// resident agent never re-reads config, so enabling an extension in
    /// Settings does not reach a chat that is already open.
    ///
    /// Observed 2026-08-13: the user asked for tide data at 23:24 in a session
    /// created at 22:44, having added Brave and Tavily keys in between. The
    /// session's stored extension set had neither, and the agent had been
    /// resident the whole time, so no refresh ever ran. What made it a bad
    /// failure rather than a missing feature is that the model could still SEE
    /// the tool names — `search_available_extensions` reads config, not session
    /// state — so it called `tavilywebsearch__tavily_search`, got
    /// `-32002 Tool not found`, looped on `search_available_extensions` until
    /// the runaway guard blocked it, and then told the user web search wasn't
    /// available. Two working search providers, a confident tool call, and no
    /// route to recovery short of starting a new chat.
    ///
    /// Additive only: this never removes an extension, so a session that
    /// deliberately disabled one keeps that choice. Returns the keys newly
    /// registered, for logging — silence here would recreate the original bug
    /// in a quieter form.
    ///
    /// Called from the resident-agent path of `get_or_create_agent`, which
    /// every agent-fetching route hits, so it is gated on the config file's
    /// mtime. `Config::get_param` has no cache — it re-reads and re-parses the
    /// ~16 KB config (plus the defaults file) on every call — so reading the
    /// enabled set unconditionally would put two file reads and a YAML parse on
    /// every request. One `stat` answers "did anything change?" instead.
    pub async fn sync_extensions_with_config(self: &Arc<Self>, session_id: &str) -> Vec<String> {
        let config = Config::global();
        let mtime = std::fs::metadata(config.path())
            .and_then(|m| m.modified())
            .ok();
        {
            let mut last = self.last_extension_config_mtime.lock().await;
            // `None` means the stat failed; fall through and do the real work
            // rather than treating an unreadable config as "unchanged".
            if let Some(mtime) = mtime {
                if *last == Some(mtime) {
                    return Vec::new();
                }
                // Claimed BEFORE the work, not after, and the lock is released
                // here rather than held across extension startup. A second
                // request arriving mid-sync therefore returns immediately and
                // may observe a partially-synced agent — if it dispatches a tool
                // call for an extension still starting, it can see one more
                // "tool not found" before the sync lands.
                //
                // The alternatives are worse: holding this lock across startup
                // blocks every other request to the agent for as long as an MCP
                // server takes to boot, and claiming afterwards lets concurrent
                // callers race to spawn the same server twice. The window is one
                // request wide, only after a config change, and self-heals on
                // the next call.
                *last = Some(mtime);
            }
        }

        let enabled = crate::config::extensions::get_enabled_extensions_with_config(config);

        // Second gate: the file may have changed for an unrelated key, so only
        // pay for extension work when something is genuinely missing.
        let mut missing = Vec::new();
        for cfg in enabled {
            if !self
                .extension_manager
                .is_extension_enabled(&cfg.name())
                .await
            {
                missing.push(cfg);
            }
        }
        if missing.is_empty() {
            return Vec::new();
        }

        let mut added = Vec::new();
        for cfg in missing {
            let name = cfg.name().to_string();
            match self.add_extension_inner(cfg, session_id).await {
                Ok(_) => added.push(name),
                // A failure here must not break the turn — the user asked a
                // question, not to load an extension. It is logged at warn so
                // it is findable, which is more than the old path offered.
                Err(e) => warn!(
                    "Could not add newly-enabled extension {} to running session {}: {}",
                    name, session_id, e
                ),
            }
        }

        if !added.is_empty() {
            info!(
                session_id = %session_id,
                extensions = %added.join(", "),
                "Added newly-enabled extensions to a running session"
            );
            if let Err(e) = self.persist_extension_state(session_id).await {
                warn!("Failed to persist extension state after config sync: {}", e);
            }
        }
        added
    }

    pub async fn add_extension(
        &self,
        extension: ExtensionConfig,
        session_id: &str,
    ) -> ExtensionResult<()> {
        self.add_extension_inner(extension, session_id).await?;

        // Persist extension state after successful add
        self.persist_extension_state(session_id)
            .await
            .map_err(|e| {
                error!("Failed to persist extension state: {}", e);
                crate::agents::extension::ExtensionError::SetupError(format!(
                    "Failed to persist extension state: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Load multiple extensions in parallel, persisting state once at the end.
    ///
    /// Unlike `add_extension`, this avoids per-extension persistence and acquires
    /// the container lock once upfront to prevent serialisation of the parallel futures.
    pub async fn add_extensions_bulk(
        self: &Arc<Self>,
        extensions: Vec<ExtensionConfig>,
        session_id: &str,
    ) -> anyhow::Result<Vec<ExtensionLoadResult>> {
        let working_dir = match self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(session) => Some(session.working_dir),
            Err(e) => {
                warn!("Failed to get session for bulk load: {}", e);
                None
            }
        };
        let container = self.container.lock().await.clone();

        let extension_futures = extensions
            .into_iter()
            .map(|config| {
                let ext_manager = Arc::clone(&self.extension_manager);
                let working_dir = working_dir.clone();
                let container = container.clone();
                let sid = session_id.to_string();

                async move {
                    let name = config.name().to_string();
                    match ext_manager
                        .add_extension(config, working_dir, container.as_ref(), Some(&sid))
                        .await
                    {
                        Ok(_) => ExtensionLoadResult {
                            name,
                            success: true,
                            error: None,
                        },
                        Err(e) => {
                            let error_msg = e.to_string();
                            warn!("Failed to load extension {}: {}", name, error_msg);
                            ExtensionLoadResult {
                                name,
                                success: false,
                                error: Some(error_msg),
                            }
                        }
                    }
                }
            })
            .collect::<Vec<_>>();

        let results = futures::future::join_all(extension_futures).await;

        if results.iter().any(|r| r.success) {
            self.persist_extension_state(session_id).await?;
        }

        Ok(results)
    }

    async fn add_extension_inner(
        &self,
        extension: ExtensionConfig,
        session_id: &str,
    ) -> ExtensionResult<()> {
        let session = self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
            .map_err(|e| {
                crate::agents::extension::ExtensionError::SetupError(format!(
                    "Failed to get session '{}': {}",
                    session_id, e
                ))
            })?;
        let working_dir = Some(session.working_dir);

        match &extension {
            ExtensionConfig::Frontend {
                tools,
                instructions,
                ..
            } => {
                // For frontend tools, just store them in the frontend_tools map
                let mut frontend_tools = self.frontend_tools.lock().await;
                for tool in tools {
                    let frontend_tool = FrontendTool {
                        name: tool.name.to_string(),
                        tool: tool.clone(),
                    };
                    frontend_tools.insert(tool.name.to_string(), frontend_tool);
                }
                // Store instructions if provided, using "frontend" as the key
                let mut frontend_instructions = self.frontend_instructions.lock().await;
                if let Some(instructions) = instructions {
                    *frontend_instructions = Some(instructions.clone());
                } else {
                    // Default frontend instructions if none provided
                    *frontend_instructions = Some(
                        "The following tools are provided directly by the frontend and will be executed by the frontend when called.".to_string(),
                    );
                }
            }
            _ => {
                let container = self.container.lock().await;
                self.extension_manager
                    .add_extension(
                        extension.clone(),
                        working_dir,
                        container.as_ref(),
                        Some(session_id),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn list_tools(&self, session_id: &str, extension_name: Option<String>) -> Vec<Tool> {
        let mut prefixed_tools = self
            .extension_manager
            .get_prefixed_tools(session_id, extension_name.clone())
            .await
            .unwrap_or_default();

        if (extension_name.is_none() || extension_name.as_deref() == Some("platform"))
            && self.config.scheduler_service.is_some()
        {
            prefixed_tools.push(platform_tools::manage_schedule_tool());
        }

        // Guided-tour lesson loader — always available (read-only, no deps).
        if extension_name.is_none() || extension_name.as_deref() == Some("platform") {
            prefixed_tools.push(platform_tools::load_feature_lesson_tool());
        }

        if extension_name.is_none() {
            if let Some(final_output_tool) = self.final_output_tool.lock().await.as_ref() {
                prefixed_tools.push(final_output_tool.tool());
            }
        }

        prefixed_tools
    }

    pub async fn remove_extension(&self, name: &str, session_id: &str) -> Result<()> {
        self.extension_manager.remove_extension(name).await?;

        // Persist extension state after successful removal
        self.persist_extension_state(session_id)
            .await
            .map_err(|e| {
                error!("Failed to persist extension state: {}", e);
                anyhow!("Failed to persist extension state: {}", e)
            })?;

        Ok(())
    }

    pub async fn list_extensions(&self) -> Vec<String> {
        self.extension_manager
            .list_extensions()
            .await
            .expect("Failed to list extensions")
    }

    pub async fn get_extension_configs(&self) -> Vec<ExtensionConfig> {
        self.extension_manager.get_extension_configs().await
    }

    /// Mark this agent as headless: no interactive approver exists for it
    /// (scheduled-recipe jobs). Approval-required tools are auto-denied with a
    /// recorded skip instead of parking, and no `tool_approval` decision is
    /// ever filed. Deliberately explicit — headlessness is a property of how
    /// the agent is RUN (set by the runner that owns it), never an accident of
    /// mode defaults.
    pub fn set_headless(&self, headless: bool) {
        self.headless
            .store(headless, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether this agent runs without an interactive approver. See
    /// [`Agent::set_headless`].
    pub fn is_headless(&self) -> bool {
        self.headless.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Handle a confirmation response for a tool request.
    ///
    /// Returns whether a live waiter actually received it: `true` when the
    /// provider's ActionRequired routing consumed the confirmation or a parked
    /// tool call was unblocked through the `ToolConfirmationRouter`; `false`
    /// when nobody was waiting (the turn was cancelled, the daemon restarted,
    /// or the request was already answered through another channel) — in that
    /// case the confirmation had no effect and callers reporting an effect to
    /// the user must say so instead of claiming the tool ran.
    pub async fn handle_confirmation(
        &self,
        request_id: String,
        confirmation: PermissionConfirmation,
    ) -> bool {
        let provider = self.provider.lock().await.clone();
        if let Some(provider) = provider.as_ref() {
            if provider.permission_routing() == PermissionRouting::ActionRequired
                && provider
                    .handle_permission_confirmation(&request_id, &confirmation)
                    .await
            {
                return true;
            }
        }
        let delivered = self
            .tool_confirmation_router
            .deliver(request_id, confirmation)
            .await;
        if !delivered {
            // Kept for callers that ignore the return value (CLI session loop):
            // the router already warn!-ed with the request_id.
            error!("Failed to deliver confirmation");
        }
        delivered
    }

    pub async fn supports_action_required_permissions(&self) -> bool {
        if let Some(provider) = self.provider.lock().await.as_ref() {
            return provider.permission_routing() == PermissionRouting::ActionRequired;
        }
        false
    }

    #[instrument(
        skip(self, user_message, session_config, cancel_token),
        fields(user_message, trace_input, session.id = %session_config.id)
    )]
    pub async fn reply(
        &self,
        user_message: Message,
        session_config: SessionConfig,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        let session_manager = self.config.session_manager.clone();

        let message_text_for_trace = user_message.as_concat_text();
        tracing::Span::current().record("user_message", message_text_for_trace.as_str());
        tracing::Span::current().record("trace_input", message_text_for_trace.as_str());

        for content in &user_message.content {
            if let MessageContent::ActionRequired(action_required) = content {
                if let ActionRequiredData::ElicitationResponse { id, user_data } =
                    &action_required.data
                {
                    if let Err(e) = ActionRequiredManager::global()
                        .submit_response(id.clone(), user_data.clone())
                        .await
                    {
                        let error_text = format!("Failed to submit elicitation response: {}", e);
                        error!(error_text);
                        return Ok(Box::pin(stream::once(async {
                            Ok(AgentEvent::Message(
                                Message::assistant().with_text(error_text),
                            ))
                        })));
                    }
                    session_manager
                        .add_message(&session_config.id, &user_message)
                        .await?;
                    return Ok(Box::pin(futures::stream::empty()));
                }
            }
        }

        let message_text = user_message.as_concat_text();

        // Track custom slash command usage (don't track command name for privacy)
        if message_text.trim().starts_with('/') {
            let command = message_text.split_whitespace().next();
            if let Some(cmd) = command {
                if crate::slash_commands::get_recipe_for_command(cmd).is_some() {
                    #[cfg(feature = "telemetry")]
                    crate::posthog::emit_custom_slash_command_used();
                }
            }
        }

        let command_result = self
            .execute_command(&message_text, &session_config.id)
            .await;

        match command_result {
            Err(e) => {
                let error_message = Message::assistant()
                    .with_text(e.to_string())
                    .with_visibility(true, false);
                return Ok(Box::pin(stream::once(async move {
                    Ok(AgentEvent::Message(error_message))
                })));
            }
            Ok(Some(response)) if response.role == rmcp::model::Role::Assistant => {
                session_manager
                    .add_message(
                        &session_config.id,
                        &user_message.clone().with_visibility(true, false),
                    )
                    .await?;
                session_manager
                    .add_message(
                        &session_config.id,
                        &response.clone().with_visibility(true, false),
                    )
                    .await?;

                // Check if this was a command that modifies conversation history
                let modifies_history = crate::agents::execute_commands::COMPACT_TRIGGERS
                    .contains(&message_text.trim())
                    || message_text.trim() == "/clear";

                return Ok(Box::pin(async_stream::try_stream! {
                    yield AgentEvent::Message(user_message);
                    yield AgentEvent::Message(response);

                    // After commands that modify history, notify UI that history was replaced
                    if modifies_history {
                        let updated_session = session_manager.get_session(&session_config.id, true)
                            .await
                            .map_err(|e| anyhow!("Failed to fetch updated session: {}", e))?;
                        let updated_conversation = updated_session
                            .conversation
                            .ok_or_else(|| anyhow!("Session has no conversation after history modification"))?;
                        yield AgentEvent::HistoryReplaced(updated_conversation);
                    }
                }));
            }
            Ok(Some(resolved_message)) => {
                session_manager
                    .add_message(
                        &session_config.id,
                        &user_message.clone().with_visibility(true, false),
                    )
                    .await?;
                session_manager
                    .add_message(
                        &session_config.id,
                        &resolved_message.clone().with_visibility(false, true),
                    )
                    .await?;
            }
            Ok(None) => {
                session_manager
                    .add_message(&session_config.id, &user_message)
                    .await?;
            }
        }
        let session = session_manager
            .get_session(&session_config.id, true)
            .await?;
        let conversation = session
            .conversation
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Session {} has no conversation", session_config.id))?;

        let needs_auto_compact = check_if_compaction_needed(
            self.provider().await?.as_ref(),
            &conversation,
            None,
            &session,
        )
        .await?;

        let conversation_to_compact = conversation.clone();

        Ok(Box::pin(async_stream::try_stream! {
            let final_conversation = if !needs_auto_compact {
                conversation
            } else {
                let config = Config::global();
                let threshold = config
                    .get_param::<f64>("GOOSE_AUTO_COMPACT_THRESHOLD")
                    .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);
                let threshold_percentage = (threshold * 100.0) as u32;

                let inline_msg = format!(
                    "Exceeded auto-compact threshold of {}%. Performing auto-compaction...",
                    threshold_percentage
                );

                yield AgentEvent::Message(
                    Message::assistant().with_system_notification(
                        SystemNotificationType::InlineMessage,
                        inline_msg,
                    )
                );

                yield AgentEvent::Message(
                    Message::assistant().with_system_notification(
                        SystemNotificationType::ThinkingMessage,
                        COMPACTION_THINKING_TEXT,
                    )
                );

                match compact_messages(
                    self.provider().await?.as_ref(),
                    &session_config.id,
                    &conversation_to_compact,
                    false,
                )
                .await
                {
                    Ok((compacted_conversation, summarization_usage)) => {
                        session_manager.replace_conversation(&session_config.id, &compacted_conversation).await?;
                        self.update_session_metrics(&session_config.id, session_config.schedule_id.clone(), &summarization_usage, true).await?;

                        yield AgentEvent::HistoryReplaced(compacted_conversation.clone());

                        yield AgentEvent::Message(
                            Message::assistant().with_system_notification(
                                SystemNotificationType::InlineMessage,
                                "Compaction complete",
                            )
                        );

                        compacted_conversation
                    }
                    Err(e) => {
                        yield AgentEvent::Message(
                            Message::assistant().with_text(
                                format!("Ran into this error trying to compact: {e}.\n\nPlease try again or create a new session")
                            )
                        );
                        return;
                    }
                }
            };

            let mut reply_stream = self.reply_internal(final_conversation, session_config, session, cancel_token).await?;
            while let Some(event) = reply_stream.next().await {
                yield event?;
            }
        }))
    }

    async fn reply_internal(
        &self,
        conversation: Conversation,
        session_config: SessionConfig,
        session: Session,
        cancel_token: Option<CancellationToken>,
    ) -> Result<BoxStream<'_, Result<AgentEvent>>> {
        // #348 — real agent lifecycle hook for the World View. This Drop guard ties
        // the primary agent's runtime state to the ACTUAL reply turn: `working` on
        // entry, and on drop `error` (a real provider failure that set the latch) or
        // else `done` → available. Covers every exit path — clean break, max-turns,
        // cancellation, panic. The `/api/henry/status` poll and the agent-state tick
        // read this registry, so a real error survives instead of being clobbered by
        // the 2s session-activity derive. Replaces the #288 interim-A simulated error.
        struct ReplyStateGuard {
            errored: bool,
        }
        impl ReplyStateGuard {
            fn start() -> Self {
                crate::events::record_agent_working("henry");
                Self { errored: false }
            }
            fn mark_error(&mut self) {
                self.errored = true;
            }
        }
        impl Drop for ReplyStateGuard {
            fn drop(&mut self) {
                if self.errored {
                    crate::events::record_agent_error("henry");
                } else {
                    crate::events::record_agent_done("henry");
                }
            }
        }

        let context = self
            .prepare_reply_context(&session.id, conversation, session.working_dir.as_path())
            .await?;
        let ReplyContext {
            mut conversation,
            mut tools,
            mut toolshim_tools,
            mut system_prompt,
            tool_call_cut_off,
            goose_mode,
            initial_messages,
        } = context;
        self.reset_retry_attempts().await;

        let provider = self.provider().await?;
        let session_manager = self.config.session_manager.clone();
        let session_id = session_config.id.clone();
        if !self.config.disable_session_naming {
            let manager_for_spawn = session_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = manager_for_spawn
                    .maybe_update_name(&session_id, provider)
                    .await
                {
                    warn!("Failed to generate session description: {}", e);
                }
            });
        }

        // Count tool calls present before this reply — everything added during
        // the reply loop is part of the current turn and should not be summarized.
        let pre_turn_tool_count = conversation
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
            .count();

        let working_dir = session.working_dir.clone();
        let reply_stream_span = tracing::info_span!(target: "permagent::agents::agent", "reply_stream", session.id = %session_config.id);
        let inner = Box::pin(async_stream::try_stream! {
            // Working on entry; Drop records done/error on every exit path (#348).
            let mut state_guard = ReplyStateGuard::start();
            let mut turns_taken = 0u32;
            let max_turns = session_config.max_turns.unwrap_or_else(|| {
                Config::global()
                    .get_param::<u32>("GOOSE_MAX_TURNS")
                    .unwrap_or(DEFAULT_MAX_TURNS)
            });
            let mut compaction_attempts = 0;
            let mut last_assistant_text = String::new();
            // S5 monologue guard (runaway-loop safety): consecutive no-tool
            // turns that produced no NEW conclusion (same text as the prior
            // no-tool turn). Reset whenever a tool actually runs.
            let mut prev_monologue_text = String::new();
            let mut consecutive_monologue_turns = 0u32;
            let mut monologue_nudged = false;
            // Consecutive turns whose tool calls could not be parsed. A
            // malformed call used to END the session — the single most common
            // weak-model failure mode, terminating instead of being corrected.
            // It is now an observation the model can retry against, bounded so
            // a model that cannot recover still stops, and cleared by any turn
            // that parses (the same shape as `compaction_attempts` at the
            // provider round-trip and the S5 monologue counter below).
            let mut consecutive_parse_failure_turns = 0u32;

            loop {
                if is_token_cancelled(&cancel_token) {
                    break;
                }

                {
                    let guard = self.final_output_tool.lock().await;
                    if let Some(ref output) = guard.as_ref().and_then(|fot| fot.final_output.clone()) {
                        yield AgentEvent::Message(Message::assistant().with_text(output));
                        break;
                    }
                }

                turns_taken += 1;
                if turns_taken > max_turns {
                    yield AgentEvent::Message(
                        Message::assistant().with_text(
                            "I've reached the maximum number of actions I can do without user input. Would you like me to continue?"
                        )
                    );
                    break;
                }

                // TODO(S8): $-budget pre-turn stop. Once the cost ledger from
                // PR #714 lands (in CI, unmerged), add the pre-turn spend check
                // HERE beside S7: read the session's accumulated $-spend and, if
                // it exceeds the configured cap, escalate to the Decision Inbox
                // and pause — never a silent overspend. Deferred until the ledger
                // exists (the ProgressMonitor S1–S5 and the S7 turn cap already
                // bound a loop in the meantime).

                let conversation_with_moim = super::moim::inject_moim(
                    &session_config.id,
                    conversation.clone(),
                    &self.extension_manager,
                    &working_dir,
                ).await;

                // Prompt-cache observability. The prefix hash is emitted BEFORE
                // the request so a turn that errors out still leaves the hash in
                // the log; the provider-reported hit/miss follows on the first
                // usage frame below. A prefix hash that changes turn to turn
                // within one session IS the regression this phase exists to
                // prevent — without this line it is invisible and merely
                // expensive.
                let prefix_hash = system_prompt.prefix_hash();
                debug!(
                    target: "prompt_cache",
                    session_id = %session_config.id,
                    prefix.hash = %prefix_hash,
                    prefix.bytes = system_prompt.stable_prefix().len(),
                    volatile.bytes = system_prompt.volatile_suffix().len(),
                    "system prompt prefix"
                );
                let mut logged_cache_result = false;

                let mut stream = Self::stream_response_from_provider(
                    self.provider().await?,
                    &session_config.id,
                    &system_prompt,
                    conversation_with_moim.messages(),
                    &tools,
                    &toolshim_tools,
                ).await?;

                let current_turn_tool_count = conversation.messages().iter()
                    .flat_map(|m| m.content.iter())
                    .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
                    .count()
                    .saturating_sub(pre_turn_tool_count);

                let tool_pair_summarization_task = crate::context_mgmt::maybe_summarize_tool_pairs(
                    self.provider().await?,
                    session_config.id.clone(),
                    conversation.clone(),
                    tool_call_cut_off,
                    current_turn_tool_count,
                );

                let mut no_tools_called = true;
                // One strike and one recovery observation per turn: a single
                // truncated response can carry several malformed calls, and
                // three near-identical "could not be parsed" messages in one
                // turn is noise the model pays for on every turn after.
                let mut saw_parse_failure_this_turn = false;
                let mut messages_to_add = Conversation::default();
                let mut tools_updated = false;
                let mut did_recovery_compact_this_iteration = false;
                let mut exit_chat = false;

                // Track whether this provider turn has already emitted visible
                // thinking so a later tool-call chunk can suppress replayed
                // reasoning without hiding final-only non-streaming thoughts.
                let mut surfaced_thinking_in_turn = false;

                while let Some(next) = stream.next().await {
                    if is_token_cancelled(&cancel_token) || exit_chat {
                        break;
                    }

                    match next {
                        Ok((response, usage)) => {
                            compaction_attempts = 0;

                            if let Some(ref usage) = usage {
                                // Pair the prefix hash with what the provider
                                // actually did with it. Once per turn: usage
                                // frames arrive repeatedly on a stream and the
                                // cache verdict is decided on the first one.
                                //
                                // `None` for both counters means the provider
                                // reports no cache telemetry at all — logged as
                                // "unreported", never as a miss. Claiming a miss
                                // we did not observe would be the same defect in
                                // the other direction.
                                if !logged_cache_result {
                                    logged_cache_result = true;
                                    debug!(
                                        target: "prompt_cache",
                                        session_id = %session_config.id,
                                        prefix.hash = %prefix_hash,
                                        model = %usage.model,
                                        cache.read_tokens = ?usage.usage.cache_read_input_tokens,
                                        cache.write_tokens = ?usage.usage.cache_write_input_tokens,
                                        cache.result = match (
                                            usage.usage.cache_read_input_tokens,
                                            usage.usage.cache_write_input_tokens,
                                        ) {
                                            (None, None) => "unreported",
                                            (Some(r), _) if r > 0 => "hit",
                                            (_, Some(w)) if w > 0 => "write",
                                            _ => "miss",
                                        },
                                        "prompt cache result"
                                    );
                                }
                                self.update_session_metrics(&session_config.id, session_config.schedule_id.clone(), usage, false).await?;
                            }

                            if let Some(response) = response {
                                // Provider-side permission parks (claude-code /
                                // ACP subprocesses in approve/smart_approve):
                                // the provider has yielded an ActionRequired
                                // tool confirmation and is now parked on its own
                                // oneshot awaiting handle_permission_confirmation.
                                // Bridge the park into the Decision Inbox — or
                                // auto-deny it when this agent is headless —
                                // BEFORE the event reaches the client, mirroring
                                // the core-park filing order. Best-effort: never
                                // breaks the turn; the legacy action_required
                                // event below still flows either way.
                                self.bridge_provider_action_required(&response, &session_config.id)
                                    .await;

                                let ToolCategorizeResult {
                                    frontend_requests,
                                    remaining_requests,
                                    filtered_response,
                                } = self
                                    .categorize_tools(
                                        &response,
                                        &tools,
                                        surfaced_thinking_in_turn,
                                    )
                                    .await;

                                surfaced_thinking_in_turn |= filtered_response.content.iter().any(
                                    |content| {
                                        matches!(
                                            content,
                                            MessageContent::Thinking(_)
                                                | MessageContent::RedactedThinking(_)
                                        )
                                    },
                                );

                                yield AgentEvent::Message(filtered_response.clone());
                                tokio::task::yield_now().await;

                                let num_tool_requests = frontend_requests.len() + remaining_requests.len();
                                if num_tool_requests == 0 {
                                    let text = filtered_response.as_concat_text();
                                    if !text.is_empty() {
                                        last_assistant_text = text;
                                    }
                                    messages_to_add.push(response);
                                    continue;
                                }

                                let mut request_to_response_map = HashMap::new();
                                let mut request_metadata: HashMap<String, Option<ProviderMetadata>> = HashMap::new();
                                for request in frontend_requests.iter().chain(remaining_requests.iter()) {
                                    request_to_response_map.insert(request.id.clone(), Message::user().with_generated_id());
                                    request_metadata.insert(request.id.clone(), request.metadata.clone());
                                }

                                for request in frontend_requests.iter() {
                                    let response_msg = request_to_response_map.get_mut(&request.id)
                                        .ok_or_else(|| anyhow::anyhow!("missing response entry for request {}", request.id))?;
                                    let mut frontend_tool_stream = self.handle_frontend_tool_request(
                                        request,
                                        response_msg,
                                    );

                                    while let Some(msg) = frontend_tool_stream.try_next().await? {
                                        yield AgentEvent::Message(msg);
                                    }
                                }
                                if goose_mode == GooseMode::Chat {
                                    for request in remaining_requests.iter() {
                                        if let Some(response) = request_to_response_map.get_mut(&request.id) {
                                            response.add_tool_response_with_metadata(
                                                request.id.clone(),
                                                Ok(CallToolResult::success(vec![Content::text(CHAT_MODE_TOOL_SKIPPED_RESPONSE)])),
                                                request.metadata.as_ref(),
                                            );
                                        }
                                    }
                                } else {
                                    // Run all tool inspectors
                                    let inspection_results = self.tool_inspection_manager
                                        .inspect_tools(
                                            &session_config.id,
                                            &remaining_requests,
                                            conversation.messages(),
                                            goose_mode,
                                        )
                                        .await?;

                                    let permission_check_result = self.tool_inspection_manager
                                        .process_inspection_results_with_permission_inspector(
                                            &remaining_requests,
                                            &inspection_results,
                                        )
                                        .unwrap_or_else(|| {
                                            let mut result = PermissionCheckResult {
                                                approved: vec![],
                                                needs_approval: vec![],
                                                denied: vec![],
                                            };
                                            result.needs_approval.extend(remaining_requests.iter().cloned());
                                            result
                                        });

                                    // Track extension requests
                                    let mut enable_extension_request_ids = vec![];
                                    for request in &remaining_requests {
                                        if let Ok(tool_call) = &request.tool_call {
                                            if tool_call.name == MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE {
                                                enable_extension_request_ids.push(request.id.clone());
                                            }
                                        }
                                    }

                                    let mut tool_futures = self.handle_approved_and_denied_tools(
                                        &permission_check_result,
                                        &mut request_to_response_map,
                                        cancel_token.clone(),
                                        &session,
                                        &inspection_results,
                                    ).await?;

                                    {
                                        let mut tool_approval_stream = self.handle_approval_tool_requests(
                                            &permission_check_result.needs_approval,
                                            &mut tool_futures,
                                            &mut request_to_response_map,
                                            cancel_token.clone(),
                                            &session,
                                            &inspection_results,
                                        );

                                        while let Some(msg) = tool_approval_stream.try_next().await? {
                                            yield AgentEvent::Message(msg);
                                        }
                                    }

                                    let with_id = tool_futures
                                        .into_iter()
                                        .map(|(request_id, stream)| {
                                            stream.map(move |item| (request_id.clone(), item))
                                        })
                                        .collect::<Vec<_>>();

                                    let mut combined = stream::select_all(with_id);
                                    let mut all_install_successful = true;

                                    loop {
                                        if is_token_cancelled(&cancel_token) {
                                            break;
                                        }

                                        for msg in self.drain_elicitation_messages(&session_config.id).await {
                                            yield AgentEvent::Message(msg);
                                        }

                                        tokio::select! {
                                            biased;

                                            tool_item = combined.next() => {
                                                match tool_item {
                                                    Some((request_id, item)) => {
                                                        match item {
                                                            ToolStreamItem::Result(output) => {
                                                                if let Ok(ref call_result) = output {
                                                                    if let Some(ref meta) = call_result.meta {
                                                                        if let Some(notification_data) = meta.0.get("platform_notification") {
                                                                            if let Some(method) = notification_data.get("method").and_then(|v| v.as_str()) {
                                                                                let params = notification_data.get("params").cloned();
                                                                                let custom_notification = rmcp::model::CustomNotification::new(
                                                                                    method.to_string(),
                                                                                    params,
                                                                                );

                                                                                let server_notification = rmcp::model::ServerNotification::CustomNotification(custom_notification);
                                                                                yield AgentEvent::McpNotification((request_id.clone(), server_notification));
                                                                            }
                                                                        }
                                                                    }
                                                                }

                                                                if enable_extension_request_ids.contains(&request_id)
                                                                    && output.is_err()
                                                                {
                                                                    all_install_successful = false;
                                                                }
                                                                if let Some(response) = request_to_response_map.get_mut(&request_id) {
                                                                    let metadata = request_metadata.get(&request_id).and_then(|m| m.as_ref());
                                                                    response.add_tool_response_with_metadata(request_id, output, metadata);
                                                                }
                                                            }
                                                            ToolStreamItem::Message(msg) => {
                                                                yield AgentEvent::McpNotification((request_id, msg));
                                                            }
                                                        }
                                                    }
                                                    None => break,
                                                }
                                            }

                                            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                                                // Continue loop to drain elicitation messages
                                            }
                                        }
                                    }

                                    // check for remaining elicitation messages after all tools complete
                                    for msg in self.drain_elicitation_messages(&session_config.id).await {
                                        yield AgentEvent::Message(msg);
                                    }

                                    if all_install_successful && !enable_extension_request_ids.is_empty() {
                                        if let Err(e) = self.save_extension_state(&session_config).await {
                                            warn!("Failed to save extension state after runtime changes: {}", e);
                                        }
                                        tools_updated = true;
                                    }
                                }

                                // Preserve thinking/reasoning content from the original response
                                // Gemini (and other thinking models) require thinking to be echoed back
                                // Kimi/DeepSeek require reasoning_content on assistant tool call messages
                                let thinking_content: Vec<MessageContent> = response.content.iter()
                                    .filter(|c| matches!(c, MessageContent::Thinking(_)))
                                    .cloned()
                                    .collect();
                                if !thinking_content.is_empty() {
                                    let thinking_msg = Message::new(
                                        response.role.clone(),
                                        response.created,
                                        thinking_content,
                                    ).with_id(format!("msg_{}", Uuid::new_v4()));
                                    messages_to_add.push(thinking_msg);
                                }

                                // Collect reasoning content to attach to tool request messages
                                let reasoning_content: Vec<MessageContent> = response.content.iter()
                                    .filter(|c| matches!(c, MessageContent::Thinking(_)))
                                    .cloned()
                                    .collect();

                                for request in frontend_requests.iter().chain(remaining_requests.iter()) {
                                    if request.tool_call.is_ok() {
                                        let mut request_msg = Message::assistant()
                                            .with_id(format!("msg_{}", Uuid::new_v4()));

                                        // Providers like Kimi require reasoning_content on all assistant
                                        // messages with tool_calls when thinking mode is enabled.
                                        for rc in &reasoning_content {
                                            request_msg = request_msg.with_content(rc.clone());
                                        }

                                        request_msg = request_msg
                                            .with_tool_request_with_metadata(
                                                request.id.clone(),
                                                request.tool_call.clone(),
                                                request.metadata.as_ref(),
                                                request.tool_meta.clone(),
                                            );
                                        messages_to_add.push(request_msg);
                                        let final_response = request_to_response_map
                                            .remove(&request.id)
                                            .unwrap_or_else(|| Message::user().with_generated_id());
                                        yield AgentEvent::Message(final_response.clone());
                                        messages_to_add.push(final_response);
                                    } else {
                                        let parse_err = request
                                            .tool_call
                                            .as_ref()
                                            .err()
                                            .map(|e| e.to_string())
                                            .unwrap_or_default();
                                        // The log keeps the whole thing — it is
                                        // neither billed nor replayed.
                                        error!("Tool call could not be parsed: {}", parse_err);
                                        if saw_parse_failure_this_turn {
                                            // Already struck and already told. A
                                            // truncated response can carry several
                                            // malformed calls; they are one event.
                                            continue;
                                        }
                                        saw_parse_failure_this_turn = true;
                                        consecutive_parse_failure_turns += 1;
                                        if consecutive_parse_failure_turns >= MAX_CONSECUTIVE_PARSE_FAILURE_TURNS {
                                            yield AgentEvent::Message(
                                                Message::assistant().with_text(
                                                    "A tool call could not be parsed — the response may have been truncated. Try breaking the task into smaller steps or resending your message."
                                                )
                                            );
                                            exit_chat = true;
                                            break;
                                        }
                                        // Hand the failure back as an observation: what
                                        // broke, so the model can correct the ONE
                                        // malformed argument rather than losing the
                                        // session. Echoing the received call is what
                                        // makes a retry land — bounded, because that
                                        // echo contains the model's own raw arguments.
                                        let recovery =
                                            Message::user().with_text(parse_recovery_text(&parse_err));
                                        yield AgentEvent::Message(recovery.clone());
                                        messages_to_add.push(recovery);
                                        continue;
                                    }
                                }

                                no_tools_called = false;
                            }
                        }
                        #[allow(unused_variables)]
                        Err(ref provider_err @ ProviderError::ContextLengthExceeded(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            compaction_attempts += 1;

                            if compaction_attempts >= 2 {
                                error!("Context limit exceeded after compaction - prompt too large");
                                yield AgentEvent::Message(
                                    Message::assistant().with_system_notification(
                                        SystemNotificationType::InlineMessage,
                                        "Unable to continue: Context limit still exceeded after compaction. Try using a shorter message, a model with a larger context window, or start a new session."
                                    )
                                );
                                state_guard.mark_error();
                                break;
                            }

                            yield AgentEvent::Message(
                                Message::assistant().with_system_notification(
                                    SystemNotificationType::InlineMessage,
                                    "Context limit reached. Compacting to continue conversation...",
                                )
                            );
                            yield AgentEvent::Message(
                                Message::assistant().with_system_notification(
                                    SystemNotificationType::ThinkingMessage,
                                    COMPACTION_THINKING_TEXT,
                                )
                            );

                            match compact_messages(
                                self.provider().await?.as_ref(),
                                &session_config.id,
                                &conversation,
                                false,
                            )
                            .await
                            {
                                Ok((compacted_conversation, usage)) => {
                                    session_manager.replace_conversation(&session_config.id, &compacted_conversation).await?;
                                    self.update_session_metrics(&session_config.id, session_config.schedule_id.clone(), &usage, true).await?;
                                    conversation = compacted_conversation;
                                    did_recovery_compact_this_iteration = true;
                                    yield AgentEvent::HistoryReplaced(conversation.clone());
                                    break;
                                }
                                Err(e) => {
                                    #[cfg(feature = "telemetry")]
                                    crate::posthog::emit_error("compaction_failed", &e.to_string());
                                    error!("Compaction failed: {}", e);
                                    yield AgentEvent::Message(
                                        Message::assistant().with_text(
                                            format!("Ran into this error trying to compact: {e}.\n\nPlease try again or create a new session")
                                        )
                                    );
                                    state_guard.mark_error();
                                    break;
                                }
                            }
                        }
                        Err(ref provider_err @ ProviderError::CreditsExhausted { details: _, ref top_up_url }) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);

                            let user_msg = if top_up_url.is_some() {
                                "Please add credits to your account, then resend your message to continue.".to_string()
                            } else {
                                "Please check your account with your provider to add more credits, then resend your message to continue.".to_string()
                            };

                            let notification_data = serde_json::json!({
                                "top_up_url": top_up_url,
                            });

                            yield AgentEvent::Message(
                                Message::assistant().with_system_notification_with_data(
                                    SystemNotificationType::CreditsExhausted,
                                    user_msg,
                                    notification_data,
                                )
                            );
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err @ ProviderError::NetworkError(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            yield AgentEvent::Message(
                                Message::assistant().with_text(
                                    format!("{provider_err}\n\nPlease resend your message to try again.")
                                )
                            );
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err @ ProviderError::Authentication(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            // Attribute a 401/auth failure to its ACTUAL source — the LLM
                            // provider's own credentials — so it is never misread (by the
                            // user or by a downstream model) as an auth problem with a
                            // website, document, or tool the agent was using.
                            let provider_label = match self.provider().await {
                                Ok(provider) => format!("LLM provider ({})", provider.get_name()),
                                Err(_) => "LLM provider".to_string(),
                            };
                            let message = Message::assistant().with_text(
                                format!(
                                    "Authentication failed for your {provider_label}: the API key was rejected (HTTP 401). \
                                     This is a credential problem with the model provider configured in Permagent — \
                                     not with any website, page, or service the agent was reading. \
                                     Check that the provider's API key is set, valid, and has the required permissions, then resend your message.\n\nDetails: {provider_err}"
                                )
                            );
                            persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err @ ProviderError::RequestFailed(_)) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            // A 4xx means the provider rejected this exact request as
                            // invalid — resending the identical payload fails the same
                            // way, so don't invite a retry. Point at the levers that
                            // actually change the request.
                            let message = Message::assistant().with_text(
                                format!("Ran into this error: {provider_err}.\n\nThe provider rejected this request as invalid, so sending it again unchanged will fail the same way. Switch the model (Settings → Models), or start a new session, then resend your message.")
                            );
                            persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                        Err(ref provider_err) => {
                            #[cfg(feature = "telemetry")]
                            crate::posthog::emit_error(provider_err.telemetry_type(), &provider_err.to_string());
                            error!("Error: {}", provider_err);
                            let message = Message::assistant().with_text(
                                format!("Ran into this error: {provider_err}.\n\nPlease retry if you think this is a transient or recoverable error.")
                            );
                            persist_turn_ending_message(&session_manager, &session_config.id, &message).await;
                            yield AgentEvent::Message(message);
                            state_guard.mark_error();
                            break;
                        }
                    }
                }
                if tools_updated {
                    (tools, toolshim_tools, system_prompt) =
                        self.prepare_tools_and_prompt(&session_config.id, &session.working_dir).await?;
                }

                {
                    let has_new_hints = self
                        .prompt_manager
                        .lock()
                        .await
                        .load_subdirectory_hints(&working_dir);
                    if has_new_hints && !tools_updated {
                        (tools, toolshim_tools, system_prompt) =
                            self.prepare_tools_and_prompt(&session_config.id, &session.working_dir).await?;
                    }
                }

                // S5 monologue guard: a tool ran this turn → reset the counter.
                if !no_tools_called {
                    consecutive_monologue_turns = 0;
                    monologue_nudged = false;
                    prev_monologue_text.clear();
                }

                // The model produced a turn with nothing unparseable in it, so
                // it recovered — the strike count starts again. This has to be
                // end-of-TURN: resetting per parsed request would let a model
                // that emits [valid, malformed] every turn (the ordinary
                // truncation shape) run forever without ever reaching the
                // bound.
                if !saw_parse_failure_this_turn {
                    consecutive_parse_failure_turns = 0;
                }

                if no_tools_called {
                    // S5: count consecutive no-tool turns that reached no NEW
                    // conclusion (identical assistant text). A turn that says
                    // something new resets the run to 1.
                    if !last_assistant_text.is_empty() && last_assistant_text == prev_monologue_text {
                        consecutive_monologue_turns += 1;
                    } else {
                        consecutive_monologue_turns = 1;
                    }
                    prev_monologue_text = last_assistant_text.clone();

                    // Lock, extract state, drop guard before branching — handle_retry_logic
                    // also locks final_output_tool and tokio::sync::Mutex is not reentrant.
                    let final_output = {
                        let guard = self.final_output_tool.lock().await;
                        guard.as_ref().map(|fot| fot.final_output.clone())
                    };

                    match final_output {
                        Some(None) => {
                            warn!("Final output tool has not been called yet. Continuing agent loop.");
                            // S5 nudge (L1, 0 human cost): the loop is about to
                            // continue but the model keeps monologuing without
                            // acting. Add a one-time hint alongside the
                            // continuation so it self-corrects.
                            if !monologue_nudged
                                && matches!(
                                    assess_monologue(consecutive_monologue_turns),
                                    LoopAction::Nudge(_)
                                )
                            {
                                monologue_nudged = true;
                                let nudge = Message::user().with_text(
                                    "You have replied several times without calling a tool or \
                                     reaching a new conclusion. Either call the final-output tool \
                                     with your answer now, take a concrete action, or state \
                                     plainly what is blocking you.",
                                );
                                messages_to_add.push(nudge.clone());
                                yield AgentEvent::Message(nudge);
                            }
                            let message = Message::user().with_text(FINAL_OUTPUT_CONTINUATION_MESSAGE);
                            messages_to_add.push(message.clone());
                            yield AgentEvent::Message(message);
                        }
                        Some(Some(output)) => {
                            let message = Message::assistant().with_text(output);
                            messages_to_add.push(message.clone());
                            yield AgentEvent::Message(message);
                            exit_chat = true;
                        }
                        None if did_recovery_compact_this_iteration => {
                            // continue from last user message after recovery compact
                        }
                        None => {
                            match self.handle_retry_logic(&mut conversation, &session_config, &initial_messages).await {
                                Ok(should_retry) => {
                                    if should_retry {
                                        info!("Retry logic triggered, restarting agent loop");
                                        messages_to_add = Conversation::default();
                                        session_manager.replace_conversation(&session_config.id, &conversation).await?;
                                        yield AgentEvent::HistoryReplaced(conversation.clone());
                                    } else {
                                        exit_chat = true;
                                    }
                                }
                                Err(e) => {
                                    error!("Retry logic failed: {}", e);
                                    yield AgentEvent::Message(
                                        Message::assistant().with_text(
                                            format!("Retry logic encountered an error: {}", e)
                                        )
                                    );
                                    exit_chat = true;
                                }
                            }
                        }
                    }
                }

                if is_token_cancelled(&cancel_token) {
                    tool_pair_summarization_task.abort();
                }

                if let Ok(summaries) = tool_pair_summarization_task.await {
                    let mut updated_messages = conversation.messages().clone();

                    for (summary_msg, tool_id) in summaries {
                        let matching: Vec<&mut Message> = updated_messages
                            .iter_mut()
                            .filter(|msg| {
                                msg.id.is_some() && msg.content.iter().any(|c| match c {
                                    MessageContent::ToolRequest(req) => req.id == tool_id,
                                    MessageContent::ToolResponse(resp) => resp.id == tool_id,
                                    _ => false,
                                })
                            })
                            .collect();

                        if matching.len() == 2 {
                            for msg in matching {
                                let id = msg.id.as_ref().unwrap();
                                msg.metadata = msg.metadata.with_agent_invisible();
                                SessionManager::update_message_metadata(&session_config.id, id, |metadata| {
                                    metadata.with_agent_invisible()
                                }).await?;
                            }
                            messages_to_add.push(summary_msg);
                        } else {
                            warn!("Expected a tool request/reply pair, but found {} matching messages",
                                matching.len());
                        }
                    }
                    conversation = Conversation::new_unvalidated(updated_messages);
                }

                for msg in &messages_to_add {
                    session_manager.add_message(&session_config.id, msg).await?;
                }
                conversation.extend(messages_to_add);
                if exit_chat {
                    break;
                }

                tokio::task::yield_now().await;
            }

            if !last_assistant_text.is_empty() {
                tracing::info!(target: "permagent::agents::agent", trace_output = last_assistant_text.as_str());
            }
        }.instrument(reply_stream_span));
        Ok(inner)
    }

    pub async fn extend_system_prompt(&self, key: String, instruction: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.add_system_prompt_extra(key, instruction);
    }

    pub async fn set_persona(&self, persona: crate::config::agent_identity::SharedPersona) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_persona(persona);
    }

    pub async fn set_persona_block_override(&self, block: String, display_name: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_persona_block_override(block, display_name);
    }

    pub async fn update_provider(
        &self,
        provider: Arc<dyn Provider>,
        session_id: &str,
    ) -> Result<()> {
        let provider_name = provider.get_name().to_string();
        let model_config = provider.get_model_config();

        let mut current_provider = self.provider.lock().await;
        *current_provider = Some(provider);

        self.config
            .session_manager
            .clone()
            .update(session_id)
            .provider_name(&provider_name)
            .model_config(model_config)
            .apply()
            .await
            .context("Failed to persist provider config to session")
    }

    pub async fn update_goose_mode(&self, mode: GooseMode, session_id: &str) -> Result<()> {
        if let Some(provider) = self.provider.lock().await.as_ref() {
            provider
                .update_mode(session_id, mode)
                .await
                .map_err(|e| anyhow::anyhow!("Provider rejected mode update: {e}"))?;
        }
        *self.current_goose_mode.lock().await = mode;
        self.config
            .session_manager
            .clone()
            .update(session_id)
            .goose_mode(mode)
            .apply()
            .await
            .context("Failed to persist goose_mode to session")
    }

    pub async fn goose_mode(&self) -> GooseMode {
        *self.current_goose_mode.lock().await
    }

    /// Restore the provider from session data or fall back to global config
    /// This is used when resuming a session to restore the provider state
    /// Returns true if the session's provider was replaced with a fallback.
    pub async fn restore_provider_from_session(&self, session: &Session) -> Result<bool> {
        let config = Config::global();

        let provider_name = session
            .provider_name
            .clone()
            .or_else(|| config.get_goose_provider().ok())
            .ok_or_else(|| anyhow!("Could not configure agent: missing provider"))?;

        let model_config = match session.model_config.clone() {
            Some(saved_config) => saved_config,
            None => {
                let model_name = config
                    .get_goose_model()
                    .ok()
                    .ok_or_else(|| anyhow!("Could not configure agent: missing model"))?;
                crate::model::ModelConfig::new(&model_name)
                    .map_err(|e| anyhow!("Could not configure agent: invalid model {}", e))?
                    .with_canonical_limits(&provider_name)
            }
        };

        let extensions =
            EnabledExtensionsState::extensions_or_default(Some(&session.extension_data), config);

        let (provider, provider_changed) = if crate::providers::get_from_registry(&provider_name)
            .await
            .is_ok()
        {
            let p = crate::providers::create(&provider_name, model_config, extensions)
                .await
                .map_err(|e| anyhow!("Could not create provider: {}", e))?;
            (p, false)
        } else {
            let fallback_provider_name = config
                .get_goose_provider()
                .ok()
                .filter(|name| name != &provider_name)
                .ok_or_else(|| {
                    anyhow!(
                        "Could not create provider: provider '{}' not found",
                        provider_name
                    )
                })?;

            tracing::warn!(
                "Session provider '{}' unavailable, falling back to '{}'",
                provider_name,
                fallback_provider_name
            );

            let fallback_model_name = config
                .get_goose_model()
                .ok()
                .ok_or_else(|| anyhow!("Could not configure fallback provider: missing model"))?;
            let fallback_model_config = crate::model::ModelConfig::new(&fallback_model_name)
                .map_err(|e| anyhow!("Could not configure fallback provider: invalid model {}", e))?
                .with_canonical_limits(&fallback_provider_name);

            let fallback_provider = crate::providers::create(
                &fallback_provider_name,
                fallback_model_config.clone(),
                extensions,
            )
            .await
            .map_err(|e| {
                anyhow!(
                    "Could not create provider '{}' or fallback '{}': {}",
                    provider_name,
                    fallback_provider_name,
                    e
                )
            })?;

            if let Err(e) = self
                .config
                .session_manager
                .update(&session.id)
                .provider_name(&fallback_provider_name)
                .model_config(fallback_model_config)
                .apply()
                .await
            {
                tracing::warn!("Failed to update session provider: {}", e);
            }

            (fallback_provider, true)
        };

        self.update_provider(provider, &session.id).await?;
        // Propagate session mode to the new provider
        if let Some(provider) = self.provider.lock().await.as_ref() {
            provider
                .update_mode(&session.id, session.goose_mode)
                .await
                .map_err(|e| anyhow!("Failed to propagate mode to provider: {}", e))?;
        }
        *self.current_goose_mode.lock().await = session.goose_mode;
        Ok(provider_changed)
    }

    /// Override the system prompt with a custom template
    pub async fn override_system_prompt(&self, template: String) {
        let mut prompt_manager = self.prompt_manager.lock().await;
        prompt_manager.set_system_prompt_override(template);
    }

    pub async fn list_extension_prompts(&self, session_id: &str) -> HashMap<String, Vec<Prompt>> {
        self.extension_manager
            .list_prompts(session_id, CancellationToken::default())
            .await
            .expect("Failed to list prompts")
    }

    pub async fn get_prompt(
        &self,
        session_id: &str,
        name: &str,
        arguments: Value,
    ) -> Result<GetPromptResult> {
        // First find which extension has this prompt
        let prompts = self
            .extension_manager
            .list_prompts(session_id, CancellationToken::default())
            .await
            .map_err(|e| anyhow!("Failed to list prompts: {}", e))?;

        if let Some(extension) = prompts
            .iter()
            .find(|(_, prompt_list)| prompt_list.iter().any(|p| p.name == name))
            .map(|(extension, _)| extension)
        {
            return self
                .extension_manager
                .get_prompt(
                    session_id,
                    extension,
                    name,
                    arguments,
                    CancellationToken::default(),
                )
                .await
                .map_err(|e| anyhow!("Failed to get prompt: {}", e));
        }

        Err(anyhow!("Prompt '{}' not found", name))
    }

    pub async fn get_plan_prompt(&self, session_id: &str) -> Result<String> {
        let tools = self
            .extension_manager
            .get_prefixed_tools(session_id, None)
            .await?;
        let tools_info = tools
            .into_iter()
            .map(|tool| {
                ToolInfo::new(
                    &tool.name,
                    tool.description
                        .as_ref()
                        .map(|d| d.as_ref())
                        .unwrap_or_default(),
                    get_parameter_names(&tool),
                    None,
                )
            })
            .collect();

        let plan_prompt = self.extension_manager.get_planning_prompt(tools_info).await;

        Ok(plan_prompt)
    }

    pub async fn handle_tool_result(&self, id: String, result: ToolResult<CallToolResult>) {
        if let Err(e) = self.tool_result_tx.send((id, result)).await {
            error!("Failed to send tool result: {}", e);
        }
    }

    pub async fn create_recipe(
        &self,
        session_id: &str,
        mut messages: Conversation,
    ) -> Result<Recipe> {
        tracing::info!("Starting recipe creation with {} messages", messages.len());

        let session = self
            .config
            .session_manager
            .get_session(session_id, false)
            .await?;
        let extensions_info = self
            .extension_manager
            .get_extensions_info(&session.working_dir)
            .await;
        tracing::debug!("Retrieved {} extensions info", extensions_info.len());
        let (extension_count, tool_count) = self
            .extension_manager
            .get_extension_and_tool_counts(session_id)
            .await;

        // Get model name from provider
        let provider = self.provider().await.map_err(|e| {
            tracing::error!("Failed to get provider for recipe creation: {}", e);
            e
        })?;
        let model_config = provider.get_model_config();
        let model_name = &model_config.model_name;
        tracing::debug!("Using model: {}", model_name);

        let goose_mode = *self.current_goose_mode.lock().await;
        let prompt_manager = self.prompt_manager.lock().await;
        let system_prompt = prompt_manager
            .builder()
            .with_extensions(extensions_info.into_iter())
            .with_frontend_instructions(self.frontend_instructions.lock().await.clone())
            .with_extension_and_tool_counts(extension_count, tool_count)
            .with_goose_mode(goose_mode)
            .build();

        let recipe_prompt = prompt_manager.get_recipe_prompt().await;
        let tools: Vec<_> = self
            .extension_manager
            .get_prefixed_tools(session_id, None)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get tools for recipe creation: {}", e);
                e
            })?
            .into_iter()
            .filter(super::reply_parts::is_tool_visible_to_model)
            .collect();

        messages.push(Message::user().with_text(recipe_prompt));

        let (messages, issues) = fix_conversation(messages);
        if !issues.is_empty() {
            issues
                .iter()
                .for_each(|issue| tracing::warn!(recipe.conversation.issue = issue));
        }

        tracing::debug!(
            "Added recipe prompt to messages, total messages: {}",
            messages.len()
        );

        tracing::info!("Calling provider to generate recipe content");
        let model_config = {
            let provider_guard = self.provider.lock().await;
            let provider = provider_guard.as_ref().ok_or_else(|| {
                let error = anyhow!("Provider not available during recipe creation");
                tracing::error!("{}", error);
                error
            })?;
            provider.get_model_config()
        };
        let (result, _usage) = self
            .provider
            .lock()
            .await
            .as_ref()
            .ok_or_else(|| {
                let error = anyhow!("Provider not available during recipe creation");
                tracing::error!("{}", error);
                error
            })?
            .complete(
                &model_config,
                session_id,
                &system_prompt,
                messages.messages(),
                &tools,
            )
            .await
            .map_err(|e| {
                tracing::error!("Provider completion failed during recipe creation: {}", e);
                e
            })?;

        let content = result.as_concat_text();
        tracing::debug!(
            "Provider returned content with {} characters",
            content.len()
        );

        // the response may be contained in ```json ```, strip that before parsing json
        let re = Regex::new(r"(?s)```[^\n]*\n(.*?)\n```").unwrap();
        let clean_content = re
            .captures(&content)
            .and_then(|caps| caps.get(1).map(|m| m.as_str()))
            .unwrap_or(&content)
            .trim()
            .to_string();

        let (instructions, activities) =
            if let Ok(json_content) = serde_json::from_str::<Value>(&clean_content) {
                let instructions = json_content
                    .get("instructions")
                    .ok_or_else(|| anyhow!("Missing 'instructions' in json response"))?
                    .as_str()
                    .ok_or_else(|| anyhow!("instructions' is not a string"))?
                    .to_string();

                let activities = json_content
                    .get("activities")
                    .ok_or_else(|| anyhow!("Missing 'activities' in json response"))?
                    .as_array()
                    .ok_or_else(|| anyhow!("'activities' is not an array'"))?
                    .iter()
                    .map(|act| {
                        act.as_str()
                            .map(|s| s.to_string())
                            .ok_or(anyhow!("'activities' array element is not a string"))
                    })
                    .collect::<Result<_, _>>()?;

                (instructions, activities)
            } else {
                tracing::warn!("Failed to parse JSON, falling back to string parsing");
                // If we can't get valid JSON, try string parsing
                // Use split_once to get the content after "Instructions:".
                let after_instructions = content
                    .split_once("instructions:")
                    .map(|(_, rest)| rest)
                    .unwrap_or(&content);

                // Split once more to separate instructions from activities.
                let (instructions_part, activities_text) = after_instructions
                    .split_once("activities:")
                    .unwrap_or((after_instructions, ""));

                let instructions = instructions_part
                    .trim_end_matches(|c: char| c.is_whitespace() || c == '#')
                    .trim()
                    .to_string();
                let activities_text = activities_text.trim();

                // Regex to remove bullet markers or numbers with an optional dot.
                let bullet_re = Regex::new(r"^[•\-*\d]+\.?\s*").expect("Invalid regex");

                // Process each line in the activities section.
                let activities: Vec<String> = activities_text
                    .lines()
                    .map(|line| bullet_re.replace(line, "").to_string())
                    .map(|s| s.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect();

                (instructions, activities)
            };

        let extension_configs = get_enabled_extensions();

        let author = Author {
            contact: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            metadata: None,
        };

        // Ideally we'd get the name of the provider we are using from the provider itself,
        // but it doesn't know and the plumbing looks complicated.
        let config = Config::global();
        let provider_name: String = config
            .get_goose_provider()
            .expect("No provider configured. Run 'goose configure' first");

        let settings = Settings {
            goose_provider: Some(provider_name.clone()),
            goose_model: Some(model_name.clone()),
            temperature: Some(model_config.temperature.unwrap_or(0.0)),
            max_turns: None,
        };

        tracing::debug!(
            "Building recipe with {} activities and {} extensions",
            activities.len(),
            extension_configs.len()
        );

        let (title, description) =
            if let Ok(json_content) = serde_json::from_str::<Value>(&clean_content) {
                let title = json_content
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Custom recipe from chat")
                    .to_string();

                let description = json_content
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("a custom recipe instance from this chat session")
                    .to_string();

                (title, description)
            } else {
                (
                    "Custom recipe from chat".to_string(),
                    "a custom recipe instance from this chat session".to_string(),
                )
            };

        let recipe = Recipe::builder()
            .title(title)
            .description(description)
            .instructions(instructions)
            .activities(activities)
            .extensions(extension_configs)
            .settings(settings)
            .author(author)
            .build()
            .map_err(|e| {
                tracing::error!("Failed to build recipe: {}", e);
                anyhow!("Recipe build failed: {}", e)
            })?;

        tracing::info!("Recipe creation completed successfully");
        Ok(recipe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::permission_confirmation::PrincipalType;
    use crate::providers::base::PermissionRouting;
    use crate::recipe::Response;

    struct ActionRequiredProvider {
        handled: tokio::sync::Mutex<Vec<(String, PermissionConfirmation)>>,
    }

    impl ActionRequiredProvider {
        fn new() -> Self {
            Self {
                handled: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl std::fmt::Debug for ActionRequiredProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ActionRequiredProvider").finish()
        }
    }

    #[async_trait::async_trait]
    impl crate::providers::base::Provider for ActionRequiredProvider {
        fn get_name(&self) -> &str {
            "test-action-required"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new("test").unwrap()
        }
        async fn stream(
            &self,
            _: &crate::model::ModelConfig,
            _: &str,
            _: &str,
            _: &[crate::conversation::message::Message],
            _: &[rmcp::model::Tool],
        ) -> Result<crate::providers::base::MessageStream, crate::providers::errors::ProviderError>
        {
            unimplemented!()
        }
        fn permission_routing(&self) -> PermissionRouting {
            PermissionRouting::ActionRequired
        }
        async fn handle_permission_confirmation(
            &self,
            request_id: &str,
            confirmation: &PermissionConfirmation,
        ) -> bool {
            self.handled
                .lock()
                .await
                .push((request_id.to_string(), confirmation.clone()));
            request_id == "known"
        }
    }

    #[tokio::test]
    async fn test_handle_confirmation_routes_to_provider() {
        let agent = Agent::new();
        let provider = Arc::new(ActionRequiredProvider::new());
        *agent.provider.lock().await =
            Some(provider.clone() as Arc<dyn crate::providers::base::Provider>);

        // Known request_id → provider handles it, confirmation_router NOT called
        let delivered = agent
            .handle_confirmation(
                "known".to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: crate::permission::Permission::AllowOnce,
                },
            )
            .await;
        assert!(delivered, "provider-consumed confirmation must report true");
        assert_eq!(provider.handled.lock().await.len(), 1);

        // Unknown request_id → provider returns false, falls through to confirmation_router
        // Register first so deliver() has somewhere to send
        let rx = agent
            .tool_confirmation_router
            .register("unknown".to_string())
            .await
            .unwrap();
        let delivered = agent
            .handle_confirmation(
                "unknown".to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: crate::permission::Permission::DenyOnce,
                },
            )
            .await;
        assert!(delivered, "router-delivered confirmation must report true");
        assert_eq!(provider.handled.lock().await.len(), 2);
        // Verify the fallthrough went to confirmation_router
        let conf = rx.await.unwrap();
        assert_eq!(conf.permission, crate::permission::Permission::DenyOnce);
    }

    #[tokio::test]
    async fn test_handle_confirmation_noop_provider() {
        let agent = Agent::new();
        // No provider set → Noop routing, goes straight to confirmation_router
        // Register first so deliver() has somewhere to send
        let rx = agent
            .tool_confirmation_router
            .register("any".to_string())
            .await
            .unwrap();
        let delivered = agent
            .handle_confirmation(
                "any".to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: crate::permission::Permission::AllowOnce,
                },
            )
            .await;
        assert!(delivered);

        let conf = rx.await.unwrap();
        assert_eq!(conf.permission, crate::permission::Permission::AllowOnce);
    }

    /// A confirmation with no live waiter (turn cancelled, daemon restarted, or
    /// already answered elsewhere) must report `false` — the honesty signal the
    /// Decision Inbox effect message depends on.
    #[tokio::test]
    async fn test_handle_confirmation_reports_no_waiter() {
        let agent = Agent::new();

        // Nothing registered at all.
        let delivered = agent
            .handle_confirmation(
                "never-registered".to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: crate::permission::Permission::AllowOnce,
                },
            )
            .await;
        assert!(!delivered, "no registered waiter must report false");

        // Registered but the awaiting task is gone (receiver dropped).
        let rx = agent
            .tool_confirmation_router
            .register("cancelled-turn".to_string())
            .await
            .unwrap();
        drop(rx);
        let delivered = agent
            .handle_confirmation(
                "cancelled-turn".to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: crate::permission::Permission::AllowOnce,
                },
            )
            .await;
        assert!(!delivered, "dropped receiver must report false");
    }

    #[tokio::test]
    async fn test_add_final_output_tool() -> Result<()> {
        let agent = Agent::new();

        let response = Response {
            json_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "result": {"type": "string"}
                }
            })),
        };

        agent.add_final_output_tool(response).await?;

        let tools = agent.list_tools("test-session-id", None).await;
        let final_output_tool = tools
            .iter()
            .find(|tool| tool.name == FINAL_OUTPUT_TOOL_NAME);

        assert!(
            final_output_tool.is_some(),
            "Final output tool should be present after adding"
        );

        let prompt_manager = agent.prompt_manager.lock().await;
        let system_prompt = prompt_manager
            .builder()
            .with_goose_mode(GooseMode::default())
            .build();

        let final_output_tool_ref = agent.final_output_tool.lock().await;
        let final_output_tool_system_prompt =
            final_output_tool_ref.as_ref().unwrap().system_prompt();
        assert!(system_prompt.contains(&final_output_tool_system_prompt));
        Ok(())
    }

    /// A recipe with a bad `response.json_schema` must surface as an error,
    /// never a panic, and must leave no half-installed final-output tool
    /// behind (bug-sweep wave 1: the old panic here crash-cycled the daemon).
    #[tokio::test]
    async fn test_add_final_output_tool_rejects_bad_schema() -> Result<()> {
        let agent = Agent::new();

        for bad in [
            None,
            Some(serde_json::json!(true)),
            Some(serde_json::json!({})),
            Some(serde_json::json!({"type": 42})),
        ] {
            let result = agent
                .apply_recipe_components(Some(Response { json_schema: bad }), true)
                .await;
            assert!(result.is_err(), "bad schema must be rejected");
        }

        assert!(
            agent.final_output_tool.lock().await.is_none(),
            "no final-output tool may be installed after a rejected schema"
        );
        let tools = agent.list_tools("test-session-id", None).await;
        assert!(
            !tools.iter().any(|t| t.name == FINAL_OUTPUT_TOOL_NAME),
            "tool listing must not contain the final-output tool"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_tool_inspection_manager_has_all_inspectors() -> Result<()> {
        let agent = Agent::new();

        // Verify that the tool inspection manager has all expected inspectors
        let inspector_names = agent.tool_inspection_manager.inspector_names();

        assert!(
            inspector_names.contains(&crate::tool_monitor::PROGRESS_MONITOR_NAME),
            "Tool inspection manager should contain the progress monitor (runaway-loop guard)"
        );
        assert!(
            inspector_names.contains(&"permission"),
            "Tool inspection manager should contain permission inspector"
        );
        assert!(
            inspector_names.contains(&"security"),
            "Tool inspection manager should contain security inspector"
        );
        assert!(
            inspector_names.contains(&"adversary"),
            "Tool inspection manager should contain adversary inspector"
        );
        assert!(
            inspector_names.contains(&crate::security::write_jail::WRITE_JAIL_INSPECTOR_NAME),
            "Tool inspection manager should contain the write jail (C3)"
        );

        Ok(())
    }

    #[test]
    fn bearer_token_is_redacted_from_tracing_and_task_description() {
        let secret = "trace-and-task-secret";
        let arguments = serde_json::json!({
            "command": format!("curl -H 'Authorization: Bearer {secret}' https://example.com")
        })
        .as_object()
        .unwrap()
        .clone();

        let tracing_input = redacted_tool_input_summary("shell", Some(&arguments));
        let task_description = tool_task_description("shell", Some(&arguments));

        for logged in [&tracing_input, &task_description] {
            assert!(logged.contains("[REDACTED]"));
            assert!(!logged.contains(secret), "Bearer token must not be logged");
        }
    }

    /// The recovery observation carries the model's own raw arguments back
    /// into the conversation, where it is persisted and re-sent every turn.
    mod parse_recovery {
        use super::*;

        fn huge_parse_error() -> String {
            // The real shape: serde's complaint, then the whole blob.
            format!(
                "Could not interpret tool use parameters for id call_1: EOF while parsing a \
                 string. Raw arguments: '{}MALFORMED_TAIL'",
                "x".repeat(400_000)
            )
        }

        #[test]
        fn bounds_a_huge_raw_argument_echo() {
            let msg = parse_recovery_text(&huge_parse_error());
            assert!(
                msg.chars().count() < MAX_PARSE_ERROR_ECHO_CHARS + 500,
                "echo not bounded: {} chars",
                msg.chars().count()
            );
            assert!(
                msg.contains("truncated"),
                "the cut must be stated, not silent"
            );
            assert!(
                msg.contains("Re-issue exactly one tool call"),
                "the instruction must survive truncation"
            );
        }

        #[test]
        fn keeps_the_end_of_the_blob_where_the_break_is() {
            // A truncated response is malformed at its TAIL. Head-only
            // elision would keep the harmless opening and discard the one
            // region the model needs in order to correct itself.
            let msg = parse_recovery_text(&huge_parse_error());
            assert!(msg.contains("MALFORMED_TAIL"), "tail elided away");
            assert!(
                msg.contains("Could not interpret tool use parameters"),
                "serde's own complaint elided away"
            );
        }

        #[test]
        fn echoes_a_short_error_verbatim_with_no_marker() {
            let err = "Could not interpret tool use parameters for id call_1: \
                       expected value at line 1 column 1";
            let msg = parse_recovery_text(err);
            assert!(msg.contains(err));
            assert!(!msg.contains("truncated"));
        }

        #[test]
        fn multibyte_text_never_splits_a_character() {
            // Transcripts and tool arguments carry arbitrary UTF-8; a byte
            // slice here would panic on the boundary.
            let err = "\u{3042}".repeat(200_000);
            let msg = parse_recovery_text(&err);
            assert!(msg.chars().count() < MAX_PARSE_ERROR_ECHO_CHARS + 500);
        }
    }
}
